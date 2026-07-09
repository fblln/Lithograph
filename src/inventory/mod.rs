//! Repository inventory, walking, and classification.

pub mod classify;
pub mod language;
pub mod limits;
pub mod safety;
pub mod vendor;
pub mod walk;

pub use classify::{ArtifactClassifier, Classification, ClassificationInput};
pub use language::{
    LANGUAGE_REGISTRY, LANGUAGE_REGISTRY_VERSION, LanguageRegistryEntry, RegistryIndexTier,
    by_extension as language_by_extension, by_id as language_by_id, by_name as language_by_name,
};
pub use limits::{MAX_ANALYZABLE_BYTES, SizeDecision, SizePolicy};
pub use safety::{SafetyDecision, SafetyPolicy};
pub use vendor::{VENDORED_ANALYSIS_THRESHOLD, VendorDecision, VendorPolicy};
pub use walk::{RepositoryWalker, WalkError, WalkOptions};
