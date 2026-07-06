//! Golden snapshot checking for generated Lithograph output.

use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

/// One snapshot action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenEntry {
    /// Repository-relative source path.
    pub source: String,
    /// Snapshot-relative path.
    pub snapshot: String,
    /// True when the snapshot was written during an update run.
    pub updated: bool,
}

/// Golden check/update report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoldenReport {
    /// Entries that matched or were updated.
    pub entries: Vec<GoldenEntry>,
    /// Mismatched or missing snapshots in check mode.
    pub failures: Vec<String>,
}

impl GoldenReport {
    /// True when every checked snapshot matched.
    pub fn is_clean(&self) -> bool {
        self.failures.is_empty()
    }
}

/// Snapshot check/update failure.
#[derive(Debug)]
pub enum GoldenError {
    /// Filesystem failure.
    Io(std::io::Error),
    /// Check mode found differences.
    Mismatch(GoldenReport),
}

impl Display for GoldenError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "golden snapshot I/O failed: {error}"),
            Self::Mismatch(report) => {
                writeln!(
                    formatter,
                    "golden snapshot check failed with {} mismatch(es):",
                    report.failures.len()
                )?;
                for failure in &report.failures {
                    writeln!(formatter, "  - {failure}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for GoldenError {}

impl From<std::io::Error> for GoldenError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Checks or updates snapshots for docs, manifest, and research artifacts.
pub fn check_or_update(
    repo_root: &Path,
    golden_dir: &Path,
    update: bool,
) -> Result<GoldenReport, GoldenError> {
    let sources = snapshot_sources(repo_root)?;
    let mut report = GoldenReport {
        entries: Vec::new(),
        failures: Vec::new(),
    };

    for source in sources {
        let relative = source
            .strip_prefix(repo_root)
            .map_or_else(|_| source.clone(), Path::to_path_buf);
        let relative_string = normalize_path(&relative);
        let snapshot_path = golden_dir.join(&relative);
        if update {
            if let Some(parent) = snapshot_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&source, &snapshot_path)?;
            report.entries.push(GoldenEntry {
                source: relative_string.clone(),
                snapshot: relative_string,
                updated: true,
            });
            continue;
        }

        let source_bytes = std::fs::read(&source)?;
        match std::fs::read(&snapshot_path) {
            Ok(snapshot_bytes) if snapshot_bytes == source_bytes => {
                report.entries.push(GoldenEntry {
                    source: relative_string.clone(),
                    snapshot: relative_string,
                    updated: false,
                });
            }
            Ok(_) => report
                .failures
                .push(format!("{relative_string} differs from snapshot")),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => report
                .failures
                .push(format!("{relative_string} has no snapshot")),
            Err(error) => return Err(GoldenError::Io(error)),
        }
    }

    if update || report.is_clean() {
        Ok(report)
    } else {
        Err(GoldenError::Mismatch(report))
    }
}

fn snapshot_sources(repo_root: &Path) -> Result<Vec<PathBuf>, GoldenError> {
    let mut files = Vec::new();
    collect_files(&repo_root.join("docs/lithograph"), &mut files)?;
    for relative in [
        ".lithograph/manifest.json",
        ".lithograph/research/brief.json",
        ".lithograph/research/system-context.json",
        ".lithograph/research/workflows.json",
        ".lithograph/research/boundaries.json",
        ".lithograph/research/configuration.json",
        ".lithograph/research/key-modules.json",
    ] {
        let path = repo_root.join(relative);
        if path.exists() {
            files.push(path);
        }
    }
    files.sort();
    Ok(files)
}

fn collect_files(root: &Path, files: &mut Vec<PathBuf>) -> Result<(), GoldenError> {
    if !root.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(root)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_files(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

fn normalize_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Renders a compact golden report.
pub fn render_report(report: &GoldenReport) -> String {
    let action = if report.entries.iter().any(|entry| entry.updated) {
        "updated"
    } else {
        "checked"
    };
    format!(
        "{action} {} golden snapshot file(s)\n",
        report.entries.len()
    )
}

#[cfg(test)]
mod tests {
    use super::{GoldenError, check_or_update};
    use crate::generation::MockModel;
    use crate::orchestrate::run_init;
    use std::path::Path;

    #[test]
    fn updates_then_checks_generated_snapshots() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let golden_dir = temp.path().join("golden");

        let updated = check_or_update(temp.path(), &golden_dir, true)?;
        let checked = check_or_update(temp.path(), &golden_dir, false)?;

        assert!(!updated.entries.is_empty());
        assert_eq!(updated.entries.len(), checked.entries.len());

        Ok(())
    }

    #[test]
    fn check_reports_mismatch() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let golden_dir = temp.path().join("golden");
        check_or_update(temp.path(), &golden_dir, true)?;
        std::fs::write(temp.path().join("docs/lithograph/overview.md"), "changed\n")?;

        match check_or_update(temp.path(), &golden_dir, false) {
            Err(GoldenError::Mismatch(_)) => {}
            Err(error) => return Err(Box::new(error)),
            Ok(_) => return Err("golden check unexpectedly passed".into()),
        }

        Ok(())
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for entry in walk_files(from)? {
            let relative = entry.strip_prefix(from)?;
            let destination = to.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry, &destination)?;
        }
        Ok(())
    }

    fn walk_files(root: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        Ok(files)
    }
}
