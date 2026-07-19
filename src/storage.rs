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
pub(crate) struct JsonStore;

impl JsonStore {
    /// Serializes `value` as pretty JSON and writes it to `path`, creating
    /// parent directories as needed.
    pub(crate) fn write<T: Serialize>(&self, path: &Path, value: &T) -> io::Result<()> {
        self.write_rendered(path, &render(value, Layout::Pretty)?)
    }

    /// Reads and parses `path`, returning `Ok(None)` when it does not exist.
    pub(crate) fn read<T: DeserializeOwned>(&self, path: &Path) -> io::Result<Option<T>> {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map(Some).map_err(to_io_error),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Writes `value` only when its rendered JSON differs from what is already
    /// at `path`, so an unchanged run leaves the file's mtime untouched.
    /// Returns whether a write happened.
    ///
    /// Serialization is deterministic, so comparing the freshly rendered bytes
    /// against the file on disk is equivalent to comparing values -- and it
    /// avoids deserializing the existing file back into `T` (a full parse of a
    /// tens-of-megabytes graph snapshot) just to decide whether to write. It
    /// also renders once instead of the previous read-parse-then-serialize
    /// path, and drops the `DeserializeOwned + PartialEq` bound so borrowed,
    /// non-owning views (e.g. a graph snapshot referencing its graph) can be
    /// persisted without cloning.
    pub(crate) fn write_if_changed<T: Serialize>(&self, path: &Path, value: &T) -> io::Result<bool> {
        self.write_if_changed_layout(path, value, Layout::Pretty)
    }

    /// Like [`write_if_changed`] but emits compact (unindented) JSON. Reserved
    /// for large, purely machine-read artifacts -- e.g. the graph snapshot,
    /// tens of megabytes where indentation is pure size and parse overhead --
    /// while small human-inspectable state files stay pretty.
    pub(crate) fn write_if_changed_compact<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
    ) -> io::Result<bool> {
        self.write_if_changed_layout(path, value, Layout::Compact)
    }

    fn write_if_changed_layout<T: Serialize>(
        &self,
        path: &Path,
        value: &T,
        layout: Layout,
    ) -> io::Result<bool> {
        let rendered = render(value, layout)?;
        if let Ok(existing) = std::fs::read_to_string(path)
            && existing == rendered
        {
            return Ok(false);
        }
        self.write_rendered(path, &rendered)?;
        Ok(true)
    }

    /// Writes already-rendered JSON bytes to `path`, creating parent
    /// directories as needed.
    fn write_rendered(&self, path: &Path, rendered: &str) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, rendered)
    }
}

/// JSON indentation choice for on-disk artifacts.
#[derive(Debug, Clone, Copy)]
enum Layout {
    /// Human-inspectable pretty JSON. The default for small state files.
    Pretty,
    /// Compact single-line JSON for large machine-read artifacts.
    Compact,
}

/// Renders `value` to the exact bytes written on disk: JSON in the requested
/// layout with a trailing newline. The single source of truth for both `write`
/// and the change check in `write_if_changed*`.
fn render<T: Serialize>(value: &T, layout: Layout) -> io::Result<String> {
    let mut json = match layout {
        Layout::Pretty => serde_json::to_string_pretty(value),
        Layout::Compact => serde_json::to_string(value),
    }
    .map_err(to_io_error)?;
    json.push('\n');
    Ok(json)
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
