//! Cargo workspace, crate, target, dependency, and feature facts resolved
//! via the `cargo metadata` subprocess.
//!
//! `docs/dev/parser-spike-decisions.md` records why `cargo metadata` is used
//! here rather than extending the raw-TOML `CargoProfileAnalyzer`: it
//! resolves facts TOML parsing cannot, such as the implicit binary target
//! cargo infers when no `[[bin]]` table is present.

use crate::domain::{Artifact, ArtifactId, EvidenceRef, ModelExposurePolicy, TextStatus};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Resolved Cargo workspace analysis for one `Cargo.toml` artifact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RustWorkspaceAnalysis {
    /// Packages resolved from this manifest (one, unless it is a workspace root).
    pub packages: Vec<RustPackage>,
    /// Error message when `cargo metadata` failed to resolve the manifest.
    pub error: Option<String>,
}

/// One resolved Cargo package.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustPackage {
    /// Package name.
    pub name: String,
    /// Package version.
    pub version: String,
    /// Repository-relative path to this package's manifest.
    pub manifest_path: String,
    /// Build targets, including binaries cargo infers implicitly.
    pub targets: Vec<RustTarget>,
    /// Declared features and the features/dependencies they enable.
    pub features: Vec<RustFeature>,
    /// Declared dependencies.
    pub dependencies: Vec<RustDependency>,
    /// Evidence for this package's manifest artifact.
    pub evidence: EvidenceRef,
}

/// One Cargo build target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustTarget {
    /// Target name.
    pub name: String,
    /// Target kinds, e.g. `lib`, `bin`, `test`, `example`, `bench`.
    pub kinds: Vec<String>,
    /// Repository-relative source entrypoint path.
    pub path: String,
}

/// One Cargo feature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustFeature {
    /// Feature name.
    pub name: String,
    /// Features and optional dependencies this feature enables.
    pub enables: Vec<String>,
}

/// Dependency table category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustDependencyKind {
    /// `[dependencies]`.
    Normal,
    /// `[dev-dependencies]`.
    Dev,
    /// `[build-dependencies]`.
    Build,
}

/// One resolved dependency requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustDependency {
    /// Dependency crate name.
    pub name: String,
    /// Version requirement as declared.
    pub requirement: String,
    /// Dependency table category.
    pub kind: RustDependencyKind,
}

/// `cargo metadata`-backed analyzer for Cargo manifests.
///
/// Unlike other analyzers this needs real filesystem access: `cargo
/// metadata` reads the manifest (and any parent workspace manifest) from
/// disk rather than from in-memory artifact text.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustWorkspaceAnalyzer;

impl RustWorkspaceAnalyzer {
    /// Resolves workspace/crate/target/dependency/feature facts for a Cargo
    /// manifest artifact rooted at `repo_root`.
    pub fn analyze(&self, artifact: &Artifact, repo_root: &Path) -> RustWorkspaceAnalysis {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return RustWorkspaceAnalysis::default();
        }

        let manifest_path = repo_root.join(artifact.path.as_str());
        match cargo_metadata::MetadataCommand::new()
            .manifest_path(&manifest_path)
            .no_deps()
            .exec()
        {
            Ok(metadata) => RustWorkspaceAnalysis {
                packages: metadata
                    .packages
                    .iter()
                    .map(|package| build_package(artifact, repo_root, package))
                    .collect(),
                error: None,
            },
            Err(error) => RustWorkspaceAnalysis {
                packages: Vec::new(),
                error: Some(error.to_string()),
            },
        }
    }
}

fn build_package(
    artifact: &Artifact,
    repo_root: &Path,
    package: &cargo_metadata::Package,
) -> RustPackage {
    let targets = package
        .targets
        .iter()
        .map(|target| RustTarget {
            name: target.name.clone(),
            kinds: target.kind.iter().map(ToString::to_string).collect(),
            path: repo_relative(repo_root, target.src_path.as_std_path()),
        })
        .collect();
    let features = package
        .features
        .iter()
        .map(|(name, enables)| RustFeature {
            name: name.clone(),
            enables: enables.clone(),
        })
        .collect();
    let dependencies = package
        .dependencies
        .iter()
        .map(|dependency| RustDependency {
            name: dependency.name.clone(),
            requirement: dependency.req.to_string(),
            kind: match dependency.kind {
                cargo_metadata::DependencyKind::Development => RustDependencyKind::Dev,
                cargo_metadata::DependencyKind::Build => RustDependencyKind::Build,
                _ => RustDependencyKind::Normal,
            },
        })
        .collect();

    RustPackage {
        name: package.name.to_string(),
        version: package.version.to_string(),
        manifest_path: repo_relative(repo_root, package.manifest_path.as_std_path()),
        targets,
        features,
        dependencies,
        evidence: EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone()),
    }
}

fn repo_relative(repo_root: &Path, path: &Path) -> String {
    path.strip_prefix(repo_root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::{RustDependencyKind, RustWorkspaceAnalyzer};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn manifest_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::PackageManifest,
            SupportTier::StructuredFormat,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_detected_format("toml")
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn resolves_fixture_crate_targets_features_and_dependencies()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = manifest_artifact("rust/Cargo.toml")?;
        let analysis = RustWorkspaceAnalyzer.analyze(&artifact, &fixture_root());

        assert!(analysis.error.is_none());
        let package = &analysis.packages[0];
        assert_eq!(package.name, "fixture-worker");
        assert_eq!(package.manifest_path, "rust/Cargo.toml");

        let lib = package
            .targets
            .iter()
            .find(|target| target.kinds.iter().any(|kind| kind == "lib"))
            .ok_or("lib target")?;
        assert_eq!(lib.path, "rust/src/lib.rs");
        let bin = package
            .targets
            .iter()
            .find(|target| target.kinds.iter().any(|kind| kind == "bin"))
            .ok_or("bin target")?;
        assert_eq!(bin.name, "worker");
        assert_eq!(bin.path, "rust/src/bin/worker.rs");

        assert!(
            package
                .features
                .iter()
                .any(|feature| feature.name == "default"
                    && feature.enables == vec!["serde-support".to_owned()])
        );

        let dependency = package
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "anyhow")
            .ok_or("anyhow dependency")?;
        assert_eq!(dependency.kind, RustDependencyKind::Normal);

        Ok(())
    }

    #[test]
    fn reports_error_for_unresolvable_manifest_and_respects_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let missing = manifest_artifact("does/not/exist/Cargo.toml")?;
        let analysis = RustWorkspaceAnalyzer.analyze(&missing, &fixture_root());
        assert!(analysis.error.is_some());
        assert!(analysis.packages.is_empty());

        let never = manifest_artifact("rust/Cargo.toml")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        assert_eq!(
            RustWorkspaceAnalyzer.analyze(&never, &fixture_root()),
            super::RustWorkspaceAnalysis::default()
        );

        Ok(())
    }
}
