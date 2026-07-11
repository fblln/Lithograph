//! Vendored-content analysis gating for inventory artifacts.

use crate::inventory::Classification;
use crate::inventory::limits::make_opaque;

/// A `vendored_score` at or above this value is treated as a confident
/// vendor-directory detection (LIT-23.4). Only path-based conventions
/// (`vendor/`, `third_party/`, `third-party/`; see
/// `classify::apply_origin_scores`) currently produce a score at all, and
/// they always score exactly 100, so this is a strict on/off cutoff today,
/// not a fuzzy threshold -- future, less certain signals could still land
/// below it without becoming an accidental opacity trigger.
pub const VENDORED_ANALYSIS_THRESHOLD: u8 = 100;

/// Vendor-content policy for inventory artifacts.
#[derive(Debug, Clone, Copy, Default)]
pub struct VendorPolicy;

/// Vendor decision for a discovered artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VendorDecision {
    /// Whether `vendored_score` meets [`VENDORED_ANALYSIS_THRESHOLD`].
    pub vendored: bool,
}

impl VendorPolicy {
    /// Returns the vendor decision for an artifact scored `vendored_score`
    /// (`Classification::vendored_score`, already computed at
    /// classification time).
    pub fn decide(&self, vendored_score: u8) -> VendorDecision {
        VendorDecision {
            vendored: vendored_score >= VENDORED_ANALYSIS_THRESHOLD,
        }
    }

    /// Applies a vendor decision to a classifier result, mirroring
    /// [`SizePolicy::apply`](crate::inventory::SizePolicy::apply): a
    /// vendored artifact's path, category, and size stay visible -- it
    /// just skips content-based extraction, the same way an oversized file
    /// already does, since third-party source shouldn't be analyzed as if
    /// it were the repository's own code.
    pub fn apply(
        &self,
        classification: Classification,
        decision: VendorDecision,
    ) -> Classification {
        if decision.vendored {
            make_opaque(classification)
        } else {
            classification
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{VENDORED_ANALYSIS_THRESHOLD, VendorDecision, VendorPolicy};
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
    fn decide_flags_scores_at_or_above_the_threshold() {
        let policy = VendorPolicy;

        assert_eq!(
            policy.decide(VENDORED_ANALYSIS_THRESHOLD - 1),
            VendorDecision { vendored: false }
        );
        assert_eq!(
            policy.decide(VENDORED_ANALYSIS_THRESHOLD),
            VendorDecision { vendored: true }
        );
    }

    #[test]
    fn apply_forces_opaque_and_excerpt_only_when_vendored() {
        let policy = VendorPolicy;
        let input = classification(
            SupportTier::GenericText,
            AnalyzerSelection::GenericText,
            ModelExposurePolicy::Allowed,
        );

        let result = policy.apply(input, VendorDecision { vendored: true });

        assert_eq!(result.support_tier, SupportTier::Opaque);
        assert_eq!(result.analyzer, AnalyzerSelection::Opaque);
        assert_eq!(result.model_policy, ModelExposurePolicy::ExcerptOnly);
    }

    #[test]
    fn apply_is_a_no_op_when_not_vendored() {
        let policy = VendorPolicy;
        let input = classification(
            SupportTier::GenericText,
            AnalyzerSelection::GenericText,
            ModelExposurePolicy::Allowed,
        );

        let result = policy.apply(input.clone(), VendorDecision { vendored: false });

        assert_eq!(result, input);
    }

    #[test]
    fn apply_never_loosens_an_already_stricter_model_policy() {
        let policy = VendorPolicy;
        let input = classification(
            SupportTier::Opaque,
            AnalyzerSelection::Opaque,
            ModelExposurePolicy::Never,
        );

        let result = policy.apply(input, VendorDecision { vendored: true });

        assert_eq!(result.model_policy, ModelExposurePolicy::Never);
    }
}
