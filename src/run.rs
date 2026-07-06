//! Run metadata: what changed this run, and the content hashes needed to
//! detect no-op runs.

use crate::domain::Artifact;
use crate::graph::Graph;
use crate::manifest::PageManifest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Persisted artifact content-hash snapshot, used to detect changed
/// artifacts across runs without re-diffing full file content.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RepositorySnapshot {
    /// Artifact path to content hash.
    pub artifact_hashes: BTreeMap<String, String>,
}

impl RepositorySnapshot {
    /// Builds a snapshot from the current artifact set.
    pub fn from_artifacts(artifacts: &[Artifact]) -> Self {
        Self {
            artifact_hashes: artifacts
                .iter()
                .map(|artifact| {
                    (
                        artifact.path.as_str().to_owned(),
                        artifact.content_hash.as_str().to_owned(),
                    )
                })
                .collect(),
        }
    }

    /// Returns artifact paths that are new, removed, or changed relative to
    /// `previous` (the prior run's snapshot). Every artifact is "changed"
    /// when there is no previous snapshot (first run).
    pub fn changed_since(&self, previous: Option<&RepositorySnapshot>) -> Vec<String> {
        let Some(previous) = previous else {
            return self.artifact_hashes.keys().cloned().collect();
        };

        let mut changed: BTreeSet<String> = BTreeSet::new();
        for (path, hash) in &self.artifact_hashes {
            if previous.artifact_hashes.get(path) != Some(hash) {
                changed.insert(path.clone());
            }
        }
        for path in previous.artifact_hashes.keys() {
            if !self.artifact_hashes.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        changed.into_iter().collect()
    }

    /// Deterministic hash over the whole snapshot.
    pub fn hash(&self) -> String {
        let mut pairs: Vec<String> = self
            .artifact_hashes
            .iter()
            .map(|(path, hash)| format!("{path}:{hash}"))
            .collect();
        pairs.sort_unstable();
        blake3::hash(pairs.join("\n").as_bytes())
            .to_hex()
            .to_string()
    }
}

/// Metadata recorded for one `init`/`update` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunMetadata {
    /// Unique identifier for this run.
    pub run_id: String,
    /// CLI command that produced this run, e.g. `init`.
    pub command: String,
    /// Repository git HEAD commit, when the repository is a git checkout.
    pub git_head: Option<String>,
    /// Hash over the current artifact snapshot.
    pub snapshot_hash: String,
    /// Hash over the exported graph.
    pub graph_hash: String,
    /// Hash over written page content, excluding run-specific metadata.
    pub output_hash: String,
    /// Artifact paths changed since the previous run.
    pub changed_artifacts: Vec<String>,
    /// Page IDs actually rewritten this run.
    pub changed_pages: Vec<String>,
}

impl RunMetadata {
    /// Computes run metadata for one completed run.
    pub fn compute(
        command: &str,
        repo_root: &Path,
        artifacts: &[Artifact],
        graph: &Graph,
        manifest: &PageManifest,
        written_pages: &[String],
        previous_snapshot: Option<&RepositorySnapshot>,
    ) -> Result<(Self, RepositorySnapshot), serde_json::Error> {
        let snapshot = RepositorySnapshot::from_artifacts(artifacts);
        let changed_artifacts = snapshot.changed_since(previous_snapshot);
        let graph_hash = blake3::hash(graph.to_json()?.as_bytes())
            .to_hex()
            .to_string();

        let metadata = Self {
            run_id: run_id(),
            command: command.to_owned(),
            git_head: git_head(repo_root),
            snapshot_hash: snapshot.hash(),
            graph_hash,
            output_hash: output_hash(manifest),
            changed_artifacts,
            changed_pages: written_pages.to_vec(),
        };
        Ok((metadata, snapshot))
    }
}

// ponytail: output_hash covers only page id + rendered output hash, never
// run_id/timestamps/git_head, so a re-run with identical page content
// produces the same output_hash even though run_id always differs.
fn output_hash(manifest: &PageManifest) -> String {
    let mut pairs: Vec<String> = manifest
        .pages
        .iter()
        .map(|page| format!("{}:{}", page.id, page.output_hash.as_deref().unwrap_or("")))
        .collect();
    pairs.sort_unstable();
    blake3::hash(pairs.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

fn run_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("run-{millis}")
}

fn git_head(repo_root: &Path) -> Option<String> {
    if !repo_root.join(".git").exists() {
        return None;
    }
    let output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|head| head.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{RepositorySnapshot, RunMetadata};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, RepoPath, SupportTier, TextStatus,
    };
    use crate::graph::Graph;
    use crate::manifest::PageManifest;

    fn artifact(path: &str, hash: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::GenericText,
            ContentHash::new(hash)?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn first_run_reports_every_artifact_as_changed() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = RepositorySnapshot::from_artifacts(&[
            artifact("a.rs", "aaaa")?,
            artifact("b.rs", "bbbb")?,
        ]);

        let mut changed = snapshot.changed_since(None);
        changed.sort();

        assert_eq!(changed, vec!["a.rs".to_owned(), "b.rs".to_owned()]);

        Ok(())
    }

    #[test]
    fn subsequent_run_reports_only_added_removed_and_modified_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let previous = RepositorySnapshot::from_artifacts(&[
            artifact("a.rs", "aaaa")?,
            artifact("b.rs", "bbbb")?,
        ]);
        let current = RepositorySnapshot::from_artifacts(&[
            artifact("a.rs", "aaaa")?,
            artifact("b.rs", "beef")?,
            artifact("c.rs", "cccc")?,
        ]);

        let mut changed = current.changed_since(Some(&previous));
        changed.sort();

        assert_eq!(changed, vec!["b.rs".to_owned(), "c.rs".to_owned()]);

        Ok(())
    }

    #[test]
    fn no_op_run_has_stable_hashes_and_no_changed_pages() -> Result<(), Box<dyn std::error::Error>>
    {
        let artifacts = vec![artifact("a.rs", "aaaa")?];
        let graph = Graph::default();
        let manifest = PageManifest::default();
        let snapshot = RepositorySnapshot::from_artifacts(&artifacts);

        let (first, _) = RunMetadata::compute(
            "init",
            std::path::Path::new("."),
            &artifacts,
            &graph,
            &manifest,
            &[],
            None,
        )?;
        let (second, _) = RunMetadata::compute(
            "init",
            std::path::Path::new("."),
            &artifacts,
            &graph,
            &manifest,
            &[],
            Some(&snapshot),
        )?;

        assert_eq!(first.snapshot_hash, second.snapshot_hash);
        assert_eq!(first.graph_hash, second.graph_hash);
        assert_eq!(first.output_hash, second.output_hash);
        assert!(second.changed_artifacts.is_empty());
        assert!(second.changed_pages.is_empty());
        assert_ne!(first.run_id, "");

        Ok(())
    }
}
