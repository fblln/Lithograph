//! Evidence validation: a generated page may only cite artifacts, spans,
//! and config paths that were actually present in its model context.

use crate::generation::context::{ContextExcerpt, ModelContext};
use crate::generation::llm::PageGeneration;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};

/// One evidence reference that failed validation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum EvidenceIssue {
    /// The reference's artifact path was not included as an excerpt in the context.
    UnknownArtifact {
        /// Raw evidence reference string as the model wrote it.
        reference: String,
    },
    /// The reference's line span extends past what was actually shown.
    SpanOutOfRange {
        /// Raw evidence reference string as the model wrote it.
        reference: String,
        /// Last line requested by the reference.
        requested_end: u32,
        /// Lines actually available in the excerpt.
        available_lines: usize,
    },
    /// The reference has a `#fragment` in a shape this validator does not
    /// recognize (only `#L<start>-L<end>` line spans are understood).
    MalformedReference {
        /// Raw evidence reference string as the model wrote it.
        reference: String,
    },
}

impl Display for EvidenceIssue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownArtifact { reference } => {
                write!(
                    formatter,
                    "evidence `{reference}` was not part of the model context"
                )
            }
            Self::SpanOutOfRange {
                reference,
                requested_end,
                available_lines,
            } => write!(
                formatter,
                "evidence `{reference}` cites line {requested_end} but only {available_lines} line(s) were shown"
            ),
            Self::MalformedReference { reference } => {
                write!(
                    formatter,
                    "evidence `{reference}` has an unrecognized `#fragment`"
                )
            }
        }
    }
}

/// Validates [`PageGeneration::evidence_refs`] against the [`ModelContext`]
/// that produced them.
#[derive(Debug, Clone, Copy, Default)]
pub struct EvidenceValidator;

impl EvidenceValidator {
    /// Returns every evidence issue found; an empty result means the page's
    /// evidence is entirely backed by the context it was generated from.
    pub fn validate(&self, page: &PageGeneration, context: &ModelContext) -> Vec<EvidenceIssue> {
        let excerpts: BTreeMap<&str, &ContextExcerpt> = context
            .excerpts
            .iter()
            .map(|excerpt| (excerpt.artifact_path.as_str(), excerpt))
            .collect();

        page.evidence_refs
            .iter()
            .filter_map(|reference| validate_one(reference, &excerpts))
            .collect()
    }

    /// Keeps only references that pass validation against `context`.
    pub fn valid_references(&self, page: &PageGeneration, context: &ModelContext) -> Vec<String> {
        let excerpts: BTreeMap<&str, &ContextExcerpt> = context
            .excerpts
            .iter()
            .map(|excerpt| (excerpt.artifact_path.as_str(), excerpt))
            .collect();

        page.evidence_refs
            .iter()
            .filter(|reference| validate_one(reference, &excerpts).is_none())
            .cloned()
            .collect()
    }
}

fn validate_one(
    reference: &str,
    excerpts: &BTreeMap<&str, &ContextExcerpt>,
) -> Option<EvidenceIssue> {
    let (path, fragment) = reference
        .split_once('#')
        .map_or((reference, None), |(path, fragment)| (path, Some(fragment)));

    let Some(excerpt) = excerpts.get(path) else {
        return Some(EvidenceIssue::UnknownArtifact {
            reference: reference.to_owned(),
        });
    };

    let fragment = fragment?;
    let Some((start, end)) = parse_line_span(fragment) else {
        return Some(EvidenceIssue::MalformedReference {
            reference: reference.to_owned(),
        });
    };
    if start == 0 || end < start || end as usize > excerpt.included_lines {
        return Some(EvidenceIssue::SpanOutOfRange {
            reference: reference.to_owned(),
            requested_end: end,
            available_lines: excerpt.included_lines,
        });
    }
    None
}

fn parse_line_span(fragment: &str) -> Option<(u32, u32)> {
    let rest = fragment.strip_prefix('L')?;
    let (start, end) = rest.split_once("-L")?;
    Some((start.parse().ok()?, end.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::{EvidenceIssue, EvidenceValidator};
    use crate::domain::ModelExposurePolicy;
    use crate::generation::context::{ContextExcerpt, ModelContext};
    use crate::generation::llm::PageGeneration;
    use crate::manifest::TaskKind;

    fn context() -> ModelContext {
        ModelContext {
            system_prompt: "system".to_owned(),
            user_prompt: "user".to_owned(),
            excerpts: vec![ContextExcerpt {
                artifact_path: "src/lib.rs".to_owned(),
                policy: ModelExposurePolicy::Allowed,
                included_lines: 10,
                truncated: false,
            }],
            input_hash: "hash".to_owned(),
            task_kind: TaskKind::ModulePage,
        }
    }

    fn page(evidence_refs: Vec<&str>) -> PageGeneration {
        PageGeneration {
            title: "T".to_owned(),
            summary: "S".to_owned(),
            evidence_refs: evidence_refs.into_iter().map(str::to_owned).collect(),
            unresolved_questions: Vec::new(),
            body: "# T\n".to_owned(),
        }
    }

    #[test]
    fn accepts_known_artifact_and_in_range_span() {
        let issues =
            EvidenceValidator.validate(&page(vec!["src/lib.rs", "src/lib.rs#L1-L10"]), &context());

        assert!(issues.is_empty());
    }

    #[test]
    fn rejects_nonexistent_artifact_path() {
        let issues = EvidenceValidator.validate(&page(vec!["src/not_in_context.rs"]), &context());

        assert_eq!(issues.len(), 1);
        assert!(
            matches!(&issues[0], EvidenceIssue::UnknownArtifact { reference } if reference == "src/not_in_context.rs")
        );
    }

    #[test]
    fn rejects_span_past_the_shown_lines() {
        let issues = EvidenceValidator.validate(&page(vec!["src/lib.rs#L1-L100"]), &context());

        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            EvidenceIssue::SpanOutOfRange {
                requested_end: 100,
                available_lines: 10,
                ..
            }
        ));
    }

    #[test]
    fn rejects_unsupported_fragment_shapes() {
        let issues =
            EvidenceValidator.validate(&page(vec!["src/lib.rs#service.image"]), &context());

        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            EvidenceIssue::MalformedReference { .. }
        ));
    }

    #[test]
    fn rejects_zero_or_inverted_spans() {
        let issues = EvidenceValidator.validate(
            &page(vec!["src/lib.rs#L0-L5", "src/lib.rs#L5-L1"]),
            &context(),
        );

        assert_eq!(issues.len(), 2);
    }

    #[test]
    fn valid_references_filters_invalid_entries() {
        let valid = EvidenceValidator.valid_references(
            &page(vec![
                "src/lib.rs",
                "src/lib.rs#L1-L3",
                "src/not_in_context.rs",
                "src/lib.rs#L1-L100",
            ]),
            &context(),
        );

        assert_eq!(
            valid,
            vec!["src/lib.rs".to_owned(), "src/lib.rs#L1-L3".to_owned()]
        );
    }
}
