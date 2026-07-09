//! Size-based analysis limits for inventory artifacts.

use crate::domain::{AnalyzerSelection, ModelExposurePolicy, SupportTier};
use crate::inventory::Classification;

/// Artifacts at or above this size are treated as opaque data blobs rather
/// than analyzed line-by-line. Generic-text and structured extraction
/// heuristics are tuned for hand-written source and config files; applied
/// across megabytes of machine-generated or telemetry data (GPX tracks,
/// lockfiles, minified bundles) they match incidentally import-like or
/// config-like substrings on nearly every line, producing a graph node
/// count that scales with file size rather than with actual code
/// structure. The artifact still gets an `Artifact` graph node -- its
/// path, category, and size stay visible -- it just skips content-based
/// extraction, exactly like a binary or an unsafe-path file.
pub const MAX_ANALYZABLE_BYTES: u64 = 1_000_000;

/// Size policy for inventory artifacts.
#[derive(Debug, Clone, Copy, Default)]
pub struct SizePolicy;

/// Size decision for a discovered artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeDecision {
    /// Whether the artifact exceeds [`MAX_ANALYZABLE_BYTES`].
    pub oversized: bool,
}

impl SizePolicy {
    /// Returns the size decision for an artifact of `size_bytes`.
    pub fn decide(&self, size_bytes: u64) -> SizeDecision {
        SizeDecision {
            oversized: size_bytes > MAX_ANALYZABLE_BYTES,
        }
    }

    /// Applies a size decision to a classifier result. Oversized artifacts
    /// become opaque for extraction purposes but keep (or gain)
    /// `ExcerptOnly` model exposure rather than `Never` -- unlike
    /// `SafetyPolicy`'s unsafe-path handling, a large file isn't secret, so
    /// a bounded excerpt may still be useful context. A classification
    /// already stricter than `Allowed` (e.g. `Never`, from `SafetyPolicy`)
    /// is left untouched.
    pub fn apply(&self, classification: Classification, decision: SizeDecision) -> Classification {
        if decision.oversized {
            make_opaque(classification)
        } else {
            classification
        }
    }
}

/// Forces a classifier result opaque for extraction purposes (no analyzer
/// runs over its content) while preserving an already-stricter model
/// exposure policy (e.g. `Never`, from `SafetyPolicy`) rather than loosening
/// it to `ExcerptOnly`. Shared by every "skip analysis, but this isn't
/// secret" policy -- [`SizePolicy`] and, mirroring it, `VendorPolicy`
/// (LIT-23.4, src/inventory/vendor.rs).
pub(crate) fn make_opaque(mut classification: Classification) -> Classification {
    classification.support_tier = SupportTier::Opaque;
    classification.analyzer = AnalyzerSelection::Opaque;
    if classification.model_policy == ModelExposurePolicy::Allowed {
        classification.model_policy = ModelExposurePolicy::ExcerptOnly;
    }
    classification
}

#[cfg(test)]
mod tests {
    use super::{MAX_ANALYZABLE_BYTES, SizeDecision, SizePolicy};
    use crate::domain::{AnalyzerSelection, ModelExposurePolicy, SupportTier};
    use crate::inventory::Classification;

    fn classification(
        support_tier: SupportTier,
        analyzer: AnalyzerSelection,
        model_policy: ModelExposurePolicy,
    ) -> Classification {
        Classification {
            category: crate::domain::ArtifactCategory::SourceCode,
            detected_format: Some("generic-text".to_owned()),
            support_tier,
            model_policy,
            analyzer,
            generated_score: 0,
            vendored_score: 0,
        }
    }

    #[test]
    fn decide_flags_files_over_the_threshold() {
        let policy = SizePolicy;

        assert_eq!(
            policy.decide(MAX_ANALYZABLE_BYTES),
            SizeDecision { oversized: false }
        );
        assert_eq!(
            policy.decide(MAX_ANALYZABLE_BYTES + 1),
            SizeDecision { oversized: true }
        );
    }

    #[test]
    fn apply_forces_opaque_and_excerpt_only_when_oversized() {
        let policy = SizePolicy;
        let input = classification(
            SupportTier::GenericText,
            AnalyzerSelection::GenericText,
            ModelExposurePolicy::Allowed,
        );

        let result = policy.apply(input, SizeDecision { oversized: true });

        assert_eq!(result.support_tier, SupportTier::Opaque);
        assert_eq!(result.analyzer, AnalyzerSelection::Opaque);
        assert_eq!(result.model_policy, ModelExposurePolicy::ExcerptOnly);
    }

    #[test]
    fn apply_is_a_no_op_when_not_oversized() {
        let policy = SizePolicy;
        let input = classification(
            SupportTier::GenericText,
            AnalyzerSelection::GenericText,
            ModelExposurePolicy::Allowed,
        );

        let result = policy.apply(input.clone(), SizeDecision { oversized: false });

        assert_eq!(result, input);
    }

    #[test]
    fn apply_never_loosens_an_already_stricter_model_policy() {
        let policy = SizePolicy;
        let input = classification(
            SupportTier::Opaque,
            AnalyzerSelection::Opaque,
            ModelExposurePolicy::Never,
        );

        let result = policy.apply(input, SizeDecision { oversized: true });

        assert_eq!(result.model_policy, ModelExposurePolicy::Never);
    }
}
