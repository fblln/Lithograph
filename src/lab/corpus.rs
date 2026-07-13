//! Immutable external-corpus discovery, fetching, and verification.

use crate::lab::model::{CorpusCase, CorpusManifest, CorpusSource, LAB_SCHEMA_VERSION, SuiteTier};
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Corpus operation failure.
#[derive(Debug)]
pub enum CorpusError {
    /// Filesystem failure.
    Io(std::io::Error),
    /// TOML decoding failure.
    Toml(toml::de::Error),
    /// Invalid manifest contract.
    Invalid(String),
    /// Git command failed.
    Git(String),
}

impl Display for CorpusError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => Display::fmt(error, formatter),
            Self::Toml(error) => Display::fmt(error, formatter),
            Self::Invalid(message) | Self::Git(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CorpusError {}

impl From<std::io::Error> for CorpusError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<toml::de::Error> for CorpusError {
    fn from(value: toml::de::Error) -> Self {
        Self::Toml(value)
    }
}

/// Loaded corpus and its external repository cache.
#[derive(Debug, Clone)]
pub struct Corpus {
    manifest_path: PathBuf,
    cache_root: PathBuf,
    manifest: CorpusManifest,
}

impl Corpus {
    /// Loads and validates a corpus manifest.
    pub fn load(manifest_path: &Path, cache_root: &Path) -> Result<Self, CorpusError> {
        let text = std::fs::read_to_string(manifest_path)?;
        let manifest: CorpusManifest = toml::from_str(&text)?;
        validate_manifest(&manifest)?;
        Ok(Self {
            manifest_path: manifest_path.to_path_buf(),
            cache_root: cache_root.to_path_buf(),
            manifest,
        })
    }

    /// Validated manifest.
    pub fn manifest(&self) -> &CorpusManifest {
        &self.manifest
    }

    /// Cases belonging to `suite`, optionally narrowed by id.
    pub fn cases(&self, suite: SuiteTier, case_id: Option<&str>) -> Vec<&CorpusCase> {
        self.manifest
            .cases
            .iter()
            .filter(|case| suite.includes(case.tier))
            .filter(|case| case_id.is_none_or(|wanted| wanted == case.id))
            .collect()
    }

    /// Returns a case by stable id.
    pub fn case(&self, id: &str) -> Result<&CorpusCase, CorpusError> {
        self.manifest
            .cases
            .iter()
            .find(|case| case.id == id)
            .ok_or_else(|| CorpusError::Invalid(format!("unknown corpus case `{id}`")))
    }

    /// Fetches and verifies one Git case. Fixture cases require no action.
    pub fn fetch(&self, case: &CorpusCase) -> Result<PathBuf, CorpusError> {
        match &case.source {
            CorpusSource::Fixture { .. } => self.resolve_root(case),
            CorpusSource::Git { url, commit, tree } => {
                let root = self.git_root(case, commit);
                if root.is_dir() {
                    self.verify_git(&root, commit, tree)?;
                    return Ok(root);
                }
                if let Some(parent) = root.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let staging = root.with_extension("fetching");
                if staging.exists() {
                    std::fs::remove_dir_all(&staging)?;
                }
                run_git(
                    None,
                    [
                        "clone",
                        "--no-checkout",
                        "--filter=blob:none",
                        url,
                        path_str(&staging)?,
                    ],
                )?;
                run_git(Some(&staging), ["fetch", "--depth=1", "origin", commit])?;
                run_git(Some(&staging), ["checkout", "--detach", commit])?;
                self.verify_git(&staging, commit, tree)?;
                std::fs::rename(&staging, &root)?;
                Ok(root)
            }
        }
    }

    /// Resolves a case without network access, failing clearly when a Git
    /// case has not been fetched yet.
    pub fn resolve_root(&self, case: &CorpusCase) -> Result<PathBuf, CorpusError> {
        match &case.source {
            CorpusSource::Fixture { path } => {
                let base = self
                    .manifest_path
                    .parent()
                    .and_then(Path::parent)
                    .ok_or_else(|| {
                        CorpusError::Invalid("corpus manifest has no project root".to_owned())
                    })?;
                let root = base.join(path);
                root.is_dir().then_some(root).ok_or_else(|| {
                    CorpusError::Invalid(format!(
                        "fixture case `{}` does not exist at {}",
                        case.id,
                        base.join(path).display()
                    ))
                })
            }
            CorpusSource::Git { commit, tree, .. } => {
                let root = self.git_root(case, commit);
                if !root.is_dir() {
                    return Err(CorpusError::Invalid(format!(
                        "corpus case `{}` is not cached; run `lithograph-lab corpus fetch --case {}`",
                        case.id, case.id
                    )));
                }
                self.verify_git(&root, commit, tree)?;
                Ok(root)
            }
        }
    }

    /// Resolves the expectation file for a case.
    pub fn expectation_path(&self, case: &CorpusCase) -> Result<PathBuf, CorpusError> {
        let base = self
            .manifest_path
            .parent()
            .ok_or_else(|| CorpusError::Invalid("corpus manifest has no parent".to_owned()))?;
        Ok(base.join(&case.expectations))
    }

    /// Path of the reviewable committed baseline for one case.
    pub fn baseline_path(&self, case_id: &str) -> Result<PathBuf, CorpusError> {
        let base = self
            .manifest_path
            .parent()
            .ok_or_else(|| CorpusError::Invalid("corpus manifest has no parent".to_owned()))?;
        Ok(base.join("baselines").join(format!("{case_id}.json")))
    }

    /// Reviewed machine-dependent performance threshold manifest.
    pub fn performance_budget_path(&self) -> Result<PathBuf, CorpusError> {
        let base = self
            .manifest_path
            .parent()
            .ok_or_else(|| CorpusError::Invalid("corpus manifest has no parent".to_owned()))?;
        Ok(base.join("performance-budgets.json"))
    }

    fn git_root(&self, case: &CorpusCase, commit: &str) -> PathBuf {
        self.cache_root.join(&case.id).join(commit)
    }

    fn verify_git(&self, root: &Path, commit: &str, tree: &str) -> Result<(), CorpusError> {
        let actual_commit = git_output(root, ["rev-parse", "HEAD"])?;
        let actual_tree = git_output(root, ["rev-parse", "HEAD^{tree}"])?;
        let dirty = git_output(root, ["status", "--porcelain"])?;
        if actual_commit != commit || actual_tree != tree || !dirty.is_empty() {
            return Err(CorpusError::Invalid(format!(
                "corpus verification failed at {}: expected commit {commit} tree {tree} and a clean checkout, observed commit {actual_commit} tree {actual_tree} dirty={}",
                root.display(),
                dirty.escape_debug()
            )));
        }
        Ok(())
    }
}

fn validate_manifest(manifest: &CorpusManifest) -> Result<(), CorpusError> {
    if manifest.schema_version != LAB_SCHEMA_VERSION {
        return Err(CorpusError::Invalid(format!(
            "unsupported corpus schema {}, expected {}",
            manifest.schema_version, LAB_SCHEMA_VERSION
        )));
    }
    let mut ids = std::collections::BTreeSet::new();
    for case in &manifest.cases {
        if case.id.is_empty()
            || !case.id.chars().all(|character| {
                character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
            })
        {
            return Err(CorpusError::Invalid(format!(
                "corpus case id `{}` must use lowercase ASCII, digits, and hyphens",
                case.id
            )));
        }
        if !ids.insert(&case.id) {
            return Err(CorpusError::Invalid(format!(
                "duplicate corpus case `{}`",
                case.id
            )));
        }
        if case.license.trim().is_empty() || case.expectations.trim().is_empty() {
            return Err(CorpusError::Invalid(format!(
                "corpus case `{}` requires license and expectations",
                case.id
            )));
        }
        if let CorpusSource::Git { commit, tree, .. } = &case.source
            && (!is_object_id(commit) || !is_object_id(tree))
        {
            return Err(CorpusError::Invalid(format!(
                "corpus case `{}` must pin full hexadecimal commit and tree ids",
                case.id
            )));
        }
    }
    Ok(())
}

fn is_object_id(value: &str) -> bool {
    value.len() == 40 && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn run_git<const N: usize>(cwd: Option<&Path>, args: [&str; N]) -> Result<(), CorpusError> {
    let mut command = Command::new("git");
    command.args(args);
    if let Some(cwd) = cwd {
        command.current_dir(cwd);
    }
    let output = command.output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(CorpusError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn git_output<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<String, CorpusError> {
    let output = Command::new("git").args(args).current_dir(cwd).output()?;
    if !output.status.success() {
        return Err(CorpusError::Git(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn path_str(path: &Path) -> Result<&str, CorpusError> {
    path.to_str().ok_or_else(|| {
        CorpusError::Invalid(format!(
            "corpus cache path is not valid UTF-8: {}",
            path.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_duplicate_and_unpinned_cases() {
        let case = CorpusCase {
            id: "sample".to_owned(),
            tier: SuiteTier::Pr,
            source: CorpusSource::Git {
                url: "local".to_owned(),
                commit: "short".to_owned(),
                tree: "also-short".to_owned(),
            },
            license: "MIT".to_owned(),
            expectations: "sample.json".to_owned(),
            exclude: Vec::new(),
        };
        let manifest = CorpusManifest {
            schema_version: LAB_SCHEMA_VERSION,
            cases: vec![case.clone(), case],
        };
        assert!(validate_manifest(&manifest).is_err());
    }

    #[test]
    fn fetches_verifies_caches_and_rejects_a_dirty_checkout()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let source = temp.path().join("source");
        std::fs::create_dir_all(&source)?;
        test_git(&source, ["init", "-q"])?;
        test_git(&source, ["config", "user.email", "lab@example.invalid"])?;
        test_git(&source, ["config", "user.name", "Lithograph Lab"])?;
        std::fs::write(source.join("sample.py"), "def sample():\n    return 1\n")?;
        test_git(&source, ["add", "sample.py"])?;
        test_git(&source, ["commit", "-q", "-m", "fixture"])?;
        let commit = git_output(&source, ["rev-parse", "HEAD"])?;
        let tree = git_output(&source, ["rev-parse", "HEAD^{tree}"])?;
        let lab_dir = temp.path().join("lab");
        std::fs::create_dir_all(&lab_dir)?;
        let manifest_path = lab_dir.join("corpus.toml");
        std::fs::write(
            &manifest_path,
            format!(
                "schema_version = 2\n[[cases]]\nid = \"local\"\ntier = \"merge\"\nsource = \"git\"\nurl = {:?}\ncommit = \"{}\"\ntree = \"{}\"\nlicense = \"MIT\"\nexpectations = \"local.json\"\n",
                source.display().to_string(),
                commit,
                tree
            ),
        )?;
        let corpus = Corpus::load(&manifest_path, &temp.path().join("cache"))?;
        let case = corpus.case("local")?;
        let fetched = corpus.fetch(case)?;
        assert_eq!(corpus.fetch(case)?, fetched);
        std::fs::write(fetched.join("sample.py"), "corrupted\n")?;
        assert!(corpus.resolve_root(case).is_err());
        Ok(())
    }

    fn test_git<const N: usize>(cwd: &Path, args: [&str; N]) -> Result<(), CorpusError> {
        run_git(Some(cwd), args)
    }
}
