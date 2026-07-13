//! Versioned contracts shared by the baseline runner, CLI, and MCP server.

use crate::domain::ArtifactCategory;
use crate::graph::{CommunityScope, GraphBuildStageTrace, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Current lab artifact schema.
pub const LAB_SCHEMA_VERSION: u32 = 2;

/// Execution tier for a corpus case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SuiteTier {
    /// Fast, hermetic fixtures required on every change.
    Pr,
    /// Pinned medium repositories run on merge/main.
    Merge,
    /// Large repositories, mutations, and performance sampling.
    Nightly,
}

impl SuiteTier {
    /// Whether a case belongs in this tier or a broader one.
    pub fn includes(self, case: Self) -> bool {
        case <= self
    }

    /// Stable CLI/config name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pr => "pr",
            Self::Merge => "merge",
            Self::Nightly => "nightly",
        }
    }
}

/// Versioned corpus configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Configured cases.
    pub cases: Vec<CorpusCase>,
}

/// One immutable repository or authored fixture case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusCase {
    /// Stable case identifier.
    pub id: String,
    /// Suite tier.
    pub tier: SuiteTier,
    /// Local fixture or immutable Git source.
    #[serde(flatten)]
    pub source: CorpusSource,
    /// SPDX license identifier.
    pub license: String,
    /// Repository-relative expectation file.
    pub expectations: String,
    /// Additional ignore globs for this case.
    #[serde(default)]
    pub exclude: Vec<String>,
}

/// Source location for one corpus case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "lowercase")]
pub enum CorpusSource {
    /// Authored repository committed with Lithograph.
    Fixture {
        /// Repository-relative fixture path.
        path: String,
    },
    /// Public repository pinned to an immutable commit and Git tree.
    Git {
        /// Clone URL.
        url: String,
        /// Full commit object id.
        commit: String,
        /// Expected root tree object id.
        tree: String,
    },
}

/// Named mutation or differential scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scenario {
    /// Stable scenario identifier.
    pub id: String,
    /// Human-readable intent.
    pub description: String,
    /// Deterministic scenario operation.
    pub operation: ScenarioOperation,
    /// Whether all original identity-sensitive expectations remain applicable.
    #[serde(default = "default_true")]
    pub preserve_expectations: bool,
}

const fn default_true() -> bool {
    true
}

/// Supported deterministic source transformations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScenarioOperation {
    /// Add a comment to a selected text file.
    AppendComment {
        /// File to mutate.
        path: String,
        /// Comment text without a language-specific delimiter requirement.
        text: String,
    },
    /// Add an unrelated source file.
    AddFile {
        /// New repository-relative path.
        path: String,
        /// Complete file content.
        content: String,
    },
    /// Rename a file without changing its bytes.
    RenameFile {
        /// Existing path.
        from: String,
        /// Destination path.
        to: String,
    },
    /// Replace a symbol and all selected fixture references with a safe name.
    ReplaceText {
        /// File to edit.
        path: String,
        /// Exact text to replace.
        from: String,
        /// Replacement text.
        to: String,
    },
    /// Move a file while rewriting an import in another fixture file.
    MoveFileAndReplace {
        /// Existing file path.
        from: String,
        /// Destination file path.
        to: String,
        /// File containing the import/reference to update.
        update_path: String,
        /// Exact old reference.
        old: String,
        /// Exact new reference.
        new: String,
    },
    /// Insert deterministic content at the beginning of a file.
    PrependText {
        /// File to edit.
        path: String,
        /// Text to prepend.
        text: String,
    },
    /// Replace a complete small authored fixture file.
    RewriteFile {
        /// File to rewrite.
        path: String,
        /// Complete deterministic content.
        content: String,
    },
}

/// Review and coverage metadata for curated truth.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TruthPackMetadata {
    /// Human or process that reviewed the expectation set.
    #[serde(default)]
    pub reviewer: String,
    /// Immutable provenance note or upstream revision.
    #[serde(default)]
    pub provenance: String,
    /// Areas intentionally not labeled yet.
    #[serde(default)]
    pub unlabeled: Vec<String>,
}

/// Curated truth for one case.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectationSet {
    /// Schema version.
    pub schema_version: u32,
    /// Assertions applied to the unmodified repository.
    pub expectations: Vec<Expectation>,
    /// Optional metamorphic scenarios.
    #[serde(default)]
    pub scenarios: Vec<Scenario>,
    /// Expiring, exact signatures for already-tracked defects.
    #[serde(default)]
    pub known_failures: Vec<KnownFailure>,
    /// Review provenance and explicit oracle gaps.
    #[serde(default)]
    pub truth_pack: TruthPackMetadata,
}

/// One already-tracked failure that may temporarily satisfy a gate only when
/// its exact signature and issue count still match.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownFailure {
    /// Assertion whose failure is expected.
    pub assertion_id: String,
    /// Backlog tasks tracking the underlying defects.
    pub backlog: Vec<String>,
    /// Last accepted UTC date in `YYYY-MM-DD` form.
    pub expires: String,
    /// Total number of graph issues expected in the assertion detail.
    pub issue_count: usize,
    /// Required substrings and their exact occurrence counts.
    pub signatures: Vec<KnownFailureSignature>,
}

/// One exact component of a known-failure signature.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownFailureSignature {
    /// Required detail substring.
    pub contains: String,
    /// Exact number of occurrences.
    pub count: usize,
}

/// Accepted expected-failure metadata copied into a run artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExpectedFailureMatch {
    /// Backlog tasks tracking the defect.
    pub backlog: Vec<String>,
    /// Expiry date.
    pub expires: String,
}

/// One typed correctness oracle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Expectation {
    /// The final graph satisfies all built-in invariants.
    GraphValid {
        /// Stable expectation id.
        id: String,
    },
    /// A discovered artifact has the expected routing result.
    Artifact {
        /// Stable expectation id.
        id: String,
        /// Expected artifact path.
        path: String,
        /// Expected category.
        category: ArtifactCategory,
        /// Expected detected format.
        format: Option<String>,
    },
    /// A path must not appear in the inventory.
    ArtifactAbsent {
        /// Stable expectation id.
        id: String,
        /// Path that must be absent.
        path: String,
    },
    /// A matching relation is present or absent.
    Relation {
        /// Stable expectation id.
        id: String,
        /// Fragment matching the source node id.
        source_contains: String,
        /// Fragment matching the target node id.
        target_contains: String,
        /// Expected relation kind.
        relation: RelationKind,
        /// Whether the relation must be present.
        present: bool,
    },
    /// Two node-id fragments are or are not assigned to one community.
    CommunityPair {
        /// Stable expectation id.
        id: String,
        /// Fragment matching the first member.
        left_contains: String,
        /// Fragment matching the second member.
        right_contains: String,
        /// Whether both nodes must share a community.
        together: bool,
    },
    /// A SimilarTo relation is present or absent for a symbol pair.
    ClonePair {
        /// Stable expectation id.
        id: String,
        /// Fragment matching the first symbol.
        left_contains: String,
        /// Fragment matching the second symbol.
        right_contains: String,
        /// Whether the pair must be reported as similar.
        similar: bool,
    },
    /// A semantic query ranks a selected class within `max_rank`.
    SemanticRank {
        /// Stable expectation id.
        id: String,
        /// Local semantic query.
        query: String,
        /// Fragment matching the expected class node.
        node_contains: String,
        /// Highest accepted one-based rank.
        max_rank: usize,
    },
}

impl Expectation {
    /// Stable assertion id.
    pub fn id(&self) -> &str {
        match self {
            Self::GraphValid { id }
            | Self::Artifact { id, .. }
            | Self::ArtifactAbsent { id, .. }
            | Self::Relation { id, .. }
            | Self::CommunityPair { id, .. }
            | Self::ClonePair { id, .. }
            | Self::SemanticRank { id, .. } => id,
        }
    }
}

/// Outcome of one expectation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AssertionResult {
    /// Expectation id.
    pub id: String,
    /// Whether the oracle passed.
    pub passed: bool,
    /// First pipeline stage whose output is relevant to the result.
    pub stage: String,
    /// Actionable explanation containing expected and observed values.
    pub detail: String,
    /// Present only when a failed assertion exactly matched an unexpired
    /// known-failure signature.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_failure: Option<ExpectedFailureMatch>,
}

impl AssertionResult {
    /// Whether this assertion allows the suite to continue.
    pub fn is_accepted(&self) -> bool {
        self.passed || self.expected_failure.is_some()
    }
}

/// One normalized quality or performance metric.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricResult {
    /// Stable metric name.
    pub name: String,
    /// Observed value.
    pub value: f64,
    /// Optional lower correctness bound.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<f64>,
    /// Whether the configured bound passed.
    pub passed: bool,
}

/// Stable summary of one lab run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunManifest {
    /// Lab schema version.
    pub schema_version: u32,
    /// Graph pipeline semantic version used for this run.
    #[serde(default)]
    pub graph_pipeline_version: u32,
    /// Content-derived run id.
    pub run_id: String,
    /// Corpus case.
    pub case_id: String,
    /// Suite used for this run.
    pub suite: SuiteTier,
    /// Immutable source identity: fixture tree hash or Git commit.
    pub source_revision: String,
    /// Hash over the artifact inventory.
    pub inventory_hash: String,
    /// Final graph hash.
    pub graph_hash: String,
    /// Ordered graph-build trace.
    pub stages: Vec<GraphBuildStageTrace>,
    /// Curated assertion results.
    pub assertions: Vec<AssertionResult>,
    /// Normalized quality metrics.
    pub metrics: Vec<MetricResult>,
    /// Independent language-tool comparisons, including explicit skips.
    #[serde(default)]
    pub differentials: Vec<DifferentialResult>,
    /// Deterministic counts and timings useful for diagnostics. Wall-clock
    /// values are never part of `run_id` or correctness comparison.
    pub observations: BTreeMap<String, u64>,
    /// Exact local reproduction command.
    pub reproduce: String,
}

/// Self-contained reproduction metadata. Third-party source is deliberately
/// referenced by immutable identity rather than copied into this artifact.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReplayBundle {
    /// Schema version.
    pub schema_version: u32,
    /// Run being reproduced.
    pub run_id: String,
    /// Corpus case.
    pub case_id: String,
    /// Fixture path or immutable Git source.
    pub source: CorpusSource,
    /// Suite tier.
    pub suite: SuiteTier,
    /// Graph-pipeline semantics version.
    pub graph_pipeline_version: u32,
    /// Artifact inventory hash.
    pub inventory_hash: String,
    /// Final graph hash.
    pub graph_hash: String,
    /// Failed or expected-failure diagnostic slice.
    pub failures: Vec<AssertionResult>,
    /// Focused decision records automatically retained for failures.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_slice: Vec<crate::graph::GraphDecisionTrace>,
    /// Exact command to rerun the case.
    pub reproduce: String,
}

impl RunManifest {
    /// True when every deterministic assertion and bound passed.
    pub fn is_clean(&self) -> bool {
        self.assertions.iter().all(AssertionResult::is_accepted)
            && self.metrics.iter().all(|metric| metric.passed)
            && self
                .differentials
                .iter()
                .all(|result| result.status != DifferentialStatus::Failed)
    }
}

/// Reviewable committed baseline summary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineRecord {
    /// Schema version.
    pub schema_version: u32,
    /// Corpus case.
    pub case_id: String,
    /// Accepted source revision.
    pub source_revision: String,
    /// Accepted graph hash.
    pub graph_hash: String,
    /// Hashes after each graph pass.
    pub stage_hashes: BTreeMap<String, String>,
    /// Accepted assertion outcomes.
    pub assertions: BTreeMap<String, bool>,
    /// Human review reason.
    pub reason: String,
    /// Hash of the baseline replaced by this acceptance, when any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_graph_hash: Option<String>,
    /// Prior review reason retained for auditability.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_reason: Option<String>,
}

/// One semantic change between an accepted baseline and a candidate run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineChange {
    /// First affected stage or assertion id.
    pub key: String,
    /// Accepted value.
    pub baseline: String,
    /// Candidate value.
    pub candidate: String,
}

/// Complete semantic baseline comparison.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaselineDiff {
    /// Corpus case.
    pub case_id: String,
    /// First divergent stage, if any.
    pub first_divergent_stage: Option<String>,
    /// Stable ordered changes.
    pub changes: Vec<BaselineChange>,
}

/// Immutable review preview required before accepting a baseline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptanceReview {
    /// Candidate run.
    pub run_id: String,
    /// Human reason bound into the token.
    pub reason: String,
    /// Current semantic difference.
    pub diff: BaselineDiff,
    /// Confirmation token that becomes stale if the baseline, run, or reason changes.
    pub confirmation_token: String,
}

/// Explicit benchmark cache/update behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkMode {
    /// Remove the analysis cache before each measurement.
    Cold,
    /// Reuse a seeded analysis cache.
    WarmCache,
    /// Measure a rebuild after a deterministic no-op inventory refresh.
    Incremental,
    /// Measure a repeated build of an unchanged repository.
    NoOp,
    /// Replay only community phases from a verified persisted graph.
    CommunityOnly,
}

impl BenchmarkMode {
    /// Stable public name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cold => "cold",
            Self::WarmCache => "warm_cache",
            Self::Incremental => "incremental",
            Self::NoOp => "no_op",
            Self::CommunityOnly => "community_only",
        }
    }
}

/// One append-only machine-dependent performance observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceSample {
    /// Stable sample id.
    pub sample_id: String,
    /// Content-addressed correctness run used by the sample.
    pub run_id: String,
    /// Measurement mode.
    pub mode: BenchmarkMode,
    /// Zero-based sample sequence within the invocation.
    pub sequence: usize,
    /// Machine identity.
    pub machine: MachineFingerprint,
    /// Raw observations.
    pub observations: BTreeMap<String, u64>,
    /// Verified canonical graph hash for focused graph-replay samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_hash: Option<String>,
    /// Normalized community scope for focused graph-replay samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_scope: Option<CommunityScope>,
    /// Community algorithm version for focused graph-replay samples.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_algorithm_version: Option<u32>,
    /// Exact command that reproduces this benchmark invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduce: Option<String>,
}

/// Machine identity recorded with non-portable performance observations.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineFingerprint {
    /// Operating system.
    pub os: String,
    /// CPU architecture.
    pub architecture: String,
    /// Available hardware threads.
    pub parallelism: usize,
}

/// Robust summary of repeated samples for one observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RobustMetric {
    /// Observation name.
    pub name: String,
    /// Median value.
    pub median: f64,
    /// Median absolute deviation.
    pub mad: f64,
}

/// Machine-specific performance report kept separate from correctness baselines.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceSummary {
    /// Schema version.
    pub schema_version: u32,
    /// Corpus case.
    pub case_id: String,
    /// Immutable source revision.
    pub source_revision: String,
    /// Machine identity.
    pub machine: MachineFingerprint,
    /// Number of completed samples.
    pub samples: usize,
    /// Measurement mode shared by all samples.
    pub mode: BenchmarkMode,
    /// Relative paths of append-only raw observations.
    pub sample_files: Vec<String>,
    /// First append-only raw sample sequence in this invocation.
    #[serde(default)]
    pub history_sequence: usize,
    /// Advisory or gated comparison with the prior dedicated-runner history.
    #[serde(default)]
    pub regressions: Vec<PerformanceRegression>,
    /// Robust per-observation summaries.
    pub metrics: Vec<RobustMetric>,
    /// Verified canonical graph hash for focused graph-replay reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_hash: Option<String>,
    /// Normalized community scope for focused graph-replay reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_scope: Option<CommunityScope>,
    /// Community algorithm version for focused graph-replay reports.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub community_algorithm_version: Option<u32>,
    /// Exact command that reproduces this benchmark invocation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reproduce: Option<String>,
}

/// Reviewed relative threshold for one machine-dependent observation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceBudget {
    /// Corpus case.
    pub case_id: String,
    /// Benchmark mode.
    pub mode: BenchmarkMode,
    /// Observation name.
    pub metric: String,
    /// Largest permitted relative median increase, e.g. `0.15` for 15%.
    pub max_relative_increase: f64,
}

/// Versioned reviewed performance policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceBudgetManifest {
    /// Lab schema version.
    pub schema_version: u32,
    /// Reviewed relative budgets.
    pub budgets: Vec<PerformanceBudget>,
}

/// One comparison against prior dedicated-runner history.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PerformanceRegression {
    /// Observation name.
    pub metric: String,
    /// Previous median.
    pub previous_median: f64,
    /// Current median.
    pub current_median: f64,
    /// Relative increase.
    pub relative_increase: f64,
    /// Reviewed allowed increase.
    pub allowed_increase: f64,
    /// Whether this comparison remains within budget.
    pub passed: bool,
}

/// Outcome of an optional independent differential tool.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DifferentialResult {
    /// Stable oracle name.
    pub name: String,
    /// Whether the oracle ran.
    pub status: DifferentialStatus,
    /// Human-readable comparison or skip reason.
    pub detail: String,
}

/// Execution state of a differential oracle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DifferentialStatus {
    /// Tool ran and matched.
    Passed,
    /// Tool ran and disagreed.
    Failed,
    /// Tool was not installed or no compatible input was present.
    Skipped,
}

/// Mechanical schema migration preview/result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MigrationReport {
    /// Input artifact.
    pub path: String,
    /// Observed version.
    pub from_version: u32,
    /// Target version.
    pub to_version: u32,
    /// Whether the command wrote the migrated artifact.
    pub applied: bool,
    /// Ordered mechanical changes; never semantic acceptance.
    pub changes: Vec<String>,
}

/// Source-free minimized diagnostic slice for one failing run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MinimizedFailureBundle {
    /// Run being minimized.
    pub run_id: String,
    /// Corpus case configuration identity.
    pub case_id: String,
    /// Suite tier used by the failing run.
    pub suite: SuiteTier,
    /// Immutable source revision.
    pub source_revision: String,
    /// Exact hash of failing expectation ids and details.
    pub failure_signature: String,
    /// Failing assertions whose signature must be preserved.
    pub failures: Vec<AssertionResult>,
    /// Smallest evidence-path set discovered from failures and decisions.
    pub relevant_files: Vec<String>,
    /// Relevant node ids.
    pub relevant_nodes: Vec<String>,
    /// Relevant graph relations.
    pub relevant_relations: Vec<crate::graph::Relation>,
    /// Optional local-only materialization directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub materialized_at: Option<String>,
}

impl BaselineDiff {
    /// True when the candidate matches the accepted deterministic baseline.
    pub fn is_clean(&self) -> bool {
        self.changes.is_empty()
    }
}
