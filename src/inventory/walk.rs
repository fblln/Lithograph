//! Safe repository walking and artifact metadata extraction.

use crate::domain::{Artifact, ContentHash, RepoPath, TextStatus};
use crate::inventory::classify::{ArtifactClassifier, ClassificationInput};
use crate::inventory::safety::SafetyPolicy;
use camino::Utf8PathBuf;
use globset::{Glob, GlobMatcher};
use ignore::WalkBuilder;
use std::fmt::{Display, Formatter};
use std::fs;
use std::path::{Path, PathBuf};

/// Options controlling repository walking.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkOptions {
    /// Glob patterns, relative to the repository root, that should be excluded.
    pub exclude_globs: Vec<String>,
    /// Whether dot-prefixed files and directories should be included.
    pub include_hidden: bool,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            exclude_globs: Vec::new(),
            include_hidden: true,
        }
    }
}

/// Repository walker that converts files into baseline artifacts.
#[derive(Debug, Clone)]
pub struct RepositoryWalker {
    options: WalkOptions,
}

impl RepositoryWalker {
    /// Creates a walker with the supplied options.
    pub fn new(options: WalkOptions) -> Self {
        Self { options }
    }

    /// Walks a repository root and returns baseline artifacts in stable path order.
    pub fn walk(&self, root: &Path) -> Result<Vec<Artifact>, WalkError> {
        let root = root
            .canonicalize()
            .map_err(|source| WalkError::CanonicalizeRoot {
                path: root.to_path_buf(),
                source,
            })?;
        if !root.is_dir() {
            return Err(WalkError::RootNotDirectory(root));
        }

        let excludes = self.build_exclude_set()?;
        let mut artifacts = Vec::new();
        let mut builder = WalkBuilder::new(&root);
        builder
            .hidden(!self.options.include_hidden)
            .follow_links(false)
            .git_ignore(true)
            .git_global(false)
            .git_exclude(true)
            .require_git(false)
            .parents(false)
            // `.git`'s own object/ref store is never a documentable
            // artifact, unlike other hidden directories such as
            // `.github/`; prune it outright rather than relying on
            // `include_hidden`, which controls hidden files generally.
            .filter_entry(|entry| entry.file_name() != std::ffi::OsStr::new(".git"))
            .sort_by_file_path(|left, right| left.cmp(right));

        for entry in builder.build() {
            let entry = entry.map_err(WalkError::from_ignore_error)?;
            if !entry
                .file_type()
                .is_some_and(|file_type| file_type.is_file())
            {
                continue;
            }

            let path = entry.path();
            let relative_path = relative_path(&root, path)?;
            if excludes
                .iter()
                .any(|exclude| exclude.is_match(relative_path.as_str()))
            {
                continue;
            }

            artifacts.push(read_artifact(path, relative_path)?);
        }

        Ok(artifacts)
    }

    fn build_exclude_set(&self) -> Result<Vec<GlobMatcher>, WalkError> {
        self.options
            .exclude_globs
            .iter()
            .map(|pattern| {
                Ok(Glob::new(pattern)
                    .map_err(|source| WalkError::InvalidExcludeGlob {
                        pattern: pattern.clone(),
                        source,
                    })?
                    .compile_matcher())
            })
            .collect()
    }
}

fn relative_path(root: &Path, path: &Path) -> Result<RepoPath, WalkError> {
    let relative = path
        .strip_prefix(root)
        .map_err(|source| WalkError::PathEscapedRoot {
            root: root.to_path_buf(),
            path: path.to_path_buf(),
            source,
        })?;
    let relative = Utf8PathBuf::from_path_buf(relative.to_path_buf())
        .map_err(WalkError::NonUtf8RepositoryPath)?;
    RepoPath::new(relative).map_err(WalkError::InvalidRepoPath)
}

fn read_artifact(path: &Path, relative_path: RepoPath) -> Result<Artifact, WalkError> {
    let metadata = fs::metadata(path).map_err(|source| WalkError::ReadMetadata {
        path: path.to_path_buf(),
        source,
    })?;
    let bytes = fs::read(path).map_err(|source| WalkError::ReadFile {
        path: path.to_path_buf(),
        source,
    })?;
    let hash = ContentHash::new(blake3::hash(&bytes).to_hex().to_string())
        .map_err(WalkError::InvalidContentHash)?;

    let (mut text_status, mut line_count, mut text) = if bytes.contains(&0) {
        (TextStatus::Binary, None, None)
    } else if let Ok(text) = std::str::from_utf8(&bytes) {
        (TextStatus::Text, Some(line_count(text)), Some(text))
    } else {
        (TextStatus::Binary, None, None)
    };
    let safety_policy = SafetyPolicy;
    let safety_decision = safety_policy.decide(relative_path.as_str(), text_status);
    if safety_decision.metadata_only {
        text_status = safety_decision.text_status;
        line_count = None;
        text = None;
    }
    let classification = ArtifactClassifier.classify(ClassificationInput {
        path: &relative_path,
        text_status,
        text,
    });
    let classification = safety_policy.apply(classification, safety_decision);
    let mut artifact = Artifact::new(
        relative_path,
        classification.category,
        classification.support_tier,
        hash,
        metadata.len(),
    )
    .with_text_status(text_status, line_count)
    .with_origin_scores(
        classification.generated_score,
        classification.vendored_score,
    )
    .with_model_policy(classification.model_policy)
    .with_analyzer(classification.analyzer);
    if let Some(detected_format) = classification.detected_format {
        artifact = artifact.with_detected_format(detected_format);
    }

    Ok(artifact)
}

fn line_count(text: &str) -> u32 {
    if text.is_empty() {
        0
    } else {
        text.lines().count().try_into().unwrap_or(u32::MAX)
    }
}

/// Error returned by repository walking.
#[derive(Debug)]
pub enum WalkError {
    /// Repository root could not be canonicalized.
    CanonicalizeRoot {
        /// Root path requested by the caller.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// Root exists but is not a directory.
    RootNotDirectory(PathBuf),
    /// Configured exclude glob is invalid.
    InvalidExcludeGlob {
        /// Invalid glob pattern.
        pattern: String,
        /// Glob parser error.
        source: globset::Error,
    },
    /// Repository walker returned an error.
    Walk(ignore::Error),
    /// A discovered path was outside the repository root.
    PathEscapedRoot {
        /// Canonical repository root.
        root: PathBuf,
        /// Discovered path.
        path: PathBuf,
        /// Prefix stripping error.
        source: std::path::StripPrefixError,
    },
    /// Repository path was not UTF-8.
    NonUtf8RepositoryPath(PathBuf),
    /// Repository-relative path failed domain validation.
    InvalidRepoPath(crate::domain::ids::RepoPathError),
    /// File bytes could not be read.
    ReadFile {
        /// File path.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// File metadata could not be read.
    ReadMetadata {
        /// File path.
        path: PathBuf,
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// Content hash failed validation.
    InvalidContentHash(crate::domain::ids::ContentHashError),
}

impl WalkError {
    fn from_ignore_error(error: ignore::Error) -> Self {
        Self::Walk(error)
    }
}

impl Display for WalkError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CanonicalizeRoot { path, source } => {
                write!(
                    formatter,
                    "failed to canonicalize root {}: {source}",
                    path.display()
                )
            }
            Self::RootNotDirectory(path) => {
                write!(
                    formatter,
                    "repository root is not a directory: {}",
                    path.display()
                )
            }
            Self::InvalidExcludeGlob { pattern, source } => {
                write!(formatter, "invalid exclude glob {pattern:?}: {source}")
            }
            Self::Walk(source) => write!(formatter, "failed while walking repository: {source}"),
            Self::PathEscapedRoot { root, path, source } => write!(
                formatter,
                "path {} escaped repository root {}: {source}",
                path.display(),
                root.display()
            ),
            Self::NonUtf8RepositoryPath(path) => {
                write!(
                    formatter,
                    "repository path is not valid UTF-8: {}",
                    path.display()
                )
            }
            Self::InvalidRepoPath(source) => {
                write!(formatter, "invalid repository-relative path: {source}")
            }
            Self::ReadFile { path, source } => {
                write!(
                    formatter,
                    "failed to read file {}: {source}",
                    path.display()
                )
            }
            Self::ReadMetadata { path, source } => {
                write!(
                    formatter,
                    "failed to read metadata for {}: {source}",
                    path.display()
                )
            }
            Self::InvalidContentHash(source) => {
                write!(formatter, "invalid content hash: {source}")
            }
        }
    }
}

impl std::error::Error for WalkError {}

#[cfg(test)]
mod tests {
    use super::{RepositoryWalker, WalkError, WalkOptions, read_artifact, relative_path};
    use crate::domain::{
        AnalyzerSelection, ArtifactCategory, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
        ids::{ContentHashError, RepoPathError},
    };
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn fixture_scan_count_is_stable() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;

        assert_eq!(artifacts.len(), 23);
        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.path.as_str() == ".github/workflows/ci.yml")
        );
        assert!(
            artifacts
                .iter()
                .any(|artifact| artifact.path.as_str() == "data/sample.bin")
        );

        Ok(())
    }

    #[test]
    fn artifacts_record_path_size_and_hash() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let readme = find_artifact(&artifacts, "README.md")?;
        let binary = find_artifact(&artifacts, "data/sample.bin")?;

        assert_eq!(readme.id.as_str(), "artifact:README.md");
        assert!(readme.size_bytes > 0);
        assert_eq!(readme.content_hash.as_str().len(), 64);
        assert_eq!(readme.text_status, TextStatus::Text);
        assert!(readme.line_count.is_some());
        assert_eq!(
            readme.analyzer,
            AnalyzerSelection::Structured("markdown".to_owned())
        );

        assert_eq!(binary.category, ArtifactCategory::BinaryAsset);
        assert_eq!(binary.text_status, TextStatus::Binary);
        assert_eq!(binary.model_policy, ModelExposurePolicy::Never);
        assert_eq!(binary.analyzer, AnalyzerSelection::Opaque);

        Ok(())
    }

    #[test]
    fn records_empty_text_files_and_invalid_utf8_as_binary()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        fs::write(temp.path().join("empty.txt"), "")?;
        fs::write(temp.path().join("invalid.dat"), [0xff, 0xfe])?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let empty = find_artifact(&artifacts, "empty.txt")?;
        let invalid = find_artifact(&artifacts, "invalid.dat")?;

        assert_eq!(empty.text_status, TextStatus::Text);
        assert_eq!(empty.line_count, Some(0));
        assert_eq!(invalid.category, ArtifactCategory::BinaryAsset);
        assert_eq!(invalid.text_status, TextStatus::Binary);
        assert_eq!(invalid.model_policy, ModelExposurePolicy::Never);

        Ok(())
    }

    #[test]
    fn respects_gitignore_and_configured_excludes() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        fs::write(temp.path().join(".gitignore"), "ignored.txt\n")?;
        fs::write(temp.path().join("kept.txt"), "kept\n")?;
        fs::write(temp.path().join("ignored.txt"), "ignored\n")?;
        fs::create_dir(temp.path().join("build"))?;
        fs::write(temp.path().join("build/output.txt"), "generated\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions {
            exclude_globs: vec!["build/**".to_owned()],
            include_hidden: true,
        })
        .walk(temp.path())?;
        let paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();

        assert_eq!(paths, vec![".gitignore", "kept.txt"]);

        Ok(())
    }

    #[test]
    fn never_walks_into_the_git_directory() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join(".git/objects/pack"))?;
        fs::write(
            temp.path().join(".git/objects/pack/pack-deadbeef.pack"),
            [0u8; 4],
        )?;
        fs::write(temp.path().join(".git/HEAD"), "ref: refs/heads/main\n")?;
        fs::write(temp.path().join("kept.txt"), "kept\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions {
            include_hidden: true,
            ..WalkOptions::default()
        })
        .walk(temp.path())?;
        let paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();

        assert_eq!(paths, vec!["kept.txt"]);

        Ok(())
    }

    #[test]
    fn rejects_invalid_roots_and_exclude_globs() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let file_root = temp.path().join("file.txt");
        let missing_root = temp.path().join("missing");
        fs::write(&file_root, "file\n")?;

        let not_directory = RepositoryWalker::new(WalkOptions::default()).walk(&file_root);
        assert!(matches!(
            not_directory,
            Err(WalkError::RootNotDirectory(path)) if path == file_root.canonicalize()?
        ));

        let missing = RepositoryWalker::new(WalkOptions::default()).walk(&missing_root);
        assert!(matches!(
            missing,
            Err(WalkError::CanonicalizeRoot { path, .. }) if path == missing_root
        ));

        let invalid_glob = RepositoryWalker::new(WalkOptions {
            exclude_globs: vec!["[".to_owned()],
            include_hidden: true,
        })
        .walk(temp.path());
        assert!(matches!(
            invalid_glob,
            Err(WalkError::InvalidExcludeGlob { pattern, .. }) if pattern == "["
        ));

        Ok(())
    }

    #[test]
    fn secret_fixtures_are_metadata_only() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join("config"))?;
        fs::create_dir_all(temp.path().join("keys"))?;
        fs::write(temp.path().join(".env"), "TOKEN=secret\n")?;
        let private_key = "-----BEGIN PRIVATE KEY-----\nsecret\n-----END PRIVATE KEY-----\n";
        let secrets_path = temp.path().join("config/secrets.yaml");
        fs::write(secrets_path, "password: secret\n")?;
        fs::write(temp.path().join("keys/private.pem"), private_key)?;
        fs::write(temp.path().join("config/public.yaml"), "name: lithograph\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        for path in [".env", "config/secrets.yaml", "keys/private.pem"] {
            let artifact = find_artifact(&artifacts, path)?;
            assert_eq!(artifact.text_status, TextStatus::UnsafeText);
            assert_eq!(artifact.line_count, None);
            assert_eq!(artifact.support_tier, SupportTier::Opaque);
            assert_eq!(artifact.model_policy, ModelExposurePolicy::Never);
            assert_eq!(artifact.analyzer, AnalyzerSelection::Opaque);
        }

        let public_config = find_artifact(&artifacts, "config/public.yaml")?;
        assert_eq!(public_config.text_status, TextStatus::Text);
        assert_ne!(public_config.model_policy, ModelExposurePolicy::Never);

        Ok(())
    }

    #[test]
    fn does_not_follow_symlinks_outside_root() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let outside = TempDir::new()?;
        fs::write(temp.path().join("inside.txt"), "inside\n")?;
        fs::write(outside.path().join("outside.txt"), "outside\n")?;

        let symlink_target = outside.path().join("outside.txt");
        let symlink_path = temp.path().join("outside-link.txt");
        create_symlink(&symlink_target, &symlink_path)?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();

        assert_eq!(paths, vec!["inside.txt"]);

        Ok(())
    }

    #[test]
    fn helper_reports_missing_artifacts() -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = Vec::new();
        let error = find_artifact(&artifacts, "missing.txt").err();

        assert!(error.as_ref().is_some_and(|error| {
            error
                .to_string()
                .contains("missing.txt artifact should exist")
        }));

        Ok(())
    }

    #[test]
    fn helper_finds_existing_artifacts() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let artifact = find_artifact(&artifacts, "README.md")?;

        assert!(artifact.path.as_str().contains("README.md"));

        Ok(())
    }

    #[test]
    fn internal_path_and_read_errors_are_reported() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let outside = TempDir::new()?;
        let root = temp.path().canonicalize()?;
        let escaped = outside.path().join("escaped.txt");
        let missing = temp.path().join("missing.txt");
        fs::write(&escaped, "escaped\n")?;

        let escaped_error = relative_path(&root, &escaped);
        assert!(matches!(
            escaped_error,
            Err(WalkError::PathEscapedRoot { path, .. }) if path == escaped
        ));

        let relative = RepoPath::new("missing.txt")?;
        let read_error = read_artifact(&missing, relative);
        assert!(matches!(
            read_error,
            Err(WalkError::ReadMetadata { path, .. }) if path == missing
        ));

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_files_report_read_errors() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new()?;
        let unreadable = temp.path().join("unreadable.txt");
        fs::write(&unreadable, "secret\n")?;

        let mut permissions = fs::metadata(&unreadable)?.permissions();
        permissions.set_mode(0o000);
        fs::set_permissions(&unreadable, permissions)?;

        let relative = RepoPath::new("unreadable.txt")?;
        let read_error = read_artifact(&unreadable, relative);

        let mut permissions = fs::metadata(&unreadable)?.permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&unreadable, permissions)?;

        assert!(matches!(
            read_error,
            Err(WalkError::ReadFile { path, .. }) if path == unreadable
        ));

        Ok(())
    }

    #[test]
    fn walk_error_display_messages_are_actionable() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        let file_path = temp.path().join("file.txt");
        let root = temp.path().join("root");
        let escaped = temp.path().join("escaped.txt");
        fs::write(&file_path, "file\n")?;
        fs::create_dir(&root)?;
        fs::write(&escaped, "escaped\n")?;

        let walk_error =
            WalkError::from_ignore_error(ignore::Error::Io(std::io::Error::other("walk")));

        let mut errors = vec![
            WalkError::CanonicalizeRoot {
                path: temp.path().join("missing"),
                source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
            },
            WalkError::RootNotDirectory(file_path.clone()),
            walk_error,
            WalkError::NonUtf8RepositoryPath(file_path.clone()),
            WalkError::InvalidRepoPath(RepoPathError::EmptyPath),
            WalkError::ReadFile {
                path: file_path.clone(),
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "read"),
            },
            WalkError::ReadMetadata {
                path: file_path,
                source: std::io::Error::new(std::io::ErrorKind::PermissionDenied, "metadata"),
            },
            WalkError::InvalidContentHash(ContentHashError::Empty),
        ];
        if let Err(error) = relative_path(&root, &escaped) {
            errors.push(error);
        }
        if let Err(error) = RepositoryWalker::new(WalkOptions {
            exclude_globs: vec!["[".to_owned()],
            include_hidden: true,
        })
        .walk(temp.path())
        {
            errors.push(error);
        }

        let rendered: Vec<String> = errors.iter().map(ToString::to_string).collect();

        assert!(
            rendered
                .iter()
                .any(|message| message.contains("failed to canonicalize root"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("repository root is not a directory"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("invalid exclude glob"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("failed while walking repository"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("escaped repository root"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("not valid UTF-8"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("invalid repository-relative path"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("failed to read file"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("failed to read metadata"))
        );
        assert!(
            rendered
                .iter()
                .any(|message| message.contains("invalid content hash"))
        );

        Ok(())
    }

    #[cfg(unix)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::unix::fs::symlink(target, link)
    }

    #[cfg(windows)]
    fn create_symlink(target: &Path, link: &Path) -> std::io::Result<()> {
        std::os::windows::fs::symlink_file(target, link)
    }

    fn find_artifact<'a>(
        artifacts: &'a [crate::domain::Artifact],
        path: &str,
    ) -> Result<&'a crate::domain::Artifact, Box<dyn std::error::Error>> {
        artifacts
            .iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("{path} artifact should exist"),
                )
                .into()
            })
    }
}
