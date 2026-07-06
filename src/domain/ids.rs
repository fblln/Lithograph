//! Stable identifier and path types.

use camino::Utf8PathBuf;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// Repository-relative UTF-8 path.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepoPath(Utf8PathBuf);

impl RepoPath {
    /// Creates a repository-relative path.
    ///
    /// Lithograph stores repository paths as normalized relative paths so graph
    /// evidence remains portable across machines and checkouts. Absolute paths
    /// are rejected here instead of being cleaned up later.
    pub fn new(path: impl Into<Utf8PathBuf>) -> Result<Self, RepoPathError> {
        let path = path.into();
        if path.is_absolute() {
            return Err(RepoPathError::AbsolutePath(path));
        }
        if path.as_str().is_empty() {
            return Err(RepoPathError::EmptyPath);
        }

        Ok(Self(path))
    }

    /// Returns the path as a string slice.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl Display for RepoPath {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned when a repository path is not valid for graph storage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepoPathError {
    /// Empty paths cannot identify artifacts.
    EmptyPath,
    /// Host-absolute paths would make evidence non-portable.
    AbsolutePath(Utf8PathBuf),
}

impl Display for RepoPathError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyPath => formatter.write_str("repository path cannot be empty"),
            Self::AbsolutePath(path) => {
                write!(formatter, "repository path must be relative: {path}")
            }
        }
    }
}

impl std::error::Error for RepoPathError {}

/// Content hash string for artifacts.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentHash(String);

impl ContentHash {
    /// Creates a content hash from a precomputed lowercase digest.
    pub fn new(value: impl Into<String>) -> Result<Self, ContentHashError> {
        let value = value.into();
        if value.is_empty() {
            return Err(ContentHashError::Empty);
        }
        if !value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(ContentHashError::InvalidHex(value));
        }

        Ok(Self(value))
    }

    /// Returns the hash as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ContentHash {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Error returned when a content hash is malformed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentHashError {
    /// Empty content hashes cannot identify content.
    Empty,
    /// Hash values must be lowercase hexadecimal strings.
    InvalidHex(String),
}

impl Display for ContentHashError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Empty => formatter.write_str("content hash cannot be empty"),
            Self::InvalidHex(value) => {
                write!(
                    formatter,
                    "content hash must be lowercase hexadecimal: {value}"
                )
            }
        }
    }
}

impl std::error::Error for ContentHashError {}

/// Stable artifact identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ArtifactId(String);

impl ArtifactId {
    /// Builds an artifact ID from its repository-relative path.
    pub fn from_path(path: &RepoPath) -> Self {
        Self(format!("artifact:{}", path.as_str()))
    }

    /// Returns the ID as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for ArtifactId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{ArtifactId, ContentHash, ContentHashError, RepoPath, RepoPathError};

    #[test]
    fn repo_path_rejects_empty_and_absolute_paths() {
        assert_eq!(RepoPath::new("").err(), Some(RepoPathError::EmptyPath));
        assert!(matches!(
            RepoPath::new("/tmp/repo/file.rs"),
            Err(RepoPathError::AbsolutePath(_))
        ));
    }

    #[test]
    fn artifact_id_is_stable_from_repo_path() -> Result<(), Box<dyn std::error::Error>> {
        let path = RepoPath::new("src/lib.rs")?;
        let id = ArtifactId::from_path(&path);

        assert_eq!(id.as_str(), "artifact:src/lib.rs");
        assert_eq!(id.to_string(), "artifact:src/lib.rs");

        Ok(())
    }

    #[test]
    fn content_hash_accepts_lowercase_hex_only() -> Result<(), Box<dyn std::error::Error>> {
        let hash = ContentHash::new("0123abcd")?;

        assert_eq!(hash.as_str(), "0123abcd");
        assert_eq!(hash.to_string(), "0123abcd");
        assert_eq!(ContentHash::new("").err(), Some(ContentHashError::Empty));
        assert_eq!(
            ContentHash::new("ABC").err(),
            Some(ContentHashError::InvalidHex("ABC".to_owned()))
        );

        Ok(())
    }

    #[test]
    fn repo_path_and_errors_have_actionable_display_messages()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = RepoPath::new("docs/README.md")?;
        let absolute_error = RepoPath::new("/tmp/repo/docs/README.md").err();

        assert_eq!(path.to_string(), "docs/README.md");
        assert_eq!(
            RepoPathError::EmptyPath.to_string(),
            "repository path cannot be empty"
        );
        assert_eq!(
            absolute_error.map(|error| error.to_string()),
            Some("repository path must be relative: /tmp/repo/docs/README.md".to_owned())
        );

        Ok(())
    }

    #[test]
    fn content_hash_errors_have_actionable_display_messages() {
        assert_eq!(
            ContentHashError::Empty.to_string(),
            "content hash cannot be empty"
        );
        assert_eq!(
            ContentHashError::InvalidHex("CAFE".to_owned()).to_string(),
            "content hash must be lowercase hexadecimal: CAFE"
        );
    }

    #[test]
    fn ids_serialize_as_stable_strings() -> Result<(), Box<dyn std::error::Error>> {
        let path = RepoPath::new("src/main.rs")?;
        let artifact_id = ArtifactId::from_path(&path);
        let hash = ContentHash::new("decaf")?;

        assert_eq!(serde_json::to_string(&path)?, "\"src/main.rs\"");
        assert_eq!(
            serde_json::to_string(&artifact_id)?,
            "\"artifact:src/main.rs\""
        );
        assert_eq!(serde_json::to_string(&hash)?, "\"decaf\"");

        let round_tripped_path: RepoPath = serde_json::from_str("\"src/main.rs\"")?;
        let round_tripped_hash: ContentHash = serde_json::from_str("\"decaf\"")?;

        assert_eq!(round_tripped_path, path);
        assert_eq!(round_tripped_hash, hash);

        Ok(())
    }
}
