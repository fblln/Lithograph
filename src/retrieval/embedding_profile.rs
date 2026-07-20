//! Typed document/query embedding profiles (LIT-86.15).
//!
//! An embedding request carries an explicit [`EmbeddingPurpose`] -- `Document`
//! or `Query` -- so no caller can silently use one undifferentiated `embed`
//! path for both roles. The persisted [`DocumentProfile`] records the immutable
//! shared geometry (provider, model revision, dimensions, element type,
//! normalization, distance metric, tokenizer/truncation identity, vector schema
//! version) plus the document prompt; the runtime [`QueryProfile`] records the
//! same shared geometry plus the query prompt. Before search, the query profile
//! is checked against the persisted document profile: the query prompt may
//! differ (a query-only instruction change never rebuilds document vectors),
//! but any shared-geometry mismatch is a hard incompatibility naming the
//! differing field and a remediation, never a silently mixed-space score.

// ponytail: this is the canonical embedding profile; chunk_index's coarser
// ProviderIdentity adopts it as a small follow-on. Drop this allow then.
#![allow(dead_code)]

use crate::retrieval::semantic_search::{EmbeddingError, EmbeddingProvider};
use serde::{Deserialize, Serialize};

/// The role an embedding plays. Callers must pass one explicitly (AC#1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum EmbeddingPurpose {
    /// A stored document/corpus vector.
    Document,
    /// A transient query vector.
    Query,
}

/// The shared geometry both profiles must agree on for scores to be comparable
/// (AC#2/#3). Every field here participates in the fingerprint (AC#7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SharedProfile {
    /// Provider identity, e.g. `"mock"`.
    pub provider: String,
    /// Immutable model revision.
    pub model_revision: String,
    /// Vector dimensionality.
    pub dimensions: usize,
    /// Vector element type, e.g. `"f32"`.
    pub element_type: String,
    /// Whether vectors are normalized.
    pub normalized: bool,
    /// Distance metric, e.g. `"cosine"`.
    pub distance_metric: String,
    /// Tokenizer/truncation identity.
    pub tokenizer_identity: String,
    /// Vector storage schema version.
    pub vector_schema_version: u32,
}

/// The persisted profile for stored document vectors (AC#2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DocumentProfile {
    /// Shared geometry.
    pub shared: SharedProfile,
    /// Document embedding prompt identity.
    pub document_prompt: String,
}

/// The runtime profile for query vectors (AC#3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct QueryProfile {
    /// Shared geometry (must match the document profile).
    pub shared: SharedProfile,
    /// Query embedding prompt identity (may differ from the document prompt).
    pub query_prompt: String,
}

/// A hard incompatibility between a query and the persisted document profile
/// (AC#5/#9): the differing shared field and a remediation the surface can show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProfileMismatch {
    /// The shared field that differs.
    pub field: String,
    /// The persisted document profile's value.
    pub persisted: String,
    /// The current query profile's value.
    pub current: String,
    /// Suggested remediation command.
    pub remediation: String,
}

impl std::fmt::Display for ProfileMismatch {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "embedding profile mismatch on `{}`: index has `{}`, query has `{}` -- {}",
            self.field, self.persisted, self.current, self.remediation
        )
    }
}

impl std::error::Error for ProfileMismatch {}

impl SharedProfile {
    /// Returns the first differing shared field, or `None` when compatible.
    /// Field order is fixed so the "first" difference is deterministic.
    fn first_difference(&self, other: &Self) -> Option<(&'static str, String, String)> {
        let checks: [(&'static str, String, String); 8] = [
            ("provider", self.provider.clone(), other.provider.clone()),
            (
                "model_revision",
                self.model_revision.clone(),
                other.model_revision.clone(),
            ),
            (
                "dimensions",
                self.dimensions.to_string(),
                other.dimensions.to_string(),
            ),
            (
                "element_type",
                self.element_type.clone(),
                other.element_type.clone(),
            ),
            (
                "normalized",
                self.normalized.to_string(),
                other.normalized.to_string(),
            ),
            (
                "distance_metric",
                self.distance_metric.clone(),
                other.distance_metric.clone(),
            ),
            (
                "tokenizer_identity",
                self.tokenizer_identity.clone(),
                other.tokenizer_identity.clone(),
            ),
            (
                "vector_schema_version",
                self.vector_schema_version.to_string(),
                other.vector_schema_version.to_string(),
            ),
        ];
        checks
            .into_iter()
            .find(|(_, persisted, current)| persisted != current)
    }
}

impl DocumentProfile {
    /// Checks a runtime `query` profile against this persisted document profile
    /// (AC#3). The prompts are allowed to differ (AC#4); only a shared-geometry
    /// mismatch is fatal.
    pub(crate) fn check_query(&self, query: &QueryProfile) -> Result<(), ProfileMismatch> {
        match self.shared.first_difference(&query.shared) {
            None => Ok(()),
            Some((field, persisted, current)) => Err(ProfileMismatch {
                field: field.to_owned(),
                persisted,
                current,
                remediation: "rebuild the index with `lithograph search-code --refresh`".to_owned(),
            }),
        }
    }

    /// True when a change to `new` (a re-derived document profile) requires
    /// rebuilding stored document vectors (AC#5): any shared-geometry change or
    /// a document-prompt change. A query-prompt change never lands here.
    pub(crate) fn requires_rebuild(&self, new: &Self) -> bool {
        self.shared != new.shared || self.document_prompt != new.document_prompt
    }
}

/// A provider that embeds with an explicit purpose (AC#1). The blanket impl
/// below adapts any symmetric [`EmbeddingProvider`]; asymmetric providers
/// override this to apply purpose-specific prompts while still exposing both
/// profiles.
pub(crate) trait PurposeAwareEmbedder {
    /// Embeds `text` in the given `purpose`'s role.
    fn embed_purpose(
        &self,
        text: &str,
        purpose: EmbeddingPurpose,
    ) -> Result<Vec<f32>, EmbeddingError>;
}

/// Any symmetric [`EmbeddingProvider`] is trivially purpose-aware: both roles
/// map to one geometry (AC#6), but the purpose is still explicit at the call
/// site, so the information is never lost at the Lithograph boundary.
impl<T: EmbeddingProvider> PurposeAwareEmbedder for T {
    fn embed_purpose(
        &self,
        text: &str,
        _purpose: EmbeddingPurpose,
    ) -> Result<Vec<f32>, EmbeddingError> {
        self.embed(text)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DocumentProfile, EmbeddingPurpose, PurposeAwareEmbedder, QueryProfile, SharedProfile,
    };
    use crate::retrieval::semantic_search::MockEmbeddingProvider;

    fn shared() -> SharedProfile {
        SharedProfile {
            provider: "mock".to_owned(),
            model_revision: "mock-hash-v1".to_owned(),
            dimensions: 64,
            element_type: "f32".to_owned(),
            normalized: true,
            distance_metric: "cosine".to_owned(),
            tokenizer_identity: "identifier-aware-v1".to_owned(),
            vector_schema_version: 1,
        }
    }

    fn document() -> DocumentProfile {
        DocumentProfile {
            shared: shared(),
            document_prompt: "code-document-v1".to_owned(),
        }
    }

    fn query() -> QueryProfile {
        QueryProfile {
            shared: shared(),
            query_prompt: "code-query-v1".to_owned(),
        }
    }

    /// AC#3/#4: matching shared geometry is compatible even when the query
    /// prompt differs from the document prompt.
    #[test]
    fn compatible_when_only_prompts_differ() {
        assert!(document().check_query(&query()).is_ok());
    }

    /// AC#4: a query-only prompt change never requires a document rebuild.
    #[test]
    fn query_prompt_change_does_not_require_rebuild() {
        let mut other_query = query();
        other_query.query_prompt = "code-query-v2".to_owned();
        assert!(document().check_query(&other_query).is_ok());
        // The document profile is unchanged, so no rebuild.
        assert!(!document().requires_rebuild(&document()));
    }

    /// AC#5: a document-prompt change requires a rebuild.
    #[test]
    fn document_prompt_change_requires_rebuild() {
        let mut changed = document();
        changed.document_prompt = "code-document-v2".to_owned();
        assert!(document().requires_rebuild(&changed));
    }

    /// AC#5/#9: dimension and normalization mismatches are hard errors naming
    /// the differing field with a remediation.
    #[test]
    fn dimension_and_normalization_mismatch_are_errors() -> Result<(), Box<dyn std::error::Error>> {
        let mut bad_dims = query();
        bad_dims.shared.dimensions = 128;
        let Err(error) = document().check_query(&bad_dims) else {
            return Err("expected a dimension mismatch".into());
        };
        assert_eq!(error.field, "dimensions");
        assert!(error.remediation.contains("refresh"));

        let mut bad_norm = query();
        bad_norm.shared.normalized = false;
        let Err(norm_error) = document().check_query(&bad_norm) else {
            return Err("expected a normalization mismatch".into());
        };
        assert_eq!(norm_error.field, "normalized");
        Ok(())
    }

    /// AC#5: a shared model-revision change requires a document rebuild (the
    /// stored vectors were produced by the old revision).
    #[test]
    fn model_revision_change_requires_rebuild() {
        let mut changed = document();
        changed.shared.model_revision = "mock-hash-v2".to_owned();
        assert!(document().requires_rebuild(&changed));
    }

    /// AC#1/#8: the mock provider is purpose-aware with reproducible geometry;
    /// a symmetric provider maps both purposes to the same vector (AC#6).
    #[test]
    fn mock_is_purpose_aware_and_symmetric() -> Result<(), Box<dyn std::error::Error>> {
        let doc =
            MockEmbeddingProvider.embed_purpose("route service", EmbeddingPurpose::Document)?;
        let qry = MockEmbeddingProvider.embed_purpose("route service", EmbeddingPurpose::Query)?;
        assert_eq!(
            doc, qry,
            "symmetric provider: same geometry for both purposes"
        );
        // Reproducible.
        let again =
            MockEmbeddingProvider.embed_purpose("route service", EmbeddingPurpose::Document)?;
        assert_eq!(doc, again);
        Ok(())
    }
}
