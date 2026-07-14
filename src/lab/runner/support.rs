//! Shared, mode-agnostic JSON I/O, content-addressing, schema migration, and
//! process telemetry primitives used by every lab mode submodule.

use super::LabError;
use crate::lab::model::LAB_SCHEMA_VERSION;
use crate::storage::JsonStore;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;

/// Reads and deserializes a file that must exist.
pub(super) fn read_required<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LabError> {
    JsonStore
        .read(path)?
        .ok_or_else(|| LabError::Invalid(format!("required file is missing: {}", path.display())))
}

/// Reads and deserializes a file that must exist, applying mechanical schema
/// migration before deserialization.
pub(super) fn read_compatible<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LabError> {
    read_optional_compatible(path)?
        .ok_or_else(|| LabError::Invalid(format!("required file is missing: {}", path.display())))
}

/// Reads and deserializes an optional file, applying mechanical schema
/// migration before deserialization.
pub(super) fn read_optional_compatible<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, LabError> {
    let Some(mut value): Option<Value> = JsonStore.read(path)? else {
        return Ok(None);
    };
    migrate_value(&mut value)?;
    Ok(Some(serde_json::from_value(value)?))
}

/// Returns the `schema_version` field of a lab JSON value.
pub(super) fn schema_version(value: &Value) -> Result<u32, LabError> {
    value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| LabError::Invalid("lab JSON has no valid schema_version".to_owned()))
}

/// Applies purely mechanical lab JSON schema migrations in place, returning a
/// human-readable description of every change made.
pub(super) fn migrate_value(value: &mut Value) -> Result<Vec<String>, LabError> {
    let version = schema_version(value)?;
    if version > LAB_SCHEMA_VERSION {
        return Err(LabError::Invalid(format!(
            "lab schema {version} is newer than supported schema {LAB_SCHEMA_VERSION}"
        )));
    }
    let mut changes = Vec::new();
    if version < 2 {
        value["schema_version"] = Value::from(2);
        changes.push("schema_version: 1 -> 2".to_owned());
        if value.get("run_id").is_some() && value.get("graph_pipeline_version").is_none() {
            value["graph_pipeline_version"] =
                Value::from(crate::graph::GRAPH_BUILD_PIPELINE_VERSION);
            changes.push(format!(
                "graph_pipeline_version: absent -> {}",
                crate::graph::GRAPH_BUILD_PIPELINE_VERSION
            ));
        }
        if let Some(stage_hashes) = value.get_mut("stage_hashes").and_then(Value::as_object_mut) {
            for (old, new) in [
                ("Structure", "structure"),
                ("DefinitionsAndImports", "definitions_and_imports"),
                ("Enrichment", "enrichment"),
                ("Resolution", "resolution"),
                ("Analytics", "analytics"),
                ("Persistence", "persistence"),
                ("Finalize", "finalize"),
            ] {
                if let Some(hash) = stage_hashes.remove(old) {
                    stage_hashes.insert(new.to_owned(), hash);
                    changes.push(format!("stage_hashes.{old} -> stage_hashes.{new}"));
                }
            }
        }
    }
    Ok(changes)
}

/// Writes a value as pretty JSON via a temporary file and atomic rename.
pub(super) fn atomic_json_write<T: Serialize>(path: &Path, value: &T) -> Result<(), LabError> {
    let parent = path.parent().ok_or_else(|| {
        LabError::Invalid(format!("baseline path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.accepting");
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

/// Content-addresses any serializable value with BLAKE3.
pub(super) fn hash_json<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    Ok(blake3::hash(serde_json::to_vec(value)?.as_slice())
        .to_hex()
        .to_string())
}

/// Reads the process's peak resident set size in KiB, best-effort.
pub(super) fn process_rss_kib() -> u64 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(value) = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    {
        return value;
    }
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p"])
        .arg(std::process::id().to_string())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}
