//! Run metadata: what changed this run, and the content hashes needed to
//! detect no-op runs.

use crate::analysis::ANALYSIS_CACHE_VERSION;
use crate::domain::Artifact;
use crate::graph::{
    GRAPH_BUILD_PIPELINE_VERSION, GRAPH_MODEL_VERSION, GRAPH_STORE_SCHEMA_VERSION, Graph,
};
use crate::inventory::LANGUAGE_REGISTRY_VERSION;
use crate::manifest::PageManifest;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Persisted artifact content-hash snapshot, used to detect changed
/// artifacts across runs without re-diffing full file content.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct RepositorySnapshot {
    /// Artifact path to content hash.
    pub artifact_hashes: BTreeMap<String, String>,
    /// Pipeline inputs that affect graph and documentation invalidation.
    #[serde(default)]
    pub pipeline: PipelineInvalidationMetadata,
}

/// Versioned pipeline facts that decide whether cached graph/research/page
/// inputs are still compatible with the current binary and run options.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PipelineInvalidationMetadata {
    /// Analyzer/cache semantics version.
    pub analyzer_version: u32,
    /// Language registry routing/tier version.
    pub language_registry_version: u32,
    /// Graph store envelope schema version.
    pub graph_schema_version: u32,
    /// Graph model shape version.
    pub graph_model_version: u32,
    /// Ordered graph-construction pass semantics version.
    pub graph_pipeline_version: u32,
    /// Prompt/context version requested for this run.
    pub prompt_version: String,
    /// Semantic grouping setting used for planning.
    pub semantic_grouping: bool,
    /// Whether conventional test artifacts were included in the scan.
    pub include_tests: bool,
}

impl Default for PipelineInvalidationMetadata {
    fn default() -> Self {
        Self {
            analyzer_version: ANALYSIS_CACHE_VERSION,
            language_registry_version: LANGUAGE_REGISTRY_VERSION,
            graph_schema_version: GRAPH_STORE_SCHEMA_VERSION,
            graph_model_version: GRAPH_MODEL_VERSION,
            graph_pipeline_version: GRAPH_BUILD_PIPELINE_VERSION,
            prompt_version: String::new(),
            semantic_grouping: false,
            include_tests: false,
        }
    }
}

impl PipelineInvalidationMetadata {
    /// Builds current metadata for a run.
    pub(crate) fn current(
        prompt_version: &str,
        semantic_grouping: bool,
        include_tests: bool,
    ) -> Self {
        Self {
            prompt_version: prompt_version.to_owned(),
            semantic_grouping,
            include_tests,
            ..Self::default()
        }
    }
}

impl RepositorySnapshot {
    /// Builds a snapshot from the current artifact set.
    pub(crate) fn from_artifacts(
        artifacts: &[Artifact],
        pipeline: PipelineInvalidationMetadata,
    ) -> Self {
        Self {
            artifact_hashes: artifacts
                .iter()
                .map(|artifact| {
                    (
                        artifact.path.as_str().to_owned(),
                        artifact.content_hash.as_str().to_owned(),
                    )
                })
                .collect(),
            pipeline,
        }
    }

    /// Returns artifact paths that are new, removed, or changed relative to
    /// `previous` (the prior run's snapshot). Every artifact is "changed"
    /// when there is no previous snapshot (first run).
    pub(crate) fn changed_since(&self, previous: Option<&RepositorySnapshot>) -> Vec<String> {
        let Some(previous) = previous else {
            return self.artifact_hashes.keys().cloned().collect();
        };
        if self.pipeline != previous.pipeline {
            return self.artifact_hashes.keys().cloned().collect();
        }

        let mut changed: BTreeSet<String> = BTreeSet::new();
        for (path, hash) in &self.artifact_hashes {
            if previous.artifact_hashes.get(path) != Some(hash) {
                changed.insert(path.clone());
            }
        }
        for path in previous.artifact_hashes.keys() {
            if !self.artifact_hashes.contains_key(path) {
                changed.insert(path.clone());
            }
        }
        changed.into_iter().collect()
    }

    /// Deterministic hash over the whole snapshot.
    pub(crate) fn hash(&self) -> String {
        let mut pairs: Vec<String> = self
            .artifact_hashes
            .iter()
            .map(|(path, hash)| format!("{path}:{hash}"))
            .collect();
        pairs.push(format!(
            "pipeline:analyzer={}:language_registry={}:graph_schema={}:graph_model={}:graph_pipeline={}:prompt={}:semantic_grouping={}:include_tests={}",
            self.pipeline.analyzer_version,
            self.pipeline.language_registry_version,
            self.pipeline.graph_schema_version,
            self.pipeline.graph_model_version,
            self.pipeline.graph_pipeline_version,
            self.pipeline.prompt_version,
            self.pipeline.semantic_grouping,
            self.pipeline.include_tests,
        ));
        pairs.sort_unstable();
        blake3::hash(pairs.join("\n").as_bytes())
            .to_hex()
            .to_string()
    }
}

/// One explicit pipeline stage (LIT-22.6.1): `init`/`update` run through
/// these four in order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PipelineStage {
    /// Repository scan, artifact classification, graph build/validate, and
    /// documentation module planning.
    PreprocessIndex,
    /// Deterministic research fact extraction over the built graph.
    Research,
    /// Context building, model generation, and page rendering.
    Compose,
    /// Run-metadata computation and writing graph/manifest/snapshot/run output.
    ValidateOutput,
}

/// Wall-clock duration of one completed [`PipelineStage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct StageTiming {
    /// Which stage this measures.
    pub stage: PipelineStage,
    /// Elapsed wall-clock time in milliseconds.
    pub duration_ms: u64,
}

/// Metadata recorded for one `init`/`update` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct RunMetadata {
    /// Unique identifier for this run.
    pub run_id: String,
    /// CLI command that produced this run, e.g. `init`.
    pub command: String,
    /// Repository git HEAD commit, when the repository is a git checkout.
    pub git_head: Option<String>,
    /// Per-stage timing for this run (LIT-22.6.1 AC1). Empty when a caller
    /// builds `RunMetadata` directly rather than through `orchestrate`'s
    /// pipeline (e.g. existing unit tests) -- set by the pipeline after
    /// `compute()` returns, since the final stage's own duration isn't
    /// known until immediately before this metadata is written to disk.
    #[serde(default)]
    pub stage_timings: Vec<StageTiming>,
    /// Hash over the current artifact snapshot.
    pub snapshot_hash: String,
    /// Hash over the exported graph.
    pub graph_hash: String,
    /// Hash over written page content, excluding run-specific metadata.
    pub output_hash: String,
    /// Artifact paths changed since the previous run.
    pub changed_artifacts: Vec<String>,
    /// Page IDs actually rewritten this run.
    pub changed_pages: Vec<String>,
    /// Graph node count for this run (LIT-22.8.4 AC1: "graph size").
    #[serde(default)]
    pub graph_node_count: usize,
    /// Graph relation count for this run (LIT-22.8.4 AC1: "graph size").
    #[serde(default)]
    pub graph_relation_count: usize,
    /// Artifacts served from the analysis cache by unchanged content hash
    /// this run (LIT-22.8.4 AC1: "cache hit rate" numerator).
    #[serde(default)]
    pub cache_hits: usize,
    /// Artifacts actually read and reparsed this run (LIT-22.8.4 AC1:
    /// "cache hit rate" denominator term).
    #[serde(default)]
    pub cache_misses: usize,
    /// Deterministic, network-free token-count estimate for every model
    /// prompt composed this run (see [`estimate_tokens`]). Never a live API
    /// usage count (LIT-22.8.4 AC1: "token estimate").
    #[serde(default)]
    pub estimated_prompt_tokens: u64,
}

impl RunMetadata {
    /// Fraction of artifacts served from cache this run: `1.0` when there
    /// were no artifacts to reanalyze at all (vacuously fully cached),
    /// otherwise `cache_hits / (cache_hits + cache_misses)`.
    pub(crate) fn cache_hit_rate(&self) -> f64 {
        let total = self.cache_hits + self.cache_misses;
        if total == 0 {
            1.0
        } else {
            self.cache_hits as f64 / total as f64
        }
    }
}

/// Rough, deterministic token-count estimate from a prompt's character
/// count: the standard ~4-characters-per-token heuristic for English/code
/// text. Deliberately never a live API-reported usage count, so it stays
/// available offline and reproducibly across runs (LIT-22.8.4 AC1).
pub(crate) fn estimate_tokens(prompt_chars: usize) -> u64 {
    (prompt_chars as u64).div_ceil(4)
}

/// Inputs required to compute one run metadata record.
pub(crate) struct RunMetadataInput<'a> {
    /// CLI command that produced this run.
    pub command: &'a str,
    /// Repository root used to read git metadata.
    pub repo_root: &'a Path,
    /// Current artifact inventory.
    pub artifacts: &'a [Artifact],
    /// Graph produced for this run.
    pub graph: &'a Graph,
    /// Page manifest after rendering decisions.
    pub manifest: &'a PageManifest,
    /// Page IDs written during this run.
    pub written_pages: &'a [String],
    /// Previous artifact snapshot, when one exists.
    pub previous_snapshot: Option<&'a RepositorySnapshot>,
    /// Current pipeline invalidation metadata.
    pub pipeline: PipelineInvalidationMetadata,
    /// Analysis cache hits this run.
    pub cache_hits: usize,
    /// Analysis cache misses this run.
    pub cache_misses: usize,
    /// Total prompt character count composed this run, converted to
    /// [`estimate_tokens`].
    pub prompt_chars: usize,
}

impl RunMetadata {
    /// Computes run metadata for one completed run.
    pub(crate) fn compute(
        input: RunMetadataInput<'_>,
    ) -> Result<(Self, RepositorySnapshot), serde_json::Error> {
        let RunMetadataInput {
            command,
            repo_root,
            artifacts,
            graph,
            manifest,
            written_pages,
            previous_snapshot,
            pipeline,
            cache_hits,
            cache_misses,
            prompt_chars,
        } = input;
        let snapshot = RepositorySnapshot::from_artifacts(artifacts, pipeline);
        let changed_artifacts = snapshot.changed_since(previous_snapshot);
        let graph_hash = blake3::hash(graph.to_json()?.as_bytes())
            .to_hex()
            .to_string();

        let metadata = Self {
            run_id: run_id(),
            command: command.to_owned(),
            git_head: git_head(repo_root),
            stage_timings: Vec::new(),
            snapshot_hash: snapshot.hash(),
            graph_hash,
            output_hash: output_hash(manifest),
            changed_artifacts,
            changed_pages: written_pages.to_vec(),
            graph_node_count: graph.nodes.len(),
            graph_relation_count: graph.relations.len(),
            cache_hits,
            cache_misses,
            estimated_prompt_tokens: estimate_tokens(prompt_chars),
        };
        Ok((metadata, snapshot))
    }
}

/// A budget over deterministic run metrics -- never wall-clock duration,
/// which is nondeterministic across machines and would make any budget
/// check flaky. Every threshold is optional; an absent threshold is never
/// checked (LIT-22.8.4 AC3).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub(crate) struct PerformanceBudget {
    /// Maximum allowed graph node count.
    pub max_graph_node_count: Option<usize>,
    /// Maximum allowed graph relation count.
    pub max_graph_relation_count: Option<usize>,
    /// Minimum allowed cache hit rate, in `[0.0, 1.0]`.
    pub min_cache_hit_rate: Option<f64>,
    /// Maximum allowed estimated prompt token count.
    pub max_estimated_prompt_tokens: Option<u64>,
}

/// One budget threshold a [`RunMetadata`] exceeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct BudgetViolation {
    /// Stable metric name, e.g. `"graph_node_count"`.
    pub metric: &'static str,
    /// The configured threshold, rendered as text.
    pub limit: String,
    /// The actual value that exceeded it, rendered as text.
    pub actual: String,
}

impl PerformanceBudget {
    /// Checks `metadata` against every configured threshold, returning one
    /// [`BudgetViolation`] per threshold exceeded (empty when within
    /// budget).
    pub(crate) fn check(&self, metadata: &RunMetadata) -> Vec<BudgetViolation> {
        let mut violations = Vec::new();
        if let Some(max) = self.max_graph_node_count
            && metadata.graph_node_count > max
        {
            violations.push(BudgetViolation {
                metric: "graph_node_count",
                limit: max.to_string(),
                actual: metadata.graph_node_count.to_string(),
            });
        }
        if let Some(max) = self.max_graph_relation_count
            && metadata.graph_relation_count > max
        {
            violations.push(BudgetViolation {
                metric: "graph_relation_count",
                limit: max.to_string(),
                actual: metadata.graph_relation_count.to_string(),
            });
        }
        if let Some(min) = self.min_cache_hit_rate {
            let actual = metadata.cache_hit_rate();
            if actual < min {
                violations.push(BudgetViolation {
                    metric: "cache_hit_rate",
                    limit: min.to_string(),
                    actual: actual.to_string(),
                });
            }
        }
        if let Some(max) = self.max_estimated_prompt_tokens
            && metadata.estimated_prompt_tokens > max
        {
            violations.push(BudgetViolation {
                metric: "estimated_prompt_tokens",
                limit: max.to_string(),
                actual: metadata.estimated_prompt_tokens.to_string(),
            });
        }
        violations
    }
}

// ponytail: output_hash covers only page id + rendered output hash, never
// run_id/timestamps/git_head, so a re-run with identical page content
// produces the same output_hash even though run_id always differs.
fn output_hash(manifest: &PageManifest) -> String {
    let mut pairs: Vec<String> = manifest
        .pages
        .iter()
        .map(|page| format!("{}:{}", page.id, page.output_hash.as_deref().unwrap_or("")))
        .collect();
    pairs.sort_unstable();
    blake3::hash(pairs.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

fn run_id() -> String {
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    format!("run-{millis}")
}

fn git_head(repo_root: &Path) -> Option<String> {
    if !repo_root.join(".git").exists() {
        return None;
    }
    let output = std::process::Command::new("git")
        .arg("rev-parse")
        .arg("HEAD")
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|head| head.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        PerformanceBudget, PipelineInvalidationMetadata, RepositorySnapshot, RunMetadata,
        RunMetadataInput, estimate_tokens,
    };
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, RepoPath, SupportTier, TextStatus,
    };
    use crate::graph::Graph;
    use crate::manifest::PageManifest;

    fn artifact(path: &str, hash: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::GenericText,
            ContentHash::new(hash)?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    fn pipeline() -> PipelineInvalidationMetadata {
        PipelineInvalidationMetadata::current("v1", false, false)
    }

    #[test]
    fn first_run_reports_every_artifact_as_changed() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = RepositorySnapshot::from_artifacts(
            &[artifact("a.rs", "aaaa")?, artifact("b.rs", "bbbb")?],
            pipeline(),
        );

        let mut changed = snapshot.changed_since(None);
        changed.sort();

        assert_eq!(changed, vec!["a.rs".to_owned(), "b.rs".to_owned()]);

        Ok(())
    }

    #[test]
    fn subsequent_run_reports_only_added_removed_and_modified_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let previous = RepositorySnapshot::from_artifacts(
            &[artifact("a.rs", "aaaa")?, artifact("b.rs", "bbbb")?],
            pipeline(),
        );
        let current = RepositorySnapshot::from_artifacts(
            &[
                artifact("a.rs", "aaaa")?,
                artifact("b.rs", "beef")?,
                artifact("c.rs", "cccc")?,
            ],
            pipeline(),
        );

        let mut changed = current.changed_since(Some(&previous));
        changed.sort();

        assert_eq!(changed, vec!["b.rs".to_owned(), "c.rs".to_owned()]);

        Ok(())
    }

    #[test]
    fn pipeline_metadata_changes_mark_all_current_artifacts_changed()
    -> Result<(), Box<dyn std::error::Error>> {
        let previous = RepositorySnapshot::from_artifacts(
            &[artifact("a.rs", "aaaa")?, artifact("b.rs", "bbbb")?],
            pipeline(),
        );
        let current_artifacts = [
            artifact("a.rs", "aaaa")?,
            artifact("b.rs", "bbbb")?,
            artifact("c.rs", "cccc")?,
        ];
        let mut analyzer_version_changed = pipeline();
        analyzer_version_changed.analyzer_version += 1;
        let mut registry_version_changed = pipeline();
        registry_version_changed.language_registry_version += 1;
        let mut graph_pipeline_version_changed = pipeline();
        graph_pipeline_version_changed.graph_pipeline_version += 1;
        let mut prompt_version_changed = pipeline();
        prompt_version_changed.prompt_version = "v2".to_owned();
        let mut config_changed = pipeline();
        config_changed.semantic_grouping = true;

        for changed_pipeline in [
            analyzer_version_changed,
            registry_version_changed,
            graph_pipeline_version_changed,
            prompt_version_changed,
            config_changed,
        ] {
            let current = RepositorySnapshot::from_artifacts(&current_artifacts, changed_pipeline);
            let mut changed = current.changed_since(Some(&previous));
            changed.sort();

            assert_eq!(
                changed,
                vec!["a.rs".to_owned(), "b.rs".to_owned(), "c.rs".to_owned()]
            );
            assert_ne!(previous.hash(), current.hash());
        }

        Ok(())
    }

    #[test]
    fn no_op_run_has_stable_hashes_and_no_changed_pages() -> Result<(), Box<dyn std::error::Error>>
    {
        let artifacts = vec![artifact("a.rs", "aaaa")?];
        let graph = Graph::default();
        let manifest = PageManifest::default();
        let snapshot = RepositorySnapshot::from_artifacts(&artifacts, pipeline());

        let (first, _) = RunMetadata::compute(RunMetadataInput {
            command: "init",
            repo_root: std::path::Path::new("."),
            artifacts: &artifacts,
            graph: &graph,
            manifest: &manifest,
            written_pages: &[],
            previous_snapshot: None,
            pipeline: pipeline(),
            cache_hits: 0,
            cache_misses: 1,
            prompt_chars: 0,
        })?;
        let (second, _) = RunMetadata::compute(RunMetadataInput {
            command: "init",
            repo_root: std::path::Path::new("."),
            artifacts: &artifacts,
            graph: &graph,
            manifest: &manifest,
            written_pages: &[],
            previous_snapshot: Some(&snapshot),
            pipeline: pipeline(),
            cache_hits: 1,
            cache_misses: 0,
            prompt_chars: 0,
        })?;

        assert_eq!(first.snapshot_hash, second.snapshot_hash);
        assert_eq!(first.graph_hash, second.graph_hash);
        assert_eq!(first.output_hash, second.output_hash);
        assert!(second.changed_artifacts.is_empty());
        assert!(second.changed_pages.is_empty());
        assert_ne!(first.run_id, "");

        Ok(())
    }

    #[test]
    fn compute_records_graph_size_cache_stats_and_token_estimate()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("a.rs", "aaaa")?, artifact("b.rs", "bbbb")?];
        let graph = Graph::default();
        let manifest = PageManifest::default();

        let (metadata, _) = RunMetadata::compute(RunMetadataInput {
            command: "init",
            repo_root: std::path::Path::new("."),
            artifacts: &artifacts,
            graph: &graph,
            manifest: &manifest,
            written_pages: &[],
            previous_snapshot: None,
            pipeline: pipeline(),
            cache_hits: 3,
            cache_misses: 1,
            prompt_chars: 400,
        })?;

        assert_eq!(metadata.graph_node_count, 0);
        assert_eq!(metadata.graph_relation_count, 0);
        assert_eq!(metadata.cache_hits, 3);
        assert_eq!(metadata.cache_misses, 1);
        assert_eq!(metadata.cache_hit_rate(), 0.75);
        assert_eq!(metadata.estimated_prompt_tokens, 100);

        Ok(())
    }

    #[test]
    fn cache_hit_rate_is_vacuously_full_when_nothing_was_scanned() {
        let metadata = RunMetadata {
            run_id: String::new(),
            command: String::new(),
            git_head: None,
            stage_timings: Vec::new(),
            snapshot_hash: String::new(),
            graph_hash: String::new(),
            output_hash: String::new(),
            changed_artifacts: Vec::new(),
            changed_pages: Vec::new(),
            graph_node_count: 0,
            graph_relation_count: 0,
            cache_hits: 0,
            cache_misses: 0,
            estimated_prompt_tokens: 0,
        };

        assert_eq!(metadata.cache_hit_rate(), 1.0);
    }

    #[test]
    fn estimate_tokens_rounds_up_to_the_nearest_token() {
        assert_eq!(estimate_tokens(0), 0);
        assert_eq!(estimate_tokens(1), 1);
        assert_eq!(estimate_tokens(4), 1);
        assert_eq!(estimate_tokens(5), 2);
        assert_eq!(estimate_tokens(400), 100);
    }

    fn metadata_with(
        graph_node_count: usize,
        graph_relation_count: usize,
        cache_hits: usize,
        cache_misses: usize,
        estimated_prompt_tokens: u64,
    ) -> RunMetadata {
        RunMetadata {
            run_id: String::new(),
            command: String::new(),
            git_head: None,
            stage_timings: Vec::new(),
            snapshot_hash: String::new(),
            graph_hash: String::new(),
            output_hash: String::new(),
            changed_artifacts: Vec::new(),
            changed_pages: Vec::new(),
            graph_node_count,
            graph_relation_count,
            cache_hits,
            cache_misses,
            estimated_prompt_tokens,
        }
    }

    #[test]
    fn performance_budget_passes_a_fixture_within_every_threshold() {
        let metadata = metadata_with(10, 20, 9, 1, 500);
        let budget = PerformanceBudget {
            max_graph_node_count: Some(50),
            max_graph_relation_count: Some(100),
            min_cache_hit_rate: Some(0.5),
            max_estimated_prompt_tokens: Some(1_000),
        };

        assert!(budget.check(&metadata).is_empty());
    }

    #[test]
    fn performance_budget_reports_every_exceeded_threshold_deterministically() {
        let metadata = metadata_with(100, 200, 1, 9, 5_000);
        let budget = PerformanceBudget {
            max_graph_node_count: Some(50),
            max_graph_relation_count: Some(100),
            min_cache_hit_rate: Some(0.5),
            max_estimated_prompt_tokens: Some(1_000),
        };

        let violations = budget.check(&metadata);

        assert_eq!(violations.len(), 4);
        let metrics: Vec<&str> = violations.iter().map(|v| v.metric).collect();
        assert_eq!(
            metrics,
            vec![
                "graph_node_count",
                "graph_relation_count",
                "cache_hit_rate",
                "estimated_prompt_tokens",
            ]
        );

        // Re-checking the same fixture twice yields byte-identical
        // violations: budgets fail only through deterministic thresholds
        // over deterministic metrics, never wall-clock timing (AC3).
        assert_eq!(violations, budget.check(&metadata));
    }

    #[test]
    fn performance_budget_with_no_thresholds_never_fails() {
        let metadata = metadata_with(usize::MAX, usize::MAX, 0, usize::MAX, u64::MAX);

        assert!(PerformanceBudget::default().check(&metadata).is_empty());
    }
}
