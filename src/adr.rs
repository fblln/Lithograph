//! Architecture Decision Record (ADR) store (LIT-22.5.4): persistent,
//! validated create/get/update/delete/list operations over
//! `.lithograph/adrs/*.json`. One JSON file per record is the source of
//! truth (simpler to validate and update programmatically than round-
//! tripping hand-authored Markdown); CLI/MCP callers render it however
//! they need.

use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

/// ADR lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AdrStatus {
    /// Drafted, not yet decided.
    Proposed,
    /// Actively in effect.
    Accepted,
    /// No longer recommended, but not replaced by a specific other ADR.
    Deprecated,
    /// Replaced by a later decision.
    Superseded,
}

/// The only section keys [`AdrStore::update_section`] accepts (AC2).
pub const ADR_SECTION_KEYS: &[&str] = &["context", "decision", "consequences"];

/// One persisted architecture decision record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdrRecord {
    /// Stable id, e.g. `"ADR-0001"`.
    pub id: String,
    /// Short decision title.
    pub title: String,
    /// Lifecycle status.
    pub status: AdrStatus,
    /// Section name to content, keys restricted to [`ADR_SECTION_KEYS`].
    pub sections: BTreeMap<String, String>,
}

/// Lightweight listing entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdrSummary {
    /// Stable id.
    pub id: String,
    /// Short decision title.
    pub title: String,
    /// Lifecycle status.
    pub status: AdrStatus,
}

/// A validation or lookup failure, always with an actionable message (AC2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdrError {
    /// `title` was empty or whitespace-only.
    EmptyTitle,
    /// A required section's content was empty or whitespace-only.
    EmptySection(String),
    /// `section` is not one of [`ADR_SECTION_KEYS`].
    UnknownSection(String),
    /// No ADR exists with this id.
    NotFound(String),
    /// The on-disk record failed to read or parse.
    Io(String),
}

impl Display for AdrError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyTitle => write!(formatter, "ADR title must not be empty"),
            Self::EmptySection(section) => {
                write!(formatter, "ADR section `{section}` must not be empty")
            }
            Self::UnknownSection(section) => write!(
                formatter,
                "unknown ADR section `{section}`: valid sections are {}",
                ADR_SECTION_KEYS.join(", ")
            ),
            Self::NotFound(id) => write!(formatter, "no ADR found with id `{id}`"),
            Self::Io(message) => write!(formatter, "ADR store error: {message}"),
        }
    }
}

impl std::error::Error for AdrError {}

/// File-backed ADR store rooted at one repository's `.lithograph/adrs/`.
#[derive(Debug, Clone)]
pub struct AdrStore {
    dir: PathBuf,
}

impl AdrStore {
    /// Opens the store for a repository. The directory is created lazily
    /// on first write.
    pub fn new(repo_root: &Path) -> Self {
        Self {
            dir: repo_root.join(".lithograph/adrs"),
        }
    }

    /// Validates and creates a new ADR with the next sequential id,
    /// `status: Proposed`, and `context`/`decision` sections (AC1/AC2).
    /// `consequences` is optional -- many real decisions don't have known
    /// consequences yet at proposal time -- and can be added later via
    /// [`Self::update_section`].
    pub fn create(
        &self,
        title: &str,
        context: &str,
        decision: &str,
        consequences: Option<&str>,
    ) -> Result<AdrRecord, AdrError> {
        let title = non_empty("title", title).map_err(|_| AdrError::EmptyTitle)?;
        let context = non_empty("context", context)
            .map_err(|_| AdrError::EmptySection("context".to_owned()))?;
        let decision = non_empty("decision", decision)
            .map_err(|_| AdrError::EmptySection("decision".to_owned()))?;

        let mut sections = BTreeMap::new();
        sections.insert("context".to_owned(), context.to_owned());
        sections.insert("decision".to_owned(), decision.to_owned());
        if let Some(consequences) = consequences {
            let consequences = non_empty("consequences", consequences)
                .map_err(|_| AdrError::EmptySection("consequences".to_owned()))?;
            sections.insert("consequences".to_owned(), consequences.to_owned());
        }

        let record = AdrRecord {
            id: self.next_id(),
            title: title.to_owned(),
            status: AdrStatus::Proposed,
            sections,
        };
        self.write(&record)?;
        Ok(record)
    }

    /// Reads one ADR by id.
    pub fn get(&self, id: &str) -> Result<AdrRecord, AdrError> {
        JsonStore
            .read(&self.path_for(id))
            .map_err(|error| AdrError::Io(error.to_string()))?
            .ok_or_else(|| AdrError::NotFound(id.to_owned()))
    }

    /// Sets one section's content, validating the section key and content
    /// (AC1/AC2).
    pub fn update_section(
        &self,
        id: &str,
        section: &str,
        value: &str,
    ) -> Result<AdrRecord, AdrError> {
        if !ADR_SECTION_KEYS.contains(&section) {
            return Err(AdrError::UnknownSection(section.to_owned()));
        }
        let value =
            non_empty(section, value).map_err(|_| AdrError::EmptySection(section.to_owned()))?;
        let mut record = self.get(id)?;
        record.sections.insert(section.to_owned(), value.to_owned());
        self.write(&record)?;
        Ok(record)
    }

    /// Sets the ADR's lifecycle status.
    pub fn update_status(&self, id: &str, status: AdrStatus) -> Result<AdrRecord, AdrError> {
        let mut record = self.get(id)?;
        record.status = status;
        self.write(&record)?;
        Ok(record)
    }

    /// Deletes one ADR. Errors with [`AdrError::NotFound`] if it doesn't exist.
    pub fn delete(&self, id: &str) -> Result<(), AdrError> {
        let path = self.path_for(id);
        if !path.exists() {
            return Err(AdrError::NotFound(id.to_owned()));
        }
        std::fs::remove_file(&path).map_err(|error| AdrError::Io(error.to_string()))
    }

    /// Lists every ADR, sorted by id.
    pub fn list(&self) -> Vec<AdrSummary> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut summaries: Vec<AdrSummary> = entries
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "json"))
            .filter_map(|entry| JsonStore.read::<AdrRecord>(&entry.path()).ok().flatten())
            .map(|record| AdrSummary {
                id: record.id,
                title: record.title,
                status: record.status,
            })
            .collect();
        summaries.sort_by(|a, b| a.id.cmp(&b.id));
        summaries
    }

    fn path_for(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn write(&self, record: &AdrRecord) -> Result<(), AdrError> {
        if !self.dir.exists() {
            std::fs::create_dir_all(&self.dir).map_err(|error| AdrError::Io(error.to_string()))?;
        }
        JsonStore
            .write(&self.path_for(&record.id), record)
            .map_err(|error| AdrError::Io(error.to_string()))
    }

    /// Assigns the next sequential `ADR-NNNN` id from the highest existing
    /// numeric suffix, so ids stay stable and gap-free even after deletes
    /// (a deleted id is never reused as long as a higher one still exists,
    /// avoiding collisions with references already made to it elsewhere).
    fn next_id(&self) -> String {
        let next = self
            .list()
            .iter()
            .filter_map(|summary| summary.id.strip_prefix("ADR-"))
            .filter_map(|suffix| suffix.parse::<u32>().ok())
            .max()
            .unwrap_or(0)
            + 1;
        format!("ADR-{next:04}")
    }
}

fn non_empty<'a>(_field: &str, value: &'a str) -> Result<&'a str, ()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        Err(())
    } else {
        Ok(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::{AdrError, AdrStatus, AdrStore};

    #[test]
    fn create_assigns_sequential_ids_and_persists() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(temp.path());

        let first = store.create("Use Postgres", "We need a database.", "Use Postgres.", None)?;
        assert_eq!(first.id, "ADR-0001");
        assert_eq!(first.status, AdrStatus::Proposed);
        assert_eq!(
            first.sections.get("context").map(String::as_str),
            Some("We need a database.")
        );
        assert!(!first.sections.contains_key("consequences"));

        let second = store.create(
            "Use gRPC",
            "Services need to talk to each other.",
            "Use gRPC.",
            Some("Requires protobuf tooling."),
        )?;
        assert_eq!(second.id, "ADR-0002");
        assert_eq!(
            second.sections.get("consequences").map(String::as_str),
            Some("Requires protobuf tooling.")
        );

        let fetched = store.get("ADR-0001")?;
        assert_eq!(fetched, first);

        Ok(())
    }

    #[test]
    fn create_rejects_empty_title_or_section() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(temp.path());

        assert_eq!(
            store.create("  ", "context", "decision", None),
            Err(AdrError::EmptyTitle)
        );
        assert_eq!(
            store.create("Title", "  ", "decision", None),
            Err(AdrError::EmptySection("context".to_owned()))
        );
        assert_eq!(
            store.create("Title", "context", "  ", None),
            Err(AdrError::EmptySection("decision".to_owned()))
        );

        Ok(())
    }

    #[test]
    fn update_section_validates_key_and_content() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(temp.path());
        let record = store.create("Use Postgres", "We need a database.", "Use Postgres.", None)?;

        assert_eq!(
            store.update_section(&record.id, "not-a-real-section", "value"),
            Err(AdrError::UnknownSection("not-a-real-section".to_owned()))
        );
        assert_eq!(
            store.update_section(&record.id, "consequences", "   "),
            Err(AdrError::EmptySection("consequences".to_owned()))
        );

        let updated =
            store.update_section(&record.id, "consequences", "Adds an ops dependency.")?;
        assert_eq!(
            updated.sections.get("consequences").map(String::as_str),
            Some("Adds an ops dependency.")
        );
        assert_eq!(store.get(&record.id)?, updated);

        Ok(())
    }

    #[test]
    fn update_status_and_delete_and_list_behave_as_documented()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(temp.path());
        let a = store.create("Use Postgres", "context", "decision", None)?;
        let b = store.create("Use gRPC", "context", "decision", None)?;

        let accepted = store.update_status(&a.id, AdrStatus::Accepted)?;
        assert_eq!(accepted.status, AdrStatus::Accepted);

        let list = store.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, a.id);
        assert_eq!(list[0].status, AdrStatus::Accepted);
        assert_eq!(list[1].id, b.id);

        store.delete(&a.id)?;
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get(&a.id), Err(AdrError::NotFound(a.id.clone())));
        assert_eq!(store.delete(&a.id), Err(AdrError::NotFound(a.id)));

        Ok(())
    }

    #[test]
    fn operations_on_a_missing_id_are_not_found() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(temp.path());

        assert_eq!(
            store.get("ADR-9999"),
            Err(AdrError::NotFound("ADR-9999".to_owned()))
        );
        assert_eq!(
            store.update_section("ADR-9999", "context", "value"),
            Err(AdrError::NotFound("ADR-9999".to_owned()))
        );
        assert_eq!(
            store.update_status("ADR-9999", AdrStatus::Accepted),
            Err(AdrError::NotFound("ADR-9999".to_owned()))
        );

        Ok(())
    }
}
