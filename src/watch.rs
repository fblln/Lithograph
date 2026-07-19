//! Opt-in auto-index and watch mode (LIT-22.8.2): reports repository
//! staleness against the last recorded `.lithograph/snapshot.json`,
//! respecting the same ignore rules as `init`/`update`, and never writes
//! anything to the repository itself -- callers decide whether staleness
//! should trigger a real `update` run (AC1: no ambient auto-indexing).

use crate::inventory::{RepositoryWalker, WalkError, WalkOptions};
use crate::orchestrate::scan_exclude_globs;
use crate::run::RepositorySnapshot;
use crate::storage::JsonStore;
use std::fmt::{Display, Formatter};
use std::path::Path;
use std::time::Duration;

/// Opt-in watch/poll configuration. Constructing this and calling
/// [`poll_once`] is the only way staleness gets checked -- nothing in
/// `init`/`update` reaches this module (AC1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WatchConfig {
    /// Maximum artifacts one poll may scan (AC2's "safe project limits").
    /// Exceeding this returns [`WatchError::ProjectTooLarge`] rather than
    /// silently scanning a truncated subset of the repository.
    pub max_artifacts: usize,
    /// Delay between polls for a long-running watch loop. Unused by
    /// [`poll_once`] itself, which always scans exactly once.
    pub poll_interval: Duration,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            max_artifacts: 20_000,
            poll_interval: Duration::from_secs(5),
        }
    }
}

/// Why one poll could not complete.
#[derive(Debug)]
pub(crate) enum WatchError {
    /// The repository has more artifacts than `max_artifacts` allows.
    ProjectTooLarge {
        /// Artifacts discovered before the walk was abandoned.
        artifact_count: usize,
        /// Configured limit that was exceeded.
        max_artifacts: usize,
    },
    /// Repository walking failed.
    Walk(WalkError),
    /// Reading the previous snapshot failed.
    Io(std::io::Error),
}

impl Display for WatchError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ProjectTooLarge {
                artifact_count,
                max_artifacts,
            } => write!(
                formatter,
                "repository has {artifact_count} artifact(s), exceeding the safe watch limit of {max_artifacts}; increase max_artifacts or narrow the watched path"
            ),
            Self::Walk(source) => write!(formatter, "watch scan failed: {source}"),
            Self::Io(source) => write!(formatter, "failed to read previous snapshot: {source}"),
        }
    }
}

impl std::error::Error for WatchError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ProjectTooLarge { .. } => None,
            Self::Walk(source) => Some(source),
            Self::Io(source) => Some(source),
        }
    }
}

impl From<WalkError> for WatchError {
    fn from(source: WalkError) -> Self {
        Self::Walk(source)
    }
}

impl From<std::io::Error> for WatchError {
    fn from(source: std::io::Error) -> Self {
        Self::Io(source)
    }
}

/// One poll's staleness result (AC3: visible to both the CLI and MCP
/// callers -- the CLI's `watch` command renders this directly, and
/// `WikiMcpServer::detect_changes` reuses the same underlying comparison).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StaleReport {
    /// `true` when any artifact was added, removed, or content-changed
    /// since the last recorded snapshot (or no snapshot exists yet).
    pub stale: bool,
    /// Changed artifact paths, sorted.
    pub changed_artifacts: Vec<String>,
    /// Total artifacts scanned this poll, after ignore-rule filtering.
    pub artifact_count: usize,
}

/// Scans `repo_root` with the same ignore rules `init`/`update` use (AC2)
/// and compares against `.lithograph/snapshot.json`, without writing
/// anything. Returns [`WatchError::ProjectTooLarge`] instead of scanning a
/// truncated subset when the repository exceeds `config.max_artifacts`.
pub(crate) fn poll_once(repo_root: &Path, config: &WatchConfig) -> Result<StaleReport, WatchError> {
    let walk_options = WalkOptions {
        exclude_globs: scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(repo_root)?;
    if artifacts.len() > config.max_artifacts {
        return Err(WatchError::ProjectTooLarge {
            artifact_count: artifacts.len(),
            max_artifacts: config.max_artifacts,
        });
    }

    let snapshot_path = repo_root.join(".lithograph/snapshot.json");
    let previous: Option<RepositorySnapshot> = JsonStore.read(&snapshot_path)?;
    let pipeline = previous
        .as_ref()
        .map(|snapshot| snapshot.pipeline.clone())
        .unwrap_or_default();
    let current = RepositorySnapshot::from_artifacts(&artifacts, pipeline);
    let changed_artifacts = current.changed_since(previous.as_ref());

    Ok(StaleReport {
        stale: !changed_artifacts.is_empty(),
        artifact_count: artifacts.len(),
        changed_artifacts,
    })
}

/// Renders a [`StaleReport`] for CLI output (AC3).
pub(crate) fn render_report(report: &StaleReport) -> String {
    if !report.stale {
        return format!(
            "up to date: {} artifact(s) scanned, no changes since the last recorded snapshot\n",
            report.artifact_count
        );
    }
    let mut rendered = format!(
        "stale: {} of {} artifact(s) changed since the last recorded snapshot\n",
        report.changed_artifacts.len(),
        report.artifact_count
    );
    for path in &report.changed_artifacts {
        rendered.push_str(&format!("  {path}\n"));
    }
    rendered
}

#[cfg(test)]
mod tests {
    use super::{WatchConfig, WatchError, poll_once, render_report};
    use crate::generation::MockModel;
    use crate::orchestrate::run_init;
    use std::path::Path;

    fn write(path: &Path, relative: &str, content: &str) -> Result<(), std::io::Error> {
        let target = path.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, content)
    }

    #[test]
    fn first_poll_with_no_snapshot_reports_every_artifact_stale()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        write(temp.path(), "src/lib.rs", "fn main() {}\n")?;

        let report = poll_once(temp.path(), &WatchConfig::default())?;

        assert!(report.stale);
        assert_eq!(report.artifact_count, 1);
        assert_eq!(report.changed_artifacts, vec!["src/lib.rs".to_owned()]);
        assert!(render_report(&report).contains("stale"));

        Ok(())
    }

    #[test]
    fn poll_after_init_with_no_edits_reports_not_stale() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        write(temp.path(), "src/lib.rs", "fn main() {}\n")?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        let report = poll_once(temp.path(), &WatchConfig::default())?;

        assert!(!report.stale);
        assert!(report.changed_artifacts.is_empty());
        assert!(render_report(&report).contains("up to date"));

        Ok(())
    }

    #[test]
    fn poll_after_editing_one_file_reports_only_that_file() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        write(temp.path(), "src/lib.rs", "fn main() {}\n")?;
        write(temp.path(), "src/other.rs", "fn other() {}\n")?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        write(temp.path(), "src/lib.rs", "fn main() { edited(); }\n")?;

        let report = poll_once(temp.path(), &WatchConfig::default())?;

        assert!(report.stale);
        assert_eq!(report.changed_artifacts, vec!["src/lib.rs".to_owned()]);

        Ok(())
    }

    #[test]
    fn poll_never_reports_gitignored_files() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        write(temp.path(), "src/lib.rs", "fn main() {}\n")?;
        write(temp.path(), ".gitignore", "ignored/\n")?;
        write(temp.path(), "ignored/secret.rs", "const X: u8 = 1;\n")?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        write(temp.path(), "ignored/secret.rs", "const X: u8 = 2;\n")?;

        let report = poll_once(temp.path(), &WatchConfig::default())?;

        assert!(!report.stale);
        assert!(
            !report
                .changed_artifacts
                .iter()
                .any(|path| path.contains("ignored"))
        );

        Ok(())
    }

    #[test]
    fn poll_rejects_repositories_over_the_safe_artifact_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        write(temp.path(), "a.rs", "fn a() {}\n")?;
        write(temp.path(), "b.rs", "fn b() {}\n")?;
        let config = WatchConfig {
            max_artifacts: 1,
            ..WatchConfig::default()
        };

        match poll_once(temp.path(), &config) {
            Ok(_) => return Err("expected a project-too-large error".into()),
            Err(WatchError::ProjectTooLarge {
                artifact_count,
                max_artifacts,
            }) => {
                assert_eq!(artifact_count, 2);
                assert_eq!(max_artifacts, 1);
            }
            Err(other) => return Err(format!("expected ProjectTooLarge, got {other:?}").into()),
        }

        Ok(())
    }
}
