//! JSON file storage: the one place Lithograph reads/writes its own
//! `.lithograph/*.json` state, so no other module reaches for ad hoc
//! `std::fs` calls to persist metadata.

use serde::Serialize;
use serde::de::DeserializeOwned;
use std::io;
use std::path::Path;

/// Reads and writes deterministic pretty-printed JSON files, creating
/// parent directories as needed and treating a missing file as `None`
/// rather than an error.
#[derive(Debug, Clone, Copy, Default)]
pub struct JsonStore;

impl JsonStore {
    /// Serializes `value` as pretty JSON and writes it to `path`, creating
    /// parent directories as needed.
    pub fn write<T: Serialize>(&self, path: &Path, value: &T) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string_pretty(value).map_err(to_io_error)?;
        json.push('\n');
        std::fs::write(path, json)
    }

    /// Reads and parses `path`, returning `Ok(None)` when it does not exist.
    pub fn read<T: DeserializeOwned>(&self, path: &Path) -> io::Result<Option<T>> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map(Some).map_err(to_io_error),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Writes `value` only when it differs from what is already at `path`,
    /// so an unchanged run leaves the file's mtime untouched. Returns
    /// whether a write happened.
    pub fn write_if_changed<T: Serialize + DeserializeOwned + PartialEq>(
        &self,
        path: &Path,
        value: &T,
    ) -> io::Result<bool> {
        let existing: Option<T> = self.read(path)?;
        if existing.as_ref() == Some(value) {
            return Ok(false);
        }
        self.write(path, value)?;
        Ok(true)
    }
}

fn to_io_error(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::JsonStore;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Sample {
        name: String,
        count: u32,
    }

    #[test]
    fn round_trips_through_a_created_directory() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("nested/dir/sample.json");
        let value = Sample {
            name: "lithograph".to_owned(),
            count: 3,
        };

        JsonStore.write(&path, &value)?;
        let read_back: Option<Sample> = JsonStore.read(&path)?;

        assert_eq!(read_back, Some(value));

        Ok(())
    }

    #[test]
    fn missing_file_reads_as_none() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        let read_back: Option<Sample> = JsonStore.read(&temp.path().join("absent.json"))?;

        assert_eq!(read_back, None);

        Ok(())
    }

    #[test]
    fn write_if_changed_skips_identical_content_and_leaves_mtime_alone()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("sample.json");
        let value = Sample {
            name: "lithograph".to_owned(),
            count: 3,
        };

        assert!(JsonStore.write_if_changed(&path, &value)?);
        let written_at = std::fs::metadata(&path)?.modified()?;
        std::thread::sleep(std::time::Duration::from_millis(10));

        assert!(!JsonStore.write_if_changed(&path, &value)?);
        assert_eq!(std::fs::metadata(&path)?.modified()?, written_at);

        let changed = Sample { count: 4, ..value };
        assert!(JsonStore.write_if_changed(&path, &changed)?);

        Ok(())
    }
}
