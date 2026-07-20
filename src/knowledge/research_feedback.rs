//! Deterministic answer-outcome feedback persisted beside research memory.

use crate::graph::Graph;
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// On-disk schema for recorded answer outcomes.
pub(crate) const ANSWER_RESULTS_SCHEMA_VERSION: u32 = 1;
/// On-disk schema for reflected lessons.
pub(crate) const RESEARCH_LESSONS_SCHEMA_VERSION: u32 = 1;

const SCORE_SCALE: i64 = 1_000_000;
const HALF_LIFE_SECONDS: u64 = 30 * 24 * 60 * 60;
const CORROBORATION_THRESHOLD: u32 = 2;

/// What happened after an answer was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AnswerOutcome {
    /// The answer and cited evidence helped.
    Useful,
    /// The cited evidence did not answer the question.
    DeadEnd,
    /// The answer was wrong and has replacement guidance.
    Corrected,
}

impl FromStr for AnswerOutcome {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "useful" => Ok(Self::Useful),
            "dead_end" => Ok(Self::DeadEnd),
            "corrected" => Ok(Self::Corrected),
            other => Err(format!(
                "invalid outcome `{other}`; expected useful, dead_end, or corrected"
            )),
        }
    }
}

/// Input accepted by CLI and MCP save-result surfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct AnswerResultInput {
    /// Question that was answered.
    pub question: String,
    /// Answer whose outcome was observed.
    pub answer: String,
    /// Graph node ids cited by the answer.
    pub cited_node_ids: Vec<String>,
    /// Observed outcome.
    pub outcome: AnswerOutcome,
    /// Replacement guidance for a corrected answer.
    pub correction: Option<String>,
    /// Unix timestamp in seconds.
    pub recorded_at: u64,
}

/// One immutable, content-addressed answer outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AnswerResult {
    /// Stable content-derived record id.
    pub id: String,
    /// Question that was answered.
    pub question: String,
    /// Answer whose outcome was observed.
    pub answer: String,
    /// Sorted, deduplicated graph node citations.
    pub cited_node_ids: Vec<String>,
    /// Observed outcome.
    pub outcome: AnswerOutcome,
    /// Replacement guidance for a corrected answer.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correction: Option<String>,
    /// Unix timestamp in seconds.
    pub recorded_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AnswerResultIndex {
    schema_version: u32,
    results: Vec<AnswerResult>,
}

impl Default for AnswerResultIndex {
    fn default() -> Self {
        Self {
            schema_version: ANSWER_RESULTS_SCHEMA_VERSION,
            results: Vec::new(),
        }
    }
}

/// Aggregated evidence for one still-present graph node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SourceLesson {
    /// Stable graph node id.
    pub node_id: String,
    /// Signed fixed-point score, where 1,000,000 is one fresh useful signal.
    pub score_millionths: i64,
    /// Number of useful observations, independent of decay.
    pub useful_signals: u32,
    /// Number of dead-end observations, independent of decay.
    pub dead_end_signals: u32,
    /// Number of correction observations, independent of decay.
    pub correction_signals: u32,
}

/// Replacement guidance retained from a corrected answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CorrectionLesson {
    /// Answer-result record that supplied this correction.
    pub result_id: String,
    /// Still-present graph node the correction concerns.
    pub node_id: String,
    /// Replacement guidance.
    pub correction: String,
    /// Original observation time in Unix seconds.
    pub recorded_at: u64,
}

/// Versioned deterministic reflection output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ResearchLessons {
    /// Lessons artifact schema version.
    pub schema_version: u32,
    /// Caller-supplied time used for every decay calculation.
    pub reflected_at: u64,
    /// Positively scored sources with two or more useful observations.
    pub preferred_sources: Vec<SourceLesson>,
    /// Positively scored sources with only one useful observation.
    pub tentative_sources: Vec<SourceLesson>,
    /// Sources that have both positive and negative observations.
    pub contested_sources: Vec<SourceLesson>,
    /// Sources with negative observations and no useful observation.
    pub known_dead_ends: Vec<SourceLesson>,
    /// Explicit replacement guidance for still-present graph nodes.
    pub corrections: Vec<CorrectionLesson>,
}

#[derive(Debug, Clone, Copy, Default)]
struct Aggregate {
    score: i64,
    useful: u32,
    dead_end: u32,
    corrected: u32,
}

/// Repository-local feedback store under `.lithograph/research`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResearchFeedbackStore {
    research_dir: PathBuf,
}

impl ResearchFeedbackStore {
    /// Creates a store rooted under one repository's research directory.
    pub(crate) fn new(repo_root: &Path) -> Self {
        Self {
            research_dir: repo_root.join(".lithograph/research"),
        }
    }

    /// Returns the versioned answer-result index path.
    pub(crate) fn results_path(&self) -> PathBuf {
        self.research_dir.join("answer-results.json")
    }

    /// Returns the reflected lessons path.
    pub(crate) fn lessons_path(&self) -> PathBuf {
        self.research_dir.join("lessons.json")
    }

    /// Validates and records an outcome. Identical content is idempotent.
    pub(crate) fn save_result(&self, input: AnswerResultInput) -> io::Result<AnswerResult> {
        let question = required_text("question", input.question)?;
        let answer = required_text("answer", input.answer)?;
        let correction = input.correction.map(|value| value.trim().to_owned());
        if input.outcome == AnswerOutcome::Corrected
            && correction.as_deref().is_none_or(str::is_empty)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "corrected outcomes require non-empty correction text",
            ));
        }
        if input.outcome != AnswerOutcome::Corrected && correction.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "correction text is only valid for corrected outcomes",
            ));
        }

        let cited_node_ids = input
            .cited_node_ids
            .into_iter()
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        let identity = serde_json::to_vec(&(
            &question,
            &answer,
            &cited_node_ids,
            input.outcome,
            &correction,
            input.recorded_at,
        ))
        .map_err(io::Error::other)?;
        let result = AnswerResult {
            id: format!("result-{}", blake3::hash(&identity).to_hex()),
            question,
            answer,
            cited_node_ids,
            outcome: input.outcome,
            correction,
            recorded_at: input.recorded_at,
        };
        let mut index: AnswerResultIndex =
            JsonStore.read(&self.results_path())?.unwrap_or_default();
        validate_results_schema(index.schema_version)?;
        if !index
            .results
            .iter()
            .any(|existing| existing.id == result.id)
        {
            index.results.push(result.clone());
            index.results.sort_by(|left, right| {
                left.recorded_at
                    .cmp(&right.recorded_at)
                    .then_with(|| left.id.cmp(&right.id))
            });
            JsonStore.write(&self.results_path(), &index)?;
        }
        Ok(result)
    }

    /// Reflects saved outcomes, dropping citations absent from `graph`.
    pub(crate) fn reflect(&self, graph: &Graph, now: u64) -> io::Result<ResearchLessons> {
        let index: AnswerResultIndex = JsonStore.read(&self.results_path())?.unwrap_or_default();
        validate_results_schema(index.schema_version)?;
        let valid_nodes = graph
            .nodes
            .iter()
            .map(|node| node.id().as_str().to_owned())
            .collect::<BTreeSet<_>>();
        let lessons = reflect_results(&index.results, &valid_nodes, now);
        JsonStore.write_if_changed(&self.lessons_path(), &lessons)?;
        Ok(lessons)
    }

    /// Reads the most recently reflected lessons.
    pub(crate) fn read_lessons(&self) -> io::Result<ResearchLessons> {
        let lessons: ResearchLessons = JsonStore.read(&self.lessons_path())?.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no reflected lessons found at {}",
                    self.lessons_path().display()
                ),
            )
        })?;
        if lessons.schema_version != RESEARCH_LESSONS_SCHEMA_VERSION {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "unsupported research lessons schema {}",
                    lessons.schema_version
                ),
            ));
        }
        Ok(lessons)
    }
}

fn required_text(name: &str, value: String) -> io::Result<String> {
    let value = value.trim().to_owned();
    if value.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{name} must not be empty"),
        ))
    } else {
        Ok(value)
    }
}

fn validate_results_schema(version: u32) -> io::Result<()> {
    if version == ANSWER_RESULTS_SCHEMA_VERSION {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported answer-results schema {version}"),
        ))
    }
}

fn decayed_weight(recorded_at: u64, now: u64) -> i64 {
    let age = now.saturating_sub(recorded_at);
    let denominator = u128::from(HALF_LIFE_SECONDS) + u128::from(age);
    ((SCORE_SCALE as u128 * u128::from(HALF_LIFE_SECONDS)) / denominator) as i64
}

fn reflect_results(
    results: &[AnswerResult],
    valid_nodes: &BTreeSet<String>,
    now: u64,
) -> ResearchLessons {
    let mut aggregates = BTreeMap::<String, Aggregate>::new();
    let mut corrections = Vec::new();
    for result in results {
        let weight = decayed_weight(result.recorded_at, now);
        for node_id in result
            .cited_node_ids
            .iter()
            .filter(|node_id| valid_nodes.contains(*node_id))
        {
            let aggregate = aggregates.entry(node_id.clone()).or_default();
            match result.outcome {
                AnswerOutcome::Useful => {
                    aggregate.score += weight;
                    aggregate.useful += 1;
                }
                AnswerOutcome::DeadEnd => {
                    aggregate.score -= weight;
                    aggregate.dead_end += 1;
                }
                AnswerOutcome::Corrected => {
                    aggregate.score -= weight;
                    aggregate.corrected += 1;
                    corrections.push(CorrectionLesson {
                        result_id: result.id.clone(),
                        node_id: node_id.clone(),
                        correction: result.correction.clone().unwrap_or_default(),
                        recorded_at: result.recorded_at,
                    });
                }
            }
        }
    }

    let mut preferred_sources = Vec::new();
    let mut tentative_sources = Vec::new();
    let mut contested_sources = Vec::new();
    let mut known_dead_ends = Vec::new();
    for (node_id, aggregate) in aggregates {
        let lesson = SourceLesson {
            node_id,
            score_millionths: aggregate.score,
            useful_signals: aggregate.useful,
            dead_end_signals: aggregate.dead_end,
            correction_signals: aggregate.corrected,
        };
        let negative = aggregate.dead_end + aggregate.corrected;
        if aggregate.useful > 0 && negative > 0 {
            contested_sources.push(lesson);
        } else if aggregate.useful >= CORROBORATION_THRESHOLD && aggregate.score > 0 {
            preferred_sources.push(lesson);
        } else if aggregate.useful > 0 && aggregate.score > 0 {
            tentative_sources.push(lesson);
        } else {
            known_dead_ends.push(lesson);
        }
    }
    corrections.sort_by(|left, right| {
        left.node_id
            .cmp(&right.node_id)
            .then_with(|| left.recorded_at.cmp(&right.recorded_at))
            .then_with(|| left.result_id.cmp(&right.result_id))
    });

    ResearchLessons {
        schema_version: RESEARCH_LESSONS_SCHEMA_VERSION,
        reflected_at: now,
        preferred_sources,
        tentative_sources,
        contested_sources,
        known_dead_ends,
        corrections,
    }
}

/// Current Unix timestamp in seconds for interactive CLI/MCP defaults.
pub(crate) fn unix_timestamp_now() -> io::Result<u64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::{
        AnswerOutcome, AnswerResult, AnswerResultIndex, AnswerResultInput, ResearchFeedbackStore,
        reflect_results,
    };
    use std::collections::BTreeSet;

    fn result(id: &str, node: &str, outcome: AnswerOutcome, recorded_at: u64) -> AnswerResult {
        AnswerResult {
            id: id.to_owned(),
            question: "q".to_owned(),
            answer: "a".to_owned(),
            cited_node_ids: vec![node.to_owned()],
            outcome,
            correction: (outcome == AnswerOutcome::Corrected).then(|| "replacement".to_owned()),
            recorded_at,
        }
    }

    #[test]
    fn one_signal_is_tentative_and_two_promote_a_source() {
        let nodes = BTreeSet::from(["node".to_owned()]);
        let one = reflect_results(
            &[result("1", "node", AnswerOutcome::Useful, 100)],
            &nodes,
            100,
        );
        assert_eq!(one.tentative_sources.len(), 1);
        assert!(one.preferred_sources.is_empty());

        let two = reflect_results(
            &[
                result("1", "node", AnswerOutcome::Useful, 100),
                result("2", "node", AnswerOutcome::Useful, 90),
            ],
            &nodes,
            100,
        );
        assert_eq!(two.preferred_sources.len(), 1);
        assert!(two.tentative_sources.is_empty());
    }

    #[test]
    fn fresh_dead_end_outweighs_stale_useful_and_conflict_is_contested() {
        let nodes = BTreeSet::from(["node".to_owned()]);
        let lessons = reflect_results(
            &[
                result("old", "node", AnswerOutcome::Useful, 0),
                result(
                    "fresh",
                    "node",
                    AnswerOutcome::DeadEnd,
                    super::HALF_LIFE_SECONDS * 2,
                ),
            ],
            &nodes,
            super::HALF_LIFE_SECONDS * 2,
        );
        assert_eq!(lessons.contested_sources.len(), 1);
        assert!(lessons.contested_sources[0].score_millionths < 0);
    }

    #[test]
    fn dangling_nodes_and_their_corrections_are_dropped() {
        let lessons = reflect_results(
            &[result("1", "missing", AnswerOutcome::Corrected, 100)],
            &BTreeSet::new(),
            100,
        );
        assert!(lessons.known_dead_ends.is_empty());
        assert!(lessons.corrections.is_empty());
    }

    #[test]
    fn corrections_are_negative_lessons_with_replacement_text() {
        let nodes = BTreeSet::from(["node".to_owned()]);
        let lessons = reflect_results(
            &[result("1", "node", AnswerOutcome::Corrected, 100)],
            &nodes,
            100,
        );
        assert_eq!(lessons.known_dead_ends.len(), 1);
        assert_eq!(lessons.corrections[0].correction, "replacement");
        assert_eq!(lessons.known_dead_ends[0].score_millionths, -1_000_000);
    }

    #[test]
    fn store_normalizes_citations_and_deduplicates_identical_saves()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = ResearchFeedbackStore::new(temp.path());
        let input = || AnswerResultInput {
            question: " question ".to_owned(),
            answer: " answer ".to_owned(),
            cited_node_ids: vec!["b".to_owned(), "a".to_owned(), "a".to_owned()],
            outcome: AnswerOutcome::Useful,
            correction: None,
            recorded_at: 100,
        };
        let first = store.save_result(input())?;
        let second = store.save_result(input())?;
        let index: AnswerResultIndex = crate::storage::JsonStore
            .read(&store.results_path())?
            .ok_or("missing results")?;
        assert_eq!(first, second);
        assert_eq!(first.cited_node_ids, ["a", "b"]);
        assert_eq!(index.results.len(), 1);
        Ok(())
    }

    #[test]
    fn corrected_save_requires_correction_text() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let saved = ResearchFeedbackStore::new(temp.path()).save_result(AnswerResultInput {
            question: "q".to_owned(),
            answer: "a".to_owned(),
            cited_node_ids: vec![],
            outcome: AnswerOutcome::Corrected,
            correction: None,
            recorded_at: 100,
        });
        match saved {
            Ok(_) => Err("missing correction unexpectedly succeeded".into()),
            Err(error) => {
                assert!(error.to_string().contains("require non-empty correction"));
                Ok(())
            }
        }
    }

    #[test]
    fn ordering_and_serialized_bytes_are_stable() -> Result<(), Box<dyn std::error::Error>> {
        let nodes = BTreeSet::from(["a".to_owned(), "b".to_owned()]);
        let left = reflect_results(
            &[
                result("2", "b", AnswerOutcome::Useful, 100),
                result("1", "a", AnswerOutcome::Useful, 100),
            ],
            &nodes,
            200,
        );
        let right = reflect_results(
            &[
                result("1", "a", AnswerOutcome::Useful, 100),
                result("2", "b", AnswerOutcome::Useful, 100),
            ],
            &nodes,
            200,
        );
        assert_eq!(
            serde_json::to_vec_pretty(&left)?,
            serde_json::to_vec_pretty(&right)?
        );
        assert_eq!(left.tentative_sources[0].node_id, "a");
        Ok(())
    }
}
