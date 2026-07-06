//! Repository artifact model.

use crate::domain::ids::{ArtifactId, ContentHash, RepoPath};
use serde::{Deserialize, Serialize};

/// Repository artifact discovered during inventory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Artifact {
    /// Stable path-derived artifact identifier.
    pub id: ArtifactId,
    /// Repository-relative artifact path.
    pub path: RepoPath,
    /// Coarse artifact category.
    pub category: ArtifactCategory,
    /// Detected format or language, such as `python`, `rust`, `yaml`, or `dockerfile`.
    pub detected_format: Option<String>,
    /// Level of semantic support available for this artifact.
    pub support_tier: SupportTier,
    /// Content digest from repository inventory.
    pub content_hash: ContentHash,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Line count for safe text artifacts.
    pub line_count: Option<u32>,
    /// Text/binary handling status.
    pub text_status: TextStatus,
    /// Generated-file probability from 0 to 100.
    pub generated_score: u8,
    /// Vendored-file probability from 0 to 100.
    pub vendored_score: u8,
    /// Whether content can be exposed to an external model.
    pub model_policy: ModelExposurePolicy,
    /// Analyzer selected for this artifact.
    pub analyzer: AnalyzerSelection,
}

impl Artifact {
    /// Creates a new artifact with a path-derived ID.
    pub fn new(
        path: RepoPath,
        category: ArtifactCategory,
        support_tier: SupportTier,
        content_hash: ContentHash,
        size_bytes: u64,
    ) -> Self {
        Self {
            id: ArtifactId::from_path(&path),
            path,
            category,
            detected_format: None,
            support_tier,
            content_hash,
            size_bytes,
            line_count: None,
            text_status: TextStatus::Unknown,
            generated_score: 0,
            vendored_score: 0,
            model_policy: ModelExposurePolicy::Allowed,
            analyzer: AnalyzerSelection::Unassigned,
        }
    }

    /// Assigns detected format or language.
    pub fn with_detected_format(mut self, detected_format: impl Into<String>) -> Self {
        self.detected_format = Some(detected_format.into());
        self
    }

    /// Assigns text status and optional line count.
    pub fn with_text_status(mut self, text_status: TextStatus, line_count: Option<u32>) -> Self {
        self.text_status = text_status;
        self.line_count = line_count;
        self
    }

    /// Assigns generated and vendored probability scores.
    pub fn with_origin_scores(mut self, generated_score: u8, vendored_score: u8) -> Self {
        self.generated_score = generated_score.min(100);
        self.vendored_score = vendored_score.min(100);
        self
    }

    /// Assigns model exposure policy.
    pub fn with_model_policy(mut self, model_policy: ModelExposurePolicy) -> Self {
        self.model_policy = model_policy;
        self
    }

    /// Assigns analyzer selection.
    pub fn with_analyzer(mut self, analyzer: AnalyzerSelection) -> Self {
        self.analyzer = analyzer;
        self
    }
}

/// Coarse repository artifact categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ArtifactCategory {
    /// Programming language source code.
    SourceCode,
    /// Runtime or application configuration.
    Configuration,
    /// Human-authored documentation.
    Documentation,
    /// Build command or build system definition.
    BuildDefinition,
    /// Package manifest.
    PackageManifest,
    /// Dependency lockfile.
    DependencyLockfile,
    /// Container build or run definition.
    ContainerDefinition,
    /// Deployment platform definition.
    DeploymentDefinition,
    /// CI/CD workflow definition.
    ContinuousIntegration,
    /// Database schema definition.
    DatabaseSchema,
    /// Database migration.
    DatabaseMigration,
    /// Executable script.
    Script,
    /// Template or markup entrypoint.
    Template,
    /// Generated source file.
    GeneratedSource,
    /// Test fixture or sample data.
    TestData,
    /// Static asset.
    StaticAsset,
    /// Binary asset.
    BinaryAsset,
    /// Safe text file that could not be classified more specifically.
    UnknownText,
    /// Opaque binary file that could not be classified more specifically.
    UnknownBinary,
}

/// Repository support tier for an artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SupportTier {
    /// AST or parser-backed language support.
    DeepLanguage,
    /// Parser-backed structured format support.
    StructuredFormat,
    /// Generic safe text extraction.
    GenericText,
    /// Metadata-only artifact.
    Opaque,
}

/// Text/binary handling status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextStatus {
    /// Inventory has not decided yet.
    Unknown,
    /// Safe UTF-8 text.
    Text,
    /// Binary content.
    Binary,
    /// Text-like content that must not be read or exposed.
    UnsafeText,
}

/// External model exposure policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelExposurePolicy {
    /// Content may be included in model context.
    Allowed,
    /// Only selected excerpts may be included.
    ExcerptOnly,
    /// Content may be included only after redaction.
    Redacted,
    /// Content must never be included.
    Never,
}

/// Analyzer selected for an artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalyzerSelection {
    /// No analyzer has been assigned yet.
    Unassigned,
    /// Specialized analyzer by stable name.
    Specialized(String),
    /// Structured format analyzer by stable name.
    Structured(String),
    /// Generic text fallback analyzer.
    GenericText,
    /// Metadata-only opaque analyzer.
    Opaque,
}

#[cfg(test)]
mod tests {
    use super::{
        AnalyzerSelection, Artifact, ArtifactCategory, ModelExposurePolicy, SupportTier, TextStatus,
    };
    use crate::domain::ids::{ContentHash, RepoPath};

    #[test]
    fn artifact_constructor_assigns_stable_path_id() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = Artifact::new(
            RepoPath::new("src/lib.rs")?,
            ArtifactCategory::SourceCode,
            SupportTier::DeepLanguage,
            ContentHash::new("abcdef")?,
            512,
        );

        assert_eq!(artifact.id.as_str(), "artifact:src/lib.rs");
        assert_eq!(artifact.path.as_str(), "src/lib.rs");
        assert_eq!(artifact.category, ArtifactCategory::SourceCode);
        assert_eq!(artifact.support_tier, SupportTier::DeepLanguage);
        assert_eq!(artifact.model_policy, ModelExposurePolicy::Allowed);

        Ok(())
    }

    #[test]
    fn artifact_builder_methods_set_optional_inventory_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = Artifact::new(
            RepoPath::new("generated/client.py")?,
            ArtifactCategory::GeneratedSource,
            SupportTier::GenericText,
            ContentHash::new("012345")?,
            256,
        )
        .with_detected_format("python")
        .with_text_status(TextStatus::Text, Some(12))
        .with_origin_scores(250, 30)
        .with_model_policy(ModelExposurePolicy::ExcerptOnly)
        .with_analyzer(AnalyzerSelection::GenericText);

        assert_eq!(artifact.detected_format.as_deref(), Some("python"));
        assert_eq!(artifact.text_status, TextStatus::Text);
        assert_eq!(artifact.line_count, Some(12));
        assert_eq!(artifact.generated_score, 100);
        assert_eq!(artifact.vendored_score, 30);
        assert_eq!(artifact.model_policy, ModelExposurePolicy::ExcerptOnly);
        assert_eq!(artifact.analyzer, AnalyzerSelection::GenericText);

        Ok(())
    }

    #[test]
    fn artifact_serializes_deterministically() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = Artifact::new(
            RepoPath::new("Dockerfile")?,
            ArtifactCategory::ContainerDefinition,
            SupportTier::StructuredFormat,
            ContentHash::new("f00d")?,
            1024,
        )
        .with_detected_format("dockerfile")
        .with_text_status(TextStatus::Text, Some(15))
        .with_analyzer(AnalyzerSelection::Structured("dockerfile".to_owned()));

        let json = serde_json::to_string_pretty(&artifact)?;
        let round_tripped: Artifact = serde_json::from_str(&json)?;

        assert_eq!(artifact, round_tripped);
        assert!(json.contains("\"category\": \"ContainerDefinition\""));
        assert!(json.contains("\"support_tier\": \"StructuredFormat\""));

        Ok(())
    }
}
