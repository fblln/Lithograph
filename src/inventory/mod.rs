//! Repository inventory, walking, and classification.

pub mod classify;
pub mod safety;
pub mod walk;

pub use classify::{ArtifactClassifier, Classification, ClassificationInput};
pub use safety::{SafetyDecision, SafetyPolicy};
pub use walk::{RepositoryWalker, WalkError, WalkOptions};
