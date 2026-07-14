//! Purely mechanical lab JSON schema migration. Never accepts a semantic
//! baseline change: it only rewrites schema shape (field renames, added
//! defaulted fields) and always goes through [`Lab::migrate`], never the
//! reviewed acceptance path.

use super::{Lab, LabError, atomic_json_write, migrate_value, schema_version};
use crate::lab::model::{LAB_SCHEMA_VERSION, MigrationReport};
use serde_json::Value;
use std::path::Path;

impl Lab {
    /// Previews or applies a purely mechanical lab JSON schema migration.
    /// It never updates semantic baselines through the acceptance path.
    pub fn migrate(&self, path: &Path, apply: bool) -> Result<MigrationReport, LabError> {
        let bytes = std::fs::read(path)?;
        let mut value: Value = serde_json::from_slice(&bytes)?;
        let from_version = schema_version(&value)?;
        let changes = migrate_value(&mut value)?;
        if apply && !changes.is_empty() {
            atomic_json_write(path, &value)?;
        }
        Ok(MigrationReport {
            path: path.display().to_string(),
            from_version,
            to_version: LAB_SCHEMA_VERSION,
            applied: apply && !changes.is_empty(),
            changes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::corpus::Corpus;

    #[test]
    fn migration_previews_v1_without_semantic_acceptance() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        let lab_dir = temp.path().join("lab");
        std::fs::create_dir_all(&lab_dir)?;
        std::fs::write(
            lab_dir.join("corpus.toml"),
            "schema_version = 2\ncases = []\n",
        )?;
        let artifact = temp.path().join("baseline.json");
        std::fs::write(
            &artifact,
            r#"{"schema_version":1,"stage_hashes":{"Structure":"abc"},"reason":"reviewed"}"#,
        )?;
        let corpus = Corpus::load(&lab_dir.join("corpus.toml"), &temp.path().join("cache"))?;
        let lab = Lab::new(corpus, temp.path().join("out"));
        let preview = lab.migrate(&artifact, false)?;
        assert!(!preview.applied);
        assert!(
            preview
                .changes
                .iter()
                .any(|change| change.contains("stage_hashes"))
        );
        assert!(std::fs::read_to_string(&artifact)?.contains("\"schema_version\":1"));
        let applied = lab.migrate(&artifact, true)?;
        assert!(applied.applied);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(artifact)?)?;
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["reason"], "reviewed");
        Ok(())
    }
}
