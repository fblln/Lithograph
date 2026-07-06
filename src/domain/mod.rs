//! Core domain types shared by Lithograph pipeline stages.

pub mod artifact;
pub mod confidence;
pub mod evidence;
pub mod ids;

pub use artifact::{
    AnalyzerSelection, Artifact, ArtifactCategory, ModelExposurePolicy, SupportTier, TextStatus,
};
pub use confidence::Confidence;
pub use evidence::{EvidenceRef, SourceSpan};
pub use ids::{ArtifactId, ContentHash, RepoPath};
