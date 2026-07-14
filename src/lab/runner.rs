//! Offline lab execution, expectation evaluation, run storage, and replay.

use crate::domain::Artifact;
use crate::graph::{
    CommunityScope, CommunitySnapshotStore, CommunitySummary, Graph, GraphBuildTraceConfig,
    GraphBuildTraceDetail, GraphNode, GraphValidator, RelationKind, analyze_communities,
    environment_aware_scope, filter_classes, leiden_communities,
};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::lab::corpus::{Corpus, CorpusError};
use crate::lab::metrics::{ConfusionMatrix, mean_reciprocal_rank};
use crate::lab::model::*;
use crate::storage::JsonStore;
use serde::Serialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Lab operation failure.
#[derive(Debug)]
pub enum LabError {
    /// Filesystem failure.
    Io(std::io::Error),
    /// JSON failure.
    Json(serde_json::Error),
    /// Corpus failure.
    Corpus(CorpusError),
    /// Repository walk failure.
    Walk(crate::inventory::WalkError),
    /// Invalid request or failed baseline check.
    Invalid(String),
}

impl Display for LabError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => Display::fmt(error, formatter),
            Self::Json(error) => Display::fmt(error, formatter),
            Self::Corpus(error) => Display::fmt(error, formatter),
            Self::Walk(error) => Display::fmt(error, formatter),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LabError {}

impl From<std::io::Error> for LabError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for LabError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<CorpusError> for LabError {
    fn from(value: CorpusError) -> Self {
        Self::Corpus(value)
    }
}

impl From<crate::inventory::WalkError> for LabError {
    fn from(value: crate::inventory::WalkError) -> Self {
        Self::Walk(value)
    }
}

/// Baseline lab bound to one corpus and artifact root.
#[derive(Debug, Clone)]
pub struct Lab {
    /// Loaded corpus.
    pub corpus: Corpus,
    /// Content-addressed run and committed-baseline root.
    pub root: PathBuf,
}

impl Lab {
    /// Creates a lab.
    pub fn new(corpus: Corpus, root: PathBuf) -> Self {
        Self { corpus, root }
    }

    /// Runs all selected cases and returns their content-addressed run directories.
    pub fn run(&self, suite: SuiteTier, case_id: Option<&str>) -> Result<Vec<PathBuf>, LabError> {
        let cases = self.corpus.cases(suite, case_id);
        if cases.is_empty() {
            return Err(LabError::Invalid(format!(
                "no corpus cases selected for {suite:?}{}",
                case_id.map_or(String::new(), |id| format!(" and `{id}`"))
            )));
        }
        let cases = cases.into_iter().cloned().collect::<Vec<_>>();
        cases
            .iter()
            .map(|case| self.run_case(case, suite))
            .collect()
    }

    /// Executes one case entirely offline.
    pub fn run_case(&self, case: &CorpusCase, suite: SuiteTier) -> Result<PathBuf, LabError> {
        self.run_case_with_cache(case, suite, None)
    }

    fn run_case_with_cache(
        &self,
        case: &CorpusCase,
        suite: SuiteTier,
        analysis_cache: Option<&crate::analysis::AnalysisCache>,
    ) -> Result<PathBuf, LabError> {
        let repo_root = self.corpus.resolve_root(case)?;
        let start = Instant::now();
        let artifacts = RepositoryWalker::new(WalkOptions {
            exclude_globs: case.exclude.clone(),
            ..WalkOptions::default()
        })
        .walk(&repo_root)?;
        let inventory_ms = millis(start.elapsed());
        let inventory_hash = hash_json(&artifacts)?;
        let graph_start = Instant::now();
        let output = crate::graph::GraphBuilder.build_with_trace(
            &repo_root,
            &artifacts,
            analysis_cache,
            GraphBuildTraceConfig {
                detail: if case.tier == SuiteTier::Pr {
                    GraphBuildTraceDetail::Full
                } else {
                    GraphBuildTraceDetail::Summary
                },
                selectors: Vec::new(),
            },
        );
        let graph_ms = millis(graph_start.elapsed());
        let mut trace = output
            .trace
            .ok_or_else(|| LabError::Invalid("graph builder omitted requested trace".to_owned()))?;
        let graph_hash = hash_json(&output.graph)?;
        let communities_start = Instant::now();
        let community_store = CommunitySnapshotStore::new(self.root.join("work/community-cache"));
        let community_analysis = analyze_communities(
            &output.graph,
            &CommunityScope::Combined,
            Some(&community_store),
        )?;
        let communities_ms = millis(communities_start.elapsed());
        let community_diagnostics = community_analysis.diagnostics;
        let community_cache_hit = community_analysis.cache_hit;
        let communities = community_analysis.communities;
        let candidate_scope_analysis = analyze_communities(
            &output.graph,
            &environment_aware_scope(),
            Some(&community_store),
        )?;
        let query_start = Instant::now();
        let _schema = crate::graph::KnowledgeIndex::new(&output.graph).schema();
        let query_ms = millis(query_start.elapsed());
        let layout_start = Instant::now();
        let _layout =
            crate::graph::compute_layout(&output.graph, &crate::graph::LayoutRequest::default())
                .map_err(LabError::Invalid)?;
        let layout_ms = millis(layout_start.elapsed());
        let semantic_start = Instant::now();
        let _semantic = filter_classes(&output.graph, "controller service persistence test");
        let semantic_ms = millis(semantic_start.elapsed());
        let expectations: ExpectationSet = read_required(&self.corpus.expectation_path(case)?)?;
        if expectations.schema_version != LAB_SCHEMA_VERSION {
            return Err(LabError::Invalid(format!(
                "expectation schema {} is unsupported",
                expectations.schema_version
            )));
        }
        let validation_start = Instant::now();
        let validation_issue_count =
            GraphValidator.validate(&output.graph, &artifacts).len() as u64;
        let validation_ms = millis(validation_start.elapsed());
        let mut assertions = evaluate(
            &expectations.expectations,
            &artifacts,
            &output.graph,
            &communities,
        );
        if case.tier == SuiteTier::Pr {
            assertions.push(differential_python_definitions(&repo_root, &output.graph)?);
            assertions.push(self.generated_parser_robustness()?);
        }
        apply_known_failures(&mut assertions, &expectations.known_failures)?;
        if case.tier != SuiteTier::Pr && assertions.iter().any(|assertion| !assertion.passed) {
            let selectors = failed_trace_selectors(
                &expectations.expectations,
                &assertions,
                &artifacts,
                &output.graph,
            );
            if !selectors.is_empty() {
                let focused = crate::graph::GraphBuilder.build_with_trace(
                    &repo_root,
                    &artifacts,
                    analysis_cache,
                    GraphBuildTraceConfig {
                        detail: GraphBuildTraceDetail::Summary,
                        selectors,
                    },
                );
                if let Some(focused_trace) = focused.trace {
                    for stage in &mut trace.stages {
                        if let Some(focused_stage) = focused_trace
                            .stages
                            .iter()
                            .find(|candidate| candidate.pass == stage.pass)
                        {
                            stage.decisions = focused_stage.decisions.clone();
                        }
                    }
                }
            }
        }
        let cache_hits = analysis_cache.map_or(0, |cache| cache.hits() as u64);
        let cache_misses = analysis_cache.map_or(0, |cache| cache.misses() as u64);
        let mut equivalence_cache_hits = 0u64;
        let mut equivalence_cache_misses = 0u64;
        if case.tier == SuiteTier::Pr {
            let cache_root = self.root.join("work/cache").join(&case.id);
            let cache = crate::analysis::AnalysisCache::new(cache_root);
            let cached_first =
                crate::graph::GraphBuilder.build_with_cache(&repo_root, &artifacts, Some(&cache));
            let cached_second =
                crate::graph::GraphBuilder.build_with_cache(&repo_root, &artifacts, Some(&cache));
            equivalence_cache_hits = cache.hits() as u64;
            equivalence_cache_misses = cache.misses() as u64;
            let equivalent = output.graph == cached_first && cached_first == cached_second;
            assertions.push(AssertionResult {
                id: "cache-equivalence".to_owned(),
                passed: equivalent,
                stage: "finalize".to_owned(),
                detail: format!(
                    "expected fresh, cold-cache, and warm-cache graphs to match; observed equivalent={equivalent}, hits={}, misses={}",
                    cache.hits(),
                    cache.misses()
                ),
                expected_failure: None,
            });
            assertions.extend(self.run_scenarios(case, &repo_root, &expectations)?);
        }
        let metrics = derive_metrics(&expectations.expectations, &assertions, &output.graph);
        let differentials = vec![
            differential_cargo_metadata(&repo_root, &output.graph),
            differential_typescript_compiler(&repo_root, &output.graph),
            differential_scip(&repo_root, &output.graph),
        ];
        let source_revision = match &case.source {
            CorpusSource::Fixture { .. } => format!("fixture:{inventory_hash}"),
            CorpusSource::Git { commit, .. } => commit.clone(),
        };
        let stage_hashes: Vec<_> = trace
            .stages
            .iter()
            .map(|stage| (stage.pass.as_str(), stage.graph_hash.clone()))
            .collect();
        let run_id = hash_json(&json!({
            "schema_version": LAB_SCHEMA_VERSION,
            "case_id": case.id,
            "suite": suite,
            "source_revision": source_revision,
            "inventory_hash": inventory_hash,
            "graph_hash": graph_hash,
            "stage_hashes": stage_hashes,
            "assertions": assertions,
            "metrics": metrics,
            "differentials": differentials,
        }))?;
        let mut observations = BTreeMap::new();
        observations.insert("inventory_ms".to_owned(), inventory_ms);
        observations.insert("graph_ms".to_owned(), graph_ms);
        observations.insert("communities_ms".to_owned(), communities_ms);
        observations.insert(
            "community_participating_nodes".to_owned(),
            community_diagnostics.participating_nodes,
        );
        observations.insert(
            "community_selected_edges".to_owned(),
            community_diagnostics.selected_edges,
        );
        observations.insert(
            "community_iterations".to_owned(),
            community_diagnostics.iterations,
        );
        observations.insert(
            "community_nodes_reconsidered".to_owned(),
            community_diagnostics.nodes_reconsidered,
        );
        observations.insert(
            "community_successful_moves".to_owned(),
            community_diagnostics.successful_moves,
        );
        observations.insert(
            "community_neighbour_label_evaluations".to_owned(),
            community_diagnostics.neighbour_label_evaluations,
        );
        observations.insert(
            "community_summary_edge_visits".to_owned(),
            community_diagnostics.summary_edge_visits,
        );
        observations.insert(
            "community_safety_bound_reached".to_owned(),
            u64::from(community_diagnostics.safety_bound_reached),
        );
        observations.insert(
            "community_adjacency_us".to_owned(),
            community_diagnostics.adjacency_us,
        );
        observations.insert(
            "community_movement_us".to_owned(),
            community_diagnostics.movement_us,
        );
        observations.insert(
            "community_summary_us".to_owned(),
            community_diagnostics.summary_us,
        );
        observations.insert(
            "community_cache_hit".to_owned(),
            u64::from(community_cache_hit),
        );
        let scope_comparison = compare_community_scopes(
            &output.graph,
            &expectations.expectations,
            &communities,
            &candidate_scope_analysis.communities,
        );
        observations.insert(
            "community_candidate_selected_edges".to_owned(),
            candidate_scope_analysis.diagnostics.selected_edges,
        );
        observations.insert(
            "community_candidate_total_us".to_owned(),
            candidate_scope_analysis
                .diagnostics
                .adjacency_us
                .saturating_add(candidate_scope_analysis.diagnostics.movement_us)
                .saturating_add(candidate_scope_analysis.diagnostics.summary_us),
        );
        observations.insert(
            "community_candidate_ari_millionths".to_owned(),
            millionths(scope_comparison.ari),
        );
        observations.insert(
            "community_candidate_nmi_millionths".to_owned(),
            millionths(scope_comparison.nmi),
        );
        observations.insert(
            "community_candidate_pair_accuracy_millionths".to_owned(),
            millionths(scope_comparison.pair_accuracy),
        );
        observations.insert(
            "community_candidate_mean_cohesion_millionths".to_owned(),
            millionths(scope_comparison.mean_cohesion),
        );
        observations.insert(
            "community_candidate_mean_conductance_millionths".to_owned(),
            millionths(scope_comparison.mean_conductance),
        );
        observations.insert("artifact_count".to_owned(), artifacts.len() as u64);
        observations.insert("node_count".to_owned(), output.graph.nodes.len() as u64);
        observations.insert(
            "relation_count".to_owned(),
            output.graph.relations.len() as u64,
        );
        observations.insert("community_count".to_owned(), communities.len() as u64);
        observations.insert("cache_hits".to_owned(), cache_hits);
        observations.insert("cache_misses".to_owned(), cache_misses);
        observations.insert("cache_equivalence_hits".to_owned(), equivalence_cache_hits);
        observations.insert(
            "cache_equivalence_misses".to_owned(),
            equivalence_cache_misses,
        );
        observations.insert("validation_ms".to_owned(), validation_ms);
        observations.insert("validation_issue_count".to_owned(), validation_issue_count);
        observations.insert("query_ms".to_owned(), query_ms);
        observations.insert("layout_ms".to_owned(), layout_ms);
        observations.insert("semantic_ms".to_owned(), semantic_ms);
        observations.insert("peak_rss_kib".to_owned(), process_rss_kib());
        if !output.graph.nodes.is_empty() {
            observations.insert(
                "graph_us_per_1k_nodes".to_owned(),
                graph_ms
                    .saturating_mul(1_000_000)
                    .checked_div(output.graph.nodes.len() as u64)
                    .unwrap_or(0),
            );
        }
        observations.insert(
            "oracle_labeled_expectation_count".to_owned(),
            expectations.expectations.len() as u64,
        );
        observations.insert(
            "oracle_unlabeled_area_count".to_owned(),
            expectations.truth_pack.unlabeled.len() as u64,
        );
        for stage in &trace.stages {
            observations.insert(
                format!("stage_{}_us", stage.pass.as_str()),
                stage.duration_us,
            );
            for (name, value) in &stage.counters {
                observations.insert(format!("stage_{}_{}", stage.pass.as_str(), name), *value);
            }
            for (name, value) in &stage.component_durations_us {
                observations.insert(format!("component_{name}_us"), *value);
            }
        }
        observations.insert("stage_definitions_cache_hits".to_owned(), cache_hits);
        observations.insert("stage_definitions_cache_misses".to_owned(), cache_misses);
        let mut manifest = RunManifest {
            schema_version: LAB_SCHEMA_VERSION,
            graph_pipeline_version: crate::graph::GRAPH_BUILD_PIPELINE_VERSION,
            run_id: run_id.clone(),
            case_id: case.id.clone(),
            suite,
            source_revision,
            inventory_hash,
            graph_hash,
            stages: trace.stages,
            assertions,
            metrics,
            differentials,
            observations,
            reproduce: format!("just baseline-replay {run_id}"),
        };
        self.persist_run(&mut manifest, case, &artifacts, &output.graph, &communities)
    }

    /// Loads a run by directory or content id.
    pub fn load_run(&self, run: &Path) -> Result<RunManifest, LabError> {
        let path = if run.is_dir() {
            run.join("manifest.json")
        } else {
            self.root.join("runs").join(run).join("manifest.json")
        };
        read_compatible(&path)
    }

    /// Compares a run with its accepted case baseline.
    pub fn check(&self, run: &RunManifest) -> Result<BaselineDiff, LabError> {
        let baseline: BaselineRecord = read_compatible(&self.corpus.baseline_path(&run.case_id)?)?;
        Ok(diff(&baseline, run))
    }

    /// Builds the semantic review preview and freshness token required by
    /// [`Self::accept`]. The token binds the candidate, current baseline,
    /// diff, and reason so it cannot be reused after any of them changes.
    pub fn acceptance_review(
        &self,
        run: &RunManifest,
        reason: &str,
    ) -> Result<AcceptanceReview, LabError> {
        if reason.trim().is_empty() {
            return Err(LabError::Invalid(
                "baseline acceptance requires --reason".to_owned(),
            ));
        }
        let path = self.corpus.baseline_path(&run.case_id)?;
        let previous: Option<BaselineRecord> = read_optional_compatible(&path)?;
        let baseline = previous.unwrap_or_else(|| empty_baseline(run));
        let diff = diff(&baseline, run);
        let reason = reason.trim().to_owned();
        let confirmation_token = hash_json(&json!({
            "run_id": run.run_id,
            "reason": reason,
            "baseline": baseline,
            "diff": diff,
        }))?;
        Ok(AcceptanceReview {
            run_id: run.run_id.clone(),
            reason,
            diff,
            confirmation_token,
        })
    }

    /// Explicitly accepts a reviewed clean run. Acceptance is disabled in CI
    /// and requires the fresh token returned by [`Self::acceptance_review`].
    pub fn accept(
        &self,
        run: &RunManifest,
        reason: &str,
        confirmation_token: &str,
    ) -> Result<BaselineRecord, LabError> {
        self.accept_with_policy(
            run,
            reason,
            confirmation_token,
            std::env::var_os("CI").is_some(),
        )
    }

    fn accept_with_policy(
        &self,
        run: &RunManifest,
        reason: &str,
        confirmation_token: &str,
        ci: bool,
    ) -> Result<BaselineRecord, LabError> {
        if ci {
            return Err(LabError::Invalid(
                "baseline acceptance is disabled in CI".to_owned(),
            ));
        }
        if !run.is_clean() {
            return Err(LabError::Invalid(
                "refusing to accept a run with failed assertions or metric bounds".to_owned(),
            ));
        }
        let review = self.acceptance_review(run, reason)?;
        if confirmation_token != review.confirmation_token {
            return Err(LabError::Invalid(format!(
                "baseline review confirmation is missing or stale; review the semantic diff and retry with --confirm {}",
                review.confirmation_token
            )));
        }
        let path = self.corpus.baseline_path(&run.case_id)?;
        let previous: Option<BaselineRecord> = read_optional_compatible(&path)?;
        let baseline = baseline_from_run(run, reason, previous.as_ref());
        atomic_json_write(&path, &baseline)?;
        Ok(baseline)
    }

    /// Returns a focused explanation for one assertion.
    pub fn explain(&self, run: &RunManifest, id: &str) -> Result<Value, LabError> {
        let assertion = run
            .assertions
            .iter()
            .find(|result| result.id == id)
            .ok_or_else(|| LabError::Invalid(format!("run has no assertion `{id}`")))?;
        Ok(json!({
            "run_id": run.run_id,
            "case_id": run.case_id,
            "assertion": assertion,
            "replay": run.reproduce,
            "inspect_stage": format!("cargo run --bin lithograph-lab -- inspect {} --stage {}", run.run_id, assertion.stage),
        }))
    }

    /// Replays the exact case and suite identified by a prior run.
    pub fn replay(&self, run: &RunManifest) -> Result<PathBuf, LabError> {
        if run.graph_pipeline_version != crate::graph::GRAPH_BUILD_PIPELINE_VERSION {
            return Err(LabError::Invalid(format!(
                "run graph pipeline version {} is incompatible with current version {}; migrate mechanical schema fields or regenerate and semantically review the baseline",
                run.graph_pipeline_version,
                crate::graph::GRAPH_BUILD_PIPELINE_VERSION
            )));
        }
        let case = self.corpus.case(&run.case_id)?.clone();
        let current_revision = match &case.source {
            CorpusSource::Fixture { .. } => None,
            CorpusSource::Git { commit, .. } => Some(commit.as_str()),
        };
        if current_revision.is_some_and(|revision| revision != run.source_revision) {
            return Err(LabError::Invalid(format!(
                "replay source changed: run used {}, corpus pins {}",
                run.source_revision,
                current_revision.unwrap_or_default()
            )));
        }
        self.run_case(&case, run.suite)
    }

    /// Lists persisted run ids.
    pub fn list_runs(&self) -> Result<Vec<String>, LabError> {
        let root = self.root.join("runs");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut runs = std::fs::read_dir(root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("manifest.json").is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        runs.sort();
        Ok(runs)
    }

    /// Previews or applies a purely mechanical lab JSON schema migration.
    /// It never updates semantic baselines through the acceptance path.
    pub fn migrate(&self, path: &Path, apply: bool) -> Result<MigrationReport, LabError> {
        let bytes = std::fs::read(path)?;
        let mut value: Value = serde_json::from_slice(&bytes)?;
        let from_version = schema_version(&value)?;
        let changes = migrate_value(&mut value)?;
        if apply && !changes.is_empty() {
            atomic_json_write(path, &value)?;
        }
        Ok(MigrationReport {
            path: path.display().to_string(),
            from_version,
            to_version: LAB_SCHEMA_VERSION,
            applied: apply && !changes.is_empty(),
            changes,
        })
    }

    /// Produces a source-free failure slice and optionally materializes only
    /// its evidence files into an explicitly local directory.
    pub fn minimize(
        &self,
        run: &RunManifest,
        materialize: Option<&Path>,
    ) -> Result<MinimizedFailureBundle, LabError> {
        let failures = run
            .assertions
            .iter()
            .filter(|assertion| !assertion.passed)
            .cloned()
            .collect::<Vec<_>>();
        if failures.is_empty() {
            return Err(LabError::Invalid(
                "failure minimization requires at least one failing assertion".to_owned(),
            ));
        }
        let signature = hash_json(&failures)?;
        let run_root = self.root.join("runs").join(&run.run_id);
        let artifacts: Vec<Artifact> = read_required(&run_root.join("inventory.json"))?;
        let graph: Graph = read_required(&run_root.join("graph.json"))?;
        let case = self.corpus.case(&run.case_id)?;
        let source = self.corpus.resolve_root(case)?;
        let expectations: ExpectationSet = read_required(&self.corpus.expectation_path(case)?)?;
        let minimized_artifacts = minimize_artifacts_preserving_signature(
            &source,
            &artifacts,
            &expectations,
            &signature,
        )?;
        let mut files = minimized_artifacts
            .iter()
            .map(|artifact| artifact.path.as_str().to_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let mut nodes = std::collections::BTreeSet::new();
        for decision in run.stages.iter().flat_map(|stage| &stage.decisions) {
            nodes.insert(decision.source.clone());
            nodes.insert(decision.target.clone());
            files.extend(decision.evidence_paths.iter().cloned());
        }
        let relations = graph
            .relations
            .iter()
            .filter(|relation| {
                nodes.contains(relation.source.as_str()) || nodes.contains(relation.target.as_str())
            })
            .cloned()
            .collect::<Vec<_>>();
        let materialized_at = if let Some(destination) = materialize {
            if destination.exists() {
                return Err(LabError::Invalid(format!(
                    "refusing to overwrite local minimized fixture {}",
                    destination.display()
                )));
            }
            for file in &files {
                let input = source.join(file);
                if !input.is_file() {
                    continue;
                }
                let output = destination.join(file);
                if let Some(parent) = output.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(input, output)?;
            }
            Some(destination.display().to_string())
        } else {
            None
        };
        let bundle = MinimizedFailureBundle {
            run_id: run.run_id.clone(),
            case_id: run.case_id.clone(),
            suite: run.suite,
            source_revision: run.source_revision.clone(),
            failure_signature: signature,
            failures,
            relevant_files: files.into_iter().collect(),
            relevant_nodes: nodes.into_iter().collect(),
            relevant_relations: relations,
            materialized_at,
        };
        JsonStore.write(&run_root.join("minimized.json"), &bundle)?;
        if let Some(destination) = materialize {
            JsonStore.write(&destination.join("minimized.json"), &bundle)?;
        }
        Ok(bundle)
    }

    /// Runs repeated warm samples and stores robust median/MAD summaries
    /// separately from deterministic correctness baselines.
    pub fn benchmark(
        &self,
        suite: SuiteTier,
        case_id: Option<&str>,
        samples: usize,
        mode: BenchmarkMode,
        gate: bool,
    ) -> Result<Vec<PerformanceSummary>, LabError> {
        if samples < 3 {
            return Err(LabError::Invalid(
                "performance benchmarking requires at least three samples".to_owned(),
            ));
        }
        if mode == BenchmarkMode::CommunityOnly {
            return self.benchmark_communities(suite, case_id, samples, gate);
        }
        let cases = self
            .corpus
            .cases(suite, case_id)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let mut summaries = Vec::new();
        let budgets: PerformanceBudgetManifest =
            read_required(&self.corpus.performance_budget_path()?)?;
        if budgets.schema_version != LAB_SCHEMA_VERSION {
            return Err(LabError::Invalid(format!(
                "performance budget schema {} is unsupported",
                budgets.schema_version
            )));
        }
        let mut gated_failures = Vec::new();
        for case in cases {
            let mut runs = Vec::with_capacity(samples);
            let cache_root = self
                .root
                .join("work/benchmark-cache")
                .join(&case.id)
                .join(mode.as_str());
            if mode == BenchmarkMode::Cold && cache_root.exists() {
                std::fs::remove_dir_all(&cache_root)?;
            }
            if mode != BenchmarkMode::Cold {
                let seed_cache = crate::analysis::AnalysisCache::new(&cache_root);
                let _ = self.run_case_with_cache(&case, suite, Some(&seed_cache))?;
            }
            let community_cache_root = self.root.join("work/community-cache");
            if community_cache_root.exists() {
                std::fs::remove_dir_all(&community_cache_root)?;
            }
            let sample_root = self
                .root
                .join("performance/samples")
                .join(&case.id)
                .join(mode.as_str());
            let start_sequence = next_sample_sequence(&sample_root)?;
            let mut sample_files = Vec::with_capacity(samples);
            for sequence in 0..samples {
                if mode == BenchmarkMode::Cold && cache_root.exists() {
                    std::fs::remove_dir_all(&cache_root)?;
                }
                // Community phase budgets must measure the implementation,
                // not an exact snapshot hit. Graph/analyzer cache behavior is
                // still controlled independently by `mode`.
                if community_cache_root.exists() {
                    std::fs::remove_dir_all(&community_cache_root)?;
                }
                // LIT-35.5: same reasoning for the near-clone snapshot cache --
                // leaving it in place would make every sample a cache hit and
                // zero out the clone phases the clone budgets gate. Clear only
                // the clone snapshot files so the analyzer cache stays warm.
                clear_clone_snapshots(&cache_root);
                let sample_cache = crate::analysis::AnalysisCache::new(&cache_root);
                let path = self.run_case_with_cache(&case, suite, Some(&sample_cache))?;
                let run = self.load_run(&path)?;
                let ordinal = start_sequence + sequence;
                let sample_id = hash_json(&json!({
                    "case": case.id,
                    "mode": mode,
                    "sequence": ordinal,
                    "run": run.run_id,
                    "observations": run.observations,
                }))?;
                let relative = format!("{}/{}/{sample_id}.json", case.id, mode.as_str());
                let sample = PerformanceSample {
                    sample_id,
                    run_id: run.run_id.clone(),
                    mode,
                    sequence: ordinal,
                    machine: machine_fingerprint(),
                    observations: run.observations.clone(),
                    graph_hash: Some(run.graph_hash.clone()),
                    community_scope: None,
                    community_algorithm_version: None,
                    reproduce: Some(run.reproduce.clone()),
                };
                JsonStore.write(
                    &sample_root.join(format!("{}.json", sample.sample_id)),
                    &sample,
                )?;
                sample_files.push(relative);
                runs.push(run);
            }
            let history_root = self
                .root
                .join("performance/history")
                .join(machine_slug(&machine_fingerprint()))
                .join(&case.id)
                .join(mode.as_str());
            let previous = latest_performance_summary(&history_root)?;
            let mut summary = performance_summary(&runs, mode)?;
            summary.sample_files = sample_files;
            summary.history_sequence = start_sequence;
            if let Some(previous) = previous.as_ref() {
                summary.regressions = compare_performance(
                    previous,
                    &summary,
                    &budgets
                        .budgets
                        .iter()
                        .filter(|budget| budget.case_id == case.id && budget.mode == mode)
                        .cloned()
                        .collect::<Vec<_>>(),
                )?;
                if gate
                    && summary
                        .regressions
                        .iter()
                        .any(|regression| !regression.passed)
                {
                    let replay = runs
                        .last()
                        .map(|run| run.reproduce.as_str())
                        .unwrap_or("rerun the benchmark case");
                    for regression in summary
                        .regressions
                        .iter()
                        .filter(|regression| !regression.passed)
                    {
                        gated_failures.push(format!(
                            "{} {} phase `{}` regressed: median {} -> {} ({:+.1}%, allowed {:+.1}%); replay: {}",
                            case.id,
                            mode.as_str(),
                            regression.metric,
                            regression.previous_median,
                            regression.current_median,
                            regression.relative_increase * 100.0,
                            regression.allowed_increase * 100.0,
                            replay,
                        ));
                    }
                }
            }
            JsonStore.write(
                &self
                    .root
                    .join("performance")
                    .join(format!("{}-{}.json", case.id, mode.as_str())),
                &summary,
            )?;
            let summary_id = hash_json(&summary)?;
            JsonStore.write(&history_root.join(format!("{summary_id}.json")), &summary)?;
            summaries.push(summary);
        }
        if gated_failures.is_empty() {
            Ok(summaries)
        } else {
            Err(LabError::Invalid(gated_failures.join("\n")))
        }
    }

    fn benchmark_communities(
        &self,
        suite: SuiteTier,
        case_id: Option<&str>,
        samples: usize,
        gate: bool,
    ) -> Result<Vec<PerformanceSummary>, LabError> {
        let cases = self
            .corpus
            .cases(suite, case_id)
            .into_iter()
            .cloned()
            .collect::<Vec<_>>();
        let budgets: PerformanceBudgetManifest =
            read_required(&self.corpus.performance_budget_path()?)?;
        let machine = machine_fingerprint();
        let mut summaries = Vec::new();
        let mut gated_failures = Vec::new();
        for case in cases {
            let (source_run, run_root) = self.community_benchmark_source(&case.id)?;
            if source_run.graph_pipeline_version != crate::graph::GRAPH_BUILD_PIPELINE_VERSION {
                return Err(LabError::Invalid(format!(
                    "community benchmark source {} uses graph pipeline version {}, expected {}",
                    source_run.run_id,
                    source_run.graph_pipeline_version,
                    crate::graph::GRAPH_BUILD_PIPELINE_VERSION
                )));
            }
            let graph: Graph = read_required(&run_root.join("graph.json"))?;
            let verified_hash = hash_json(&graph)?;
            if verified_hash != source_run.graph_hash {
                return Err(LabError::Invalid(format!(
                    "community benchmark graph hash mismatch for {}: manifest={}, artifact={verified_hash}",
                    case.id, source_run.graph_hash
                )));
            }
            let expected: Vec<CommunitySummary> =
                read_required(&run_root.join("communities.json"))?;
            let scope = CommunityScope::Combined;
            let verification = crate::graph::leiden_communities_with_diagnostics(&graph, &scope);
            if verification.communities != expected {
                let expected_hash = hash_json(&expected)?;
                let observed_hash = hash_json(&verification.communities)?;
                let first_difference = expected
                    .iter()
                    .zip(&verification.communities)
                    .position(|(left, right)| left != right)
                    .unwrap_or(expected.len().min(verification.communities.len()));
                let difference_detail = expected
                    .get(first_difference)
                    .zip(verification.communities.get(first_difference))
                    .map(|(left, right)| {
                        format!(
                            "expected id={} cohesion={:016x} conductance={:016x} members={} boundaries={}; observed id={} cohesion={:016x} conductance={:016x} members={} boundaries={}",
                            left.id,
                            left.cohesion.to_bits(),
                            left.conductance.to_bits(),
                            left.members.len(),
                            left.boundary_edges.len(),
                            right.id,
                            right.cohesion.to_bits(),
                            right.conductance.to_bits(),
                            right.members.len(),
                            right.boundary_edges.len(),
                        )
                    })
                    .unwrap_or_else(|| "collection length differs".to_owned());
                return Err(LabError::Invalid(format!(
                    "community benchmark correctness mismatch for {} run {}: expected count/hash {}/{}, observed {}/{}, first difference {} ({difference_detail}); replay: {}",
                    case.id,
                    source_run.run_id,
                    expected.len(),
                    expected_hash,
                    verification.communities.len(),
                    observed_hash,
                    first_difference,
                    source_run.reproduce
                )));
            }
            let reproduce = format!(
                "cargo run --quiet --bin lithograph-lab -- benchmark --suite {} --case {} --samples {} --mode community-only{}",
                suite.as_str(),
                case.id,
                samples,
                if gate { " --gate" } else { "" }
            );
            let mode = BenchmarkMode::CommunityOnly;
            let sample_root = self
                .root
                .join("performance/samples")
                .join(&case.id)
                .join(mode.as_str());
            let start_sequence = next_sample_sequence(&sample_root)?;
            let mut sample_files = Vec::with_capacity(samples);
            let mut timing_runs = Vec::with_capacity(samples);
            for sequence in 0..samples {
                let analysis = crate::graph::leiden_communities_with_diagnostics(&graph, &scope);
                if analysis.communities != expected {
                    return Err(LabError::Invalid(format!(
                        "community output changed during sample {} for {}; replay: {}",
                        sequence, case.id, reproduce
                    )));
                }
                let diagnostics = analysis.diagnostics;
                let observations = BTreeMap::from([
                    (
                        "community_adjacency_us".to_owned(),
                        diagnostics.adjacency_us,
                    ),
                    ("community_movement_us".to_owned(), diagnostics.movement_us),
                    ("community_summary_us".to_owned(), diagnostics.summary_us),
                    (
                        "community_participating_nodes".to_owned(),
                        diagnostics.participating_nodes,
                    ),
                    (
                        "community_selected_edges".to_owned(),
                        diagnostics.selected_edges,
                    ),
                    ("community_iterations".to_owned(), diagnostics.iterations),
                    (
                        "community_nodes_reconsidered".to_owned(),
                        diagnostics.nodes_reconsidered,
                    ),
                    (
                        "community_successful_moves".to_owned(),
                        diagnostics.successful_moves,
                    ),
                    (
                        "community_neighbour_label_evaluations".to_owned(),
                        diagnostics.neighbour_label_evaluations,
                    ),
                    (
                        "community_summary_edge_visits".to_owned(),
                        diagnostics.summary_edge_visits,
                    ),
                    ("peak_rss_kib".to_owned(), process_rss_kib()),
                ]);
                let ordinal = start_sequence + sequence;
                let sample_id = hash_json(&json!({
                    "case": case.id,
                    "mode": mode,
                    "sequence": ordinal,
                    "run": source_run.run_id,
                    "graph_hash": verified_hash,
                    "scope": scope,
                    "algorithm_version": crate::graph::LEIDEN_ALGORITHM_VERSION,
                    "observations": observations,
                }))?;
                let relative = format!("{}/{}/{sample_id}.json", case.id, mode.as_str());
                JsonStore.write(
                    &sample_root.join(format!("{sample_id}.json")),
                    &PerformanceSample {
                        sample_id,
                        run_id: source_run.run_id.clone(),
                        mode,
                        sequence: ordinal,
                        machine: machine.clone(),
                        observations: observations.clone(),
                        graph_hash: Some(verified_hash.clone()),
                        community_scope: Some(scope.clone()),
                        community_algorithm_version: Some(crate::graph::LEIDEN_ALGORITHM_VERSION),
                        reproduce: Some(reproduce.clone()),
                    },
                )?;
                sample_files.push(relative);
                let mut timing_run = source_run.clone();
                timing_run.observations = observations;
                timing_runs.push(timing_run);
            }
            let history_root = self
                .root
                .join("performance/history")
                .join(machine_slug(&machine))
                .join(&case.id)
                .join(mode.as_str());
            let previous = latest_performance_summary(&history_root)?;
            let mut summary = performance_summary(&timing_runs, mode)?;
            summary.sample_files = sample_files;
            summary.history_sequence = start_sequence;
            summary.graph_hash = Some(verified_hash);
            summary.community_scope = Some(scope);
            summary.community_algorithm_version = Some(crate::graph::LEIDEN_ALGORITHM_VERSION);
            summary.reproduce = Some(reproduce.clone());
            if let Some(previous) = previous.as_ref() {
                summary.regressions = compare_performance(
                    previous,
                    &summary,
                    &budgets
                        .budgets
                        .iter()
                        .filter(|budget| budget.case_id == case.id && budget.mode == mode)
                        .cloned()
                        .collect::<Vec<_>>(),
                )?;
                for regression in summary
                    .regressions
                    .iter()
                    .filter(|regression| gate && !regression.passed)
                {
                    gated_failures.push(format!(
                        "{} community-only phase `{}` regressed: median {} -> {} ({:+.1}%, allowed {:+.1}%); replay: {}",
                        case.id,
                        regression.metric,
                        regression.previous_median,
                        regression.current_median,
                        regression.relative_increase * 100.0,
                        regression.allowed_increase * 100.0,
                        reproduce,
                    ));
                }
            }
            JsonStore.write(
                &self
                    .root
                    .join("performance")
                    .join(format!("{}-{}.json", case.id, mode.as_str())),
                &summary,
            )?;
            let summary_id = hash_json(&summary)?;
            JsonStore.write(&history_root.join(format!("{summary_id}.json")), &summary)?;
            summaries.push(summary);
        }
        if gated_failures.is_empty() {
            Ok(summaries)
        } else {
            Err(LabError::Invalid(gated_failures.join("\n")))
        }
    }

    fn community_benchmark_source(
        &self,
        case_id: &str,
    ) -> Result<(RunManifest, PathBuf), LabError> {
        let baseline: BaselineRecord = read_compatible(&self.corpus.baseline_path(case_id)?)?;
        let runs_root = self.root.join("runs");
        let mut candidates = std::fs::read_dir(&runs_root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().is_dir())
            .filter_map(|entry| {
                let root = entry.path();
                let run = read_compatible::<RunManifest>(&root.join("manifest.json")).ok()?;
                (run.case_id == case_id
                    && run.graph_hash == baseline.graph_hash
                    && run.is_clean()
                    && root.join("graph.json").is_file()
                    && root.join("communities.json").is_file())
                .then_some((run, root))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|left, right| left.0.run_id.cmp(&right.0.run_id));
        candidates.pop().ok_or_else(|| {
            LabError::Invalid(format!(
                "no verified persisted graph run exists for {case_id}; run the correctness suite first"
            ))
        })
    }

    fn persist_run(
        &self,
        manifest: &mut RunManifest,
        case: &CorpusCase,
        artifacts: &[Artifact],
        graph: &Graph,
        communities: &[CommunitySummary],
    ) -> Result<PathBuf, LabError> {
        let root = self.root.join("runs").join(&manifest.run_id);
        let persistence_started = Instant::now();
        JsonStore.write(&root.join("inventory.json"), &artifacts)?;
        JsonStore.write(&root.join("graph.json"), graph)?;
        JsonStore.write(&root.join("communities.json"), &communities)?;
        JsonStore.write(&root.join("assertions.json"), &manifest.assertions)?;
        JsonStore.write(
            &root.join("replay.json"),
            &ReplayBundle {
                schema_version: LAB_SCHEMA_VERSION,
                run_id: manifest.run_id.clone(),
                case_id: manifest.case_id.clone(),
                source: case.source.clone(),
                suite: manifest.suite,
                graph_pipeline_version: crate::graph::GRAPH_BUILD_PIPELINE_VERSION,
                inventory_hash: manifest.inventory_hash.clone(),
                graph_hash: manifest.graph_hash.clone(),
                failures: manifest
                    .assertions
                    .iter()
                    .filter(|assertion| !assertion.passed)
                    .cloned()
                    .collect(),
                decision_slice: manifest
                    .stages
                    .iter()
                    .flat_map(|stage| stage.decisions.iter().cloned())
                    .collect(),
                reproduce: manifest.reproduce.clone(),
            },
        )?;
        for stage in &manifest.stages {
            JsonStore.write(
                &root
                    .join("stages")
                    .join(format!("{}.json", stage.pass.as_str())),
                stage,
            )?;
        }
        let mut events = String::new();
        for stage in &manifest.stages {
            events.push_str(&serde_json::to_string(
                &json!({"event": "stage", "stage": stage}),
            )?);
            events.push('\n');
        }
        for assertion in &manifest.assertions {
            events.push_str(&serde_json::to_string(
                &json!({"event": "assertion", "assertion": assertion}),
            )?);
            events.push('\n');
        }
        std::fs::write(root.join("events.jsonl"), events)?;
        manifest.observations.insert(
            "persistence_ms".to_owned(),
            millis(persistence_started.elapsed()),
        );
        JsonStore.write(&root.join("manifest.json"), manifest)?;
        Ok(root)
    }

    fn run_scenarios(
        &self,
        case: &CorpusCase,
        source: &Path,
        expectations: &ExpectationSet,
    ) -> Result<Vec<AssertionResult>, LabError> {
        let mut results = Vec::new();
        for scenario in &expectations.scenarios {
            let root = self
                .root
                .join("work/scenarios")
                .join(&case.id)
                .join(&scenario.id);
            if root.exists() {
                std::fs::remove_dir_all(&root)?;
            }
            copy_repository(source, &root)?;
            apply_scenario(&root, &scenario.operation)?;
            let artifacts = RepositoryWalker::new(WalkOptions {
                exclude_globs: case.exclude.clone(),
                ..WalkOptions::default()
            })
            .walk(&root)?;
            let graph = crate::graph::GraphBuilder.build(&root, &artifacts);
            let cache = crate::analysis::AnalysisCache::new(
                self.root
                    .join("work/scenario-cache")
                    .join(&case.id)
                    .join(&scenario.id),
            );
            let source_artifacts = RepositoryWalker::new(WalkOptions {
                exclude_globs: case.exclude.clone(),
                ..WalkOptions::default()
            })
            .walk(source)?;
            let _seed = crate::graph::GraphBuilder.build_with_cache(
                source,
                &source_artifacts,
                Some(&cache),
            );
            let incremental =
                crate::graph::GraphBuilder.build_with_cache(&root, &artifacts, Some(&cache));
            let communities = leiden_communities(&graph, &CommunityScope::Combined);
            let scenario_results = if scenario.preserve_expectations {
                evaluate(&expectations.expectations, &artifacts, &graph, &communities)
            } else {
                let issues = GraphValidator.validate(&graph, &artifacts);
                vec![AssertionResult {
                    id: "transformed-graph-valid".to_owned(),
                    passed: issues.is_empty(),
                    stage: "finalize".to_owned(),
                    detail: format!(
                        "identity-changing scenario requires a valid transformed graph; issues={issues:?}"
                    ),
                    expected_failure: None,
                }]
            };
            results.extend(scenario_results.into_iter().map(|result| AssertionResult {
                id: format!("scenario:{}:{}", scenario.id, result.id),
                passed: result.passed,
                stage: result.stage,
                detail: format!("scenario `{}`: {}", scenario.id, result.detail),
                expected_failure: result.expected_failure,
            }));
            results.push(AssertionResult {
                id: format!("scenario:{}:incremental-equivalence", scenario.id),
                passed: graph == incremental,
                stage: "finalize".to_owned(),
                detail: format!(
                    "scenario `{}` expected incremental cached build to equal a clean rebuild; equivalent={}, cache_hits={}, cache_misses={}",
                    scenario.id,
                    graph == incremental,
                    cache.hits(),
                    cache.misses()
                ),
                expected_failure: None,
            });
        }
        Ok(results)
    }

    fn generated_parser_robustness(&self) -> Result<AssertionResult, LabError> {
        let root = self.root.join("work/generated-parser-inputs");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;
        for seed in 0..32u32 {
            let content = generated_python(seed);
            let path = root.join(format!("case_{seed}.py"));
            std::fs::write(&path, &content)?;
            let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
            let graph = crate::graph::GraphBuilder.build(&root, &artifacts);
            let issues = GraphValidator.validate(&graph, &artifacts);
            if !issues.is_empty() {
                let minimized = shrink_generated_python(&root, &content, &issues)?;
                return Ok(AssertionResult {
                    id: "generated-parser-robustness".to_owned(),
                    passed: false,
                    stage: "definitions_and_imports".to_owned(),
                    detail: format!(
                        "bounded generated input failed; seed={seed}; minimized_input={minimized:?}; issues={issues:?}"
                    ),
                    expected_failure: None,
                });
            }
            std::fs::remove_file(path)?;
        }
        Ok(AssertionResult {
            id: "generated-parser-robustness".to_owned(),
            passed: true,
            stage: "definitions_and_imports".to_owned(),
            detail: "32 deterministic bounded Python inputs passed; failure reports retain seed and minimized input"
                .to_owned(),
            expected_failure: None,
        })
    }
}

fn minimize_artifacts_preserving_signature(
    source: &Path,
    artifacts: &[Artifact],
    expectations: &ExpectationSet,
    required_signature: &str,
) -> Result<Vec<Artifact>, LabError> {
    let mut retained = artifacts.to_vec();
    let mut chunk_size = retained.len().div_ceil(2).max(1);
    let mut attempts = 0usize;
    while chunk_size > 0 && attempts < 16 && retained.len() > 1 {
        let mut removed_any = false;
        let mut start = 0usize;
        while start < retained.len() && attempts < 16 {
            let end = (start + chunk_size).min(retained.len());
            let mut candidate = retained.clone();
            candidate.drain(start..end);
            attempts += 1;
            if !candidate.is_empty()
                && artifact_failure_signature(source, &candidate, expectations)?.as_deref()
                    == Some(required_signature)
            {
                retained = candidate;
                removed_any = true;
                break;
            }
            start = end;
        }
        if !removed_any {
            if chunk_size == 1 {
                break;
            }
            chunk_size = chunk_size.div_ceil(2);
        }
    }
    Ok(retained)
}

fn artifact_failure_signature(
    source: &Path,
    artifacts: &[Artifact],
    expectations: &ExpectationSet,
) -> Result<Option<String>, LabError> {
    let graph = crate::graph::GraphBuilder.build(source, artifacts);
    let communities = leiden_communities(&graph, &CommunityScope::Combined);
    let mut assertions = evaluate(&expectations.expectations, artifacts, &graph, &communities);
    apply_known_failures(&mut assertions, &expectations.known_failures)?;
    let failures = assertions
        .into_iter()
        .filter(|assertion| !assertion.passed)
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(None)
    } else {
        Ok(Some(hash_json(&failures)?))
    }
}

/// Compares a baseline and run without filesystem access.
pub fn diff(baseline: &BaselineRecord, run: &RunManifest) -> BaselineDiff {
    let mut changes = Vec::new();
    let mut first_divergent_stage = None;
    let mut observed_stages = std::collections::BTreeSet::new();
    for candidate_stage in &run.stages {
        let stage = candidate_stage.pass.as_str().to_owned();
        observed_stages.insert(stage.clone());
        let accepted = baseline
            .stage_hashes
            .get(&stage)
            .cloned()
            .unwrap_or_default();
        if candidate_stage.graph_hash != accepted {
            first_divergent_stage.get_or_insert_with(|| stage.clone());
            changes.push(BaselineChange {
                key: format!("stage:{stage}"),
                baseline: accepted,
                candidate: candidate_stage.graph_hash.clone(),
            });
        }
    }
    for (stage, accepted) in &baseline.stage_hashes {
        if !observed_stages.contains(stage) {
            first_divergent_stage.get_or_insert_with(|| stage.clone());
            changes.push(BaselineChange {
                key: format!("stage:{stage}"),
                baseline: accepted.clone(),
                candidate: String::new(),
            });
        }
    }
    if baseline.graph_hash != run.graph_hash {
        changes.push(BaselineChange {
            key: "graph_hash".to_owned(),
            baseline: baseline.graph_hash.clone(),
            candidate: run.graph_hash.clone(),
        });
    }
    let candidate_assertions = run
        .assertions
        .iter()
        .map(|result| (result.id.clone(), result.is_accepted()))
        .collect::<BTreeMap<_, _>>();
    for (id, accepted) in &baseline.assertions {
        let candidate = candidate_assertions.get(id).copied().unwrap_or(false);
        if candidate != *accepted {
            changes.push(BaselineChange {
                key: format!("assertion:{id}"),
                baseline: accepted.to_string(),
                candidate: candidate.to_string(),
            });
        }
    }
    for (id, candidate) in &candidate_assertions {
        if !baseline.assertions.contains_key(id) {
            changes.push(BaselineChange {
                key: format!("assertion:{id}"),
                baseline: String::new(),
                candidate: candidate.to_string(),
            });
        }
    }
    BaselineDiff {
        case_id: run.case_id.clone(),
        first_divergent_stage,
        changes,
    }
}

fn baseline_from_run(
    run: &RunManifest,
    reason: &str,
    previous: Option<&BaselineRecord>,
) -> BaselineRecord {
    BaselineRecord {
        schema_version: LAB_SCHEMA_VERSION,
        case_id: run.case_id.clone(),
        source_revision: run.source_revision.clone(),
        graph_hash: run.graph_hash.clone(),
        stage_hashes: run
            .stages
            .iter()
            .map(|stage| (stage.pass.as_str().to_owned(), stage.graph_hash.clone()))
            .collect(),
        assertions: run
            .assertions
            .iter()
            .map(|result| (result.id.clone(), result.is_accepted()))
            .collect(),
        reason: reason.trim().to_owned(),
        previous_graph_hash: previous.map(|baseline| baseline.graph_hash.clone()),
        previous_reason: previous.map(|baseline| baseline.reason.clone()),
    }
}

fn empty_baseline(run: &RunManifest) -> BaselineRecord {
    BaselineRecord {
        schema_version: LAB_SCHEMA_VERSION,
        case_id: run.case_id.clone(),
        source_revision: String::new(),
        graph_hash: String::new(),
        stage_hashes: BTreeMap::new(),
        assertions: BTreeMap::new(),
        reason: String::new(),
        previous_graph_hash: None,
        previous_reason: None,
    }
}

fn atomic_json_write<T: Serialize>(path: &Path, value: &T) -> Result<(), LabError> {
    let parent = path.parent().ok_or_else(|| {
        LabError::Invalid(format!("baseline path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent)?;
    let temporary = path.with_extension("json.accepting");
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    std::fs::write(&temporary, bytes)?;
    std::fs::rename(temporary, path)?;
    Ok(())
}

fn evaluate(
    expectations: &[Expectation],
    artifacts: &[Artifact],
    graph: &Graph,
    communities: &[CommunitySummary],
) -> Vec<AssertionResult> {
    let issues = GraphValidator.validate(graph, artifacts);
    expectations
        .iter()
        .map(|expectation| match expectation {
            Expectation::GraphValid { id } => AssertionResult {
                id: id.clone(),
                passed: issues.is_empty(),
                stage: "finalize".to_owned(),
                detail: if issues.is_empty() {
                    "expected a valid graph; observed no invariant issues".to_owned()
                } else {
                    format!("expected a valid graph; observed {issues:?}")
                },
                expected_failure: None,
            },
            Expectation::Artifact {
                id,
                path,
                category,
                format,
            } => {
                let observed = artifacts.iter().find(|artifact| artifact.path.as_str() == path);
                let passed = observed.is_some_and(|artifact| {
                    artifact.category == *category && artifact.detected_format == *format
                });
                AssertionResult {
                    id: id.clone(),
                    passed,
                    stage: "inventory".to_owned(),
                    detail: format!(
                        "expected {path} => {category:?}/{format:?}; observed {}",
                        observed.map_or("missing".to_owned(), |artifact| format!(
                            "{:?}/{:?}", artifact.category, artifact.detected_format
                        ))
                    ),
                    expected_failure: None,
                }
            }
            Expectation::ArtifactAbsent { id, path } => {
                let found = artifacts
                    .iter()
                    .any(|artifact| artifact.path.as_str() == path);
                AssertionResult {
                    id: id.clone(),
                    passed: !found,
                    stage: "inventory".to_owned(),
                    detail: format!("expected {path} to be absent; observed present={found}"),
                    expected_failure: None,
                }
            }
            Expectation::Relation {
                id,
                source_contains,
                target_contains,
                relation,
                present,
            } => {
                let found = graph.relations.iter().any(|edge| {
                    edge.kind == *relation
                        && edge.source.as_str().contains(source_contains)
                        && edge.target.as_str().contains(target_contains)
                });
                AssertionResult {
                    id: id.clone(),
                    passed: found == *present,
                    stage: relation_stage(*relation).to_owned(),
                    detail: format!(
                        "expected {relation:?} {source_contains} -> {target_contains} present={present}; observed present={found}"
                    ),
                    expected_failure: None,
                }
            }
            Expectation::ClonePair {
                id,
                left_contains,
                right_contains,
                similar,
            } => {
                let found = graph.relations.iter().any(|edge| {
                    edge.kind == RelationKind::SimilarTo
                        && ((edge.source.as_str().contains(left_contains)
                            && edge.target.as_str().contains(right_contains))
                            || (edge.source.as_str().contains(right_contains)
                                && edge.target.as_str().contains(left_contains)))
                });
                AssertionResult {
                    id: id.clone(),
                    passed: found == *similar,
                    stage: "enrichment".to_owned(),
                    detail: format!(
                        "expected clone {left_contains} <-> {right_contains} similar={similar}; observed similar={found}"
                    ),
                    expected_failure: None,
                }
            }
            Expectation::CommunityPair {
                id,
                left_contains,
                right_contains,
                together,
            } => {
                let found = communities.iter().any(|community| {
                    community
                        .members
                        .iter()
                        .any(|member| member.as_str().contains(left_contains))
                        && community
                            .members
                            .iter()
                            .any(|member| member.as_str().contains(right_contains))
                });
                AssertionResult {
                    id: id.clone(),
                    passed: found == *together,
                    stage: "analytics".to_owned(),
                    detail: format!(
                        "expected community pair {left_contains}/{right_contains} together={together}; observed together={found}"
                    ),
                    expected_failure: None,
                }
            }
            Expectation::SemanticRank {
                id,
                query,
                node_contains,
                max_rank,
            } => {
                let matches = filter_classes(graph, query);
                let rank = matches
                    .iter()
                    .position(|item| item.profile.node_id.as_str().contains(node_contains))
                    .map(|index| index + 1);
                AssertionResult {
                    id: id.clone(),
                    passed: rank.is_some_and(|value| value <= *max_rank),
                    stage: "analytics".to_owned(),
                    detail: format!(
                        "expected semantic query `{query}` to rank `{node_contains}` <= {max_rank}; observed rank={rank:?} with scores={:?}",
                        matches
                            .iter()
                            .take(5)
                            .map(|item| (item.profile.node_id.as_str(), item.score.total()))
                            .collect::<Vec<_>>()
                    ),
                    expected_failure: None,
                }
            }
        })
        .collect()
}

fn failed_trace_selectors(
    expectations: &[Expectation],
    assertions: &[AssertionResult],
    artifacts: &[Artifact],
    graph: &Graph,
) -> Vec<String> {
    let failed = assertions
        .iter()
        .filter(|assertion| !assertion.passed)
        .map(|assertion| assertion.id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut selectors = Vec::new();
    for expectation in expectations {
        if !failed.contains(expectation.id()) {
            continue;
        }
        match expectation {
            Expectation::Artifact { path, .. } | Expectation::ArtifactAbsent { path, .. } => {
                selectors.push(path.clone());
            }
            Expectation::Relation {
                source_contains,
                target_contains,
                ..
            } => {
                selectors.push(source_contains.clone());
                selectors.push(target_contains.clone());
            }
            Expectation::CommunityPair {
                left_contains,
                right_contains,
                ..
            }
            | Expectation::ClonePair {
                left_contains,
                right_contains,
                ..
            } => {
                selectors.push(left_contains.clone());
                selectors.push(right_contains.clone());
            }
            Expectation::SemanticRank { node_contains, .. } => {
                selectors.push(node_contains.clone());
            }
            Expectation::GraphValid { .. } => {
                let issue_text = format!("{:?}", GraphValidator.validate(graph, artifacts));
                selectors.extend(
                    artifacts
                        .iter()
                        .filter(|artifact| issue_text.contains(artifact.path.as_str()))
                        .map(|artifact| artifact.path.as_str().to_owned()),
                );
            }
        }
    }
    selectors.sort();
    selectors.dedup();
    selectors
}

#[derive(Debug, Clone, Copy)]
struct CommunityScopeComparison {
    ari: f64,
    nmi: f64,
    pair_accuracy: f64,
    mean_cohesion: f64,
    mean_conductance: f64,
}

fn compare_community_scopes(
    graph: &Graph,
    expectations: &[Expectation],
    baseline: &[CommunitySummary],
    candidate: &[CommunitySummary],
) -> CommunityScopeComparison {
    let node_ids: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| node.id().clone())
        .chain(
            graph
                .relations
                .iter()
                .flat_map(|edge| [edge.source.clone(), edge.target.clone()]),
        )
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let assignments = |communities: &[CommunitySummary]| {
        let labels: BTreeMap<_, _> = communities
            .iter()
            .enumerate()
            .flat_map(|(label, community)| {
                community
                    .members
                    .iter()
                    .cloned()
                    .map(move |member| (member, label))
            })
            .collect();
        node_ids
            .iter()
            .enumerate()
            .map(|(index, node)| {
                labels
                    .get(node)
                    .copied()
                    .unwrap_or(communities.len() + index)
            })
            .collect::<Vec<_>>()
    };
    let baseline_assignments = assignments(baseline);
    let candidate_assignments = assignments(candidate);
    let ari =
        crate::lab::metrics::adjusted_rand_index(&baseline_assignments, &candidate_assignments)
            .unwrap_or(1.0);
    let nmi = crate::lab::metrics::normalized_mutual_information(
        &baseline_assignments,
        &candidate_assignments,
    )
    .unwrap_or(1.0);
    let pair_results: Vec<bool> = expectations
        .iter()
        .filter_map(|expectation| match expectation {
            Expectation::CommunityPair {
                left_contains,
                right_contains,
                together,
                ..
            } => {
                let observed_together = candidate.iter().any(|community| {
                    community
                        .members
                        .iter()
                        .any(|member| member.as_str().contains(left_contains))
                        && community
                            .members
                            .iter()
                            .any(|member| member.as_str().contains(right_contains))
                });
                Some(observed_together == *together)
            }
            _ => None,
        })
        .collect();
    let pair_accuracy = if pair_results.is_empty() {
        1.0
    } else {
        pair_results.iter().filter(|passed| **passed).count() as f64 / pair_results.len() as f64
    };
    let mean = |values: Vec<f64>, empty: f64| {
        if values.is_empty() {
            empty
        } else {
            values.iter().sum::<f64>() / values.len() as f64
        }
    };
    CommunityScopeComparison {
        ari,
        nmi,
        pair_accuracy,
        mean_cohesion: mean(
            candidate
                .iter()
                .map(|community| community.cohesion)
                .collect(),
            1.0,
        ),
        mean_conductance: mean(
            candidate
                .iter()
                .map(|community| community.conductance)
                .collect(),
            0.0,
        ),
    }
}

fn millionths(value: f64) -> u64 {
    (value.clamp(0.0, 1.0) * 1_000_000.0).round() as u64
}

fn derive_metrics(
    expectations: &[Expectation],
    assertions: &[AssertionResult],
    graph: &Graph,
) -> Vec<MetricResult> {
    let result_by_id = assertions
        .iter()
        .map(|result| (result.id.as_str(), result))
        .collect::<BTreeMap<_, _>>();
    let mut clone_confusion = ConfusionMatrix::default();
    let mut relation_confusion = ConfusionMatrix::default();
    let mut artifact_confusion = ConfusionMatrix::default();
    let mut expected_clusters = Vec::new();
    let mut observed_clusters = Vec::new();
    let mut semantic_ranks = Vec::new();
    let mut semantic_ndcg = Vec::new();
    for expectation in expectations {
        match expectation {
            Expectation::ClonePair { id, similar, .. } => {
                let passed = result_by_id[id.as_str()].is_accepted();
                match (*similar, passed) {
                    (true, true) => clone_confusion.true_positive += 1,
                    (true, false) => clone_confusion.false_negative += 1,
                    (false, true) => clone_confusion.true_negative += 1,
                    (false, false) => clone_confusion.false_positive += 1,
                }
            }
            Expectation::Relation { id, present, .. } => {
                update_confusion(
                    &mut relation_confusion,
                    *present,
                    result_by_id[id.as_str()].is_accepted(),
                );
            }
            Expectation::Artifact { id, .. } => {
                update_confusion(
                    &mut artifact_confusion,
                    true,
                    result_by_id[id.as_str()].is_accepted(),
                );
            }
            Expectation::ArtifactAbsent { id, .. } => {
                update_confusion(
                    &mut artifact_confusion,
                    false,
                    result_by_id[id.as_str()].is_accepted(),
                );
            }
            Expectation::CommunityPair { id, together, .. } => {
                expected_clusters.push(usize::from(*together));
                let observed = if result_by_id[id.as_str()].is_accepted() {
                    *together
                } else {
                    !*together
                };
                observed_clusters.push(usize::from(observed));
            }
            Expectation::SemanticRank {
                query,
                node_contains,
                ..
            } => {
                let matches = filter_classes(graph, query);
                let rank = matches
                    .iter()
                    .position(|item| item.profile.node_id.as_str().contains(node_contains))
                    .map(|index| index + 1);
                semantic_ranks.push(rank);
                semantic_ndcg.push(crate::lab::metrics::ndcg(
                    &matches
                        .iter()
                        .map(|item| item.profile.node_id.as_str().contains(node_contains))
                        .collect::<Vec<_>>(),
                ));
            }
            _ => {}
        }
    }
    let accuracy = if assertions.is_empty() {
        1.0
    } else {
        assertions
            .iter()
            .filter(|result| result.is_accepted())
            .count() as f64
            / assertions.len() as f64
    };
    let unresolved = graph
        .nodes
        .iter()
        .filter(|node| matches!(node, GraphNode::Unresolved(_)))
        .count();
    let unresolved_rate = if graph.nodes.is_empty() {
        0.0
    } else {
        unresolved as f64 / graph.nodes.len() as f64
    };
    vec![
        metric("assertion_accuracy", accuracy, Some(1.0)),
        metric("clone_precision", clone_confusion.precision(), Some(1.0)),
        metric("clone_recall", clone_confusion.recall(), Some(1.0)),
        metric(
            "relation_precision",
            relation_confusion.precision(),
            Some(1.0),
        ),
        metric("relation_recall", relation_confusion.recall(), Some(1.0)),
        metric(
            "artifact_precision",
            artifact_confusion.precision(),
            Some(1.0),
        ),
        metric("artifact_recall", artifact_confusion.recall(), Some(1.0)),
        metric(
            "cluster_ari",
            crate::lab::metrics::adjusted_rand_index(&expected_clusters, &observed_clusters)
                .unwrap_or(1.0),
            Some(1.0),
        ),
        metric(
            "semantic_mrr",
            mean_reciprocal_rank(&semantic_ranks),
            Some(1.0),
        ),
        metric(
            "semantic_ndcg",
            if semantic_ndcg.is_empty() {
                1.0
            } else {
                semantic_ndcg.iter().sum::<f64>() / semantic_ndcg.len() as f64
            },
            Some(1.0),
        ),
        MetricResult {
            name: "unresolved_rate".to_owned(),
            value: unresolved_rate,
            minimum: None,
            passed: true,
        },
    ]
}

fn update_confusion(matrix: &mut ConfusionMatrix, expected: bool, expectation_passed: bool) {
    match (expected, expectation_passed) {
        (true, true) => matrix.true_positive += 1,
        (true, false) => matrix.false_negative += 1,
        (false, true) => matrix.true_negative += 1,
        (false, false) => matrix.false_positive += 1,
    }
}

fn metric(name: &str, value: f64, minimum: Option<f64>) -> MetricResult {
    MetricResult {
        name: name.to_owned(),
        value,
        minimum,
        passed: minimum.is_none_or(|bound| value >= bound),
    }
}

fn relation_stage(kind: RelationKind) -> &'static str {
    match kind {
        RelationKind::SimilarTo | RelationKind::Tests | RelationKind::DocumentsSource => {
            "enrichment"
        }
        RelationKind::Calls
        | RelationKind::Imports
        | RelationKind::Implements
        | RelationKind::Inherits
        | RelationKind::UsesType
        | RelationKind::TypeRefs
        | RelationKind::Usages => "resolution",
        _ => "definitions_and_imports",
    }
}

fn read_required<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LabError> {
    JsonStore
        .read(path)?
        .ok_or_else(|| LabError::Invalid(format!("required file is missing: {}", path.display())))
}

fn read_compatible<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T, LabError> {
    read_optional_compatible(path)?
        .ok_or_else(|| LabError::Invalid(format!("required file is missing: {}", path.display())))
}

fn read_optional_compatible<T: serde::de::DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, LabError> {
    let Some(mut value): Option<Value> = JsonStore.read(path)? else {
        return Ok(None);
    };
    migrate_value(&mut value)?;
    Ok(Some(serde_json::from_value(value)?))
}

fn schema_version(value: &Value) -> Result<u32, LabError> {
    value
        .get("schema_version")
        .and_then(Value::as_u64)
        .and_then(|version| u32::try_from(version).ok())
        .ok_or_else(|| LabError::Invalid("lab JSON has no valid schema_version".to_owned()))
}

fn migrate_value(value: &mut Value) -> Result<Vec<String>, LabError> {
    let version = schema_version(value)?;
    if version > LAB_SCHEMA_VERSION {
        return Err(LabError::Invalid(format!(
            "lab schema {version} is newer than supported schema {LAB_SCHEMA_VERSION}"
        )));
    }
    let mut changes = Vec::new();
    if version < 2 {
        value["schema_version"] = Value::from(2);
        changes.push("schema_version: 1 -> 2".to_owned());
        if value.get("run_id").is_some() && value.get("graph_pipeline_version").is_none() {
            value["graph_pipeline_version"] =
                Value::from(crate::graph::GRAPH_BUILD_PIPELINE_VERSION);
            changes.push(format!(
                "graph_pipeline_version: absent -> {}",
                crate::graph::GRAPH_BUILD_PIPELINE_VERSION
            ));
        }
        if let Some(stage_hashes) = value.get_mut("stage_hashes").and_then(Value::as_object_mut) {
            for (old, new) in [
                ("Structure", "structure"),
                ("DefinitionsAndImports", "definitions_and_imports"),
                ("Enrichment", "enrichment"),
                ("Resolution", "resolution"),
                ("Analytics", "analytics"),
                ("Persistence", "persistence"),
                ("Finalize", "finalize"),
            ] {
                if let Some(hash) = stage_hashes.remove(old) {
                    stage_hashes.insert(new.to_owned(), hash);
                    changes.push(format!("stage_hashes.{old} -> stage_hashes.{new}"));
                }
            }
        }
    }
    Ok(changes)
}

fn hash_json<T: Serialize + ?Sized>(value: &T) -> Result<String, serde_json::Error> {
    Ok(blake3::hash(serde_json::to_vec(value)?.as_slice())
        .to_hex()
        .to_string())
}

fn millis(duration: std::time::Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn process_rss_kib() -> u64 {
    if let Ok(status) = std::fs::read_to_string("/proc/self/status")
        && let Some(value) = status
            .lines()
            .find_map(|line| line.strip_prefix("VmHWM:"))
            .and_then(|line| line.split_whitespace().next())
            .and_then(|value| value.parse().ok())
    {
        return value;
    }
    std::process::Command::new("ps")
        .args(["-o", "rss=", "-p"])
        .arg(std::process::id().to_string())
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0)
}

fn performance_summary(
    runs: &[RunManifest],
    mode: BenchmarkMode,
) -> Result<PerformanceSummary, LabError> {
    let first = runs
        .first()
        .ok_or_else(|| LabError::Invalid("performance summary has no samples".to_owned()))?;
    let mut values = BTreeMap::<String, Vec<f64>>::new();
    for run in runs {
        if run.case_id != first.case_id || run.source_revision != first.source_revision {
            return Err(LabError::Invalid(
                "performance samples do not share one immutable case".to_owned(),
            ));
        }
        for (name, value) in &run.observations {
            values.entry(name.clone()).or_default().push(*value as f64);
        }
    }
    let metrics = values
        .into_iter()
        .map(|(name, mut samples)| {
            samples.sort_by(f64::total_cmp);
            let median_value = median(&samples);
            let mut deviations = samples
                .iter()
                .map(|value| (value - median_value).abs())
                .collect::<Vec<_>>();
            deviations.sort_by(f64::total_cmp);
            RobustMetric {
                name,
                median: median_value,
                mad: median(&deviations),
            }
        })
        .collect();
    Ok(PerformanceSummary {
        schema_version: LAB_SCHEMA_VERSION,
        case_id: first.case_id.clone(),
        source_revision: first.source_revision.clone(),
        machine: machine_fingerprint(),
        samples: runs.len(),
        mode,
        sample_files: Vec::new(),
        history_sequence: 0,
        regressions: Vec::new(),
        metrics,
        graph_hash: None,
        community_scope: None,
        community_algorithm_version: None,
        reproduce: None,
    })
}

fn latest_performance_summary(root: &Path) -> Result<Option<PerformanceSummary>, LabError> {
    if !root.is_dir() {
        return Ok(None);
    }
    let mut summaries = std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter_map(|entry| read_required::<PerformanceSummary>(&entry.path()).ok())
        .collect::<Vec<_>>();
    summaries.sort_by_key(|summary| summary.history_sequence);
    Ok(summaries.pop())
}

fn compare_performance(
    previous: &PerformanceSummary,
    current: &PerformanceSummary,
    budgets: &[PerformanceBudget],
) -> Result<Vec<PerformanceRegression>, LabError> {
    if previous.case_id != current.case_id
        || previous.source_revision != current.source_revision
        || previous.mode != current.mode
    {
        return Err(LabError::Invalid(format!(
            "performance histories cannot be mixed: previous={}/{}/{}, current={}/{}/{}",
            previous.case_id,
            previous.source_revision,
            previous.mode.as_str(),
            current.case_id,
            current.source_revision,
            current.mode.as_str(),
        )));
    }
    let previous_metrics = previous
        .metrics
        .iter()
        .map(|metric| (metric.name.as_str(), metric.median))
        .collect::<BTreeMap<_, _>>();
    let current_metrics = current
        .metrics
        .iter()
        .map(|metric| (metric.name.as_str(), metric.median))
        .collect::<BTreeMap<_, _>>();
    Ok(budgets
        .iter()
        .filter_map(|budget| {
            let previous_median = *previous_metrics.get(budget.metric.as_str())?;
            let current_median = *current_metrics.get(budget.metric.as_str())?;
            let relative_increase = if previous_median == 0.0 {
                f64::from(current_median > 0.0)
            } else {
                (current_median - previous_median) / previous_median
            };
            Some(PerformanceRegression {
                metric: budget.metric.clone(),
                previous_median,
                current_median,
                relative_increase,
                allowed_increase: budget.max_relative_increase,
                passed: relative_increase <= budget.max_relative_increase,
            })
        })
        .collect())
}

fn machine_fingerprint() -> MachineFingerprint {
    MachineFingerprint {
        os: std::env::consts::OS.to_owned(),
        architecture: std::env::consts::ARCH.to_owned(),
        parallelism: std::thread::available_parallelism().map_or(1, usize::from),
    }
}

fn machine_slug(machine: &MachineFingerprint) -> String {
    format!(
        "{}-{}-{}",
        machine.os, machine.architecture, machine.parallelism
    )
}

/// Removes persisted near-clone snapshot files from an analyzer cache dir so
/// the next graph build re-runs full clone detection (LIT-35.5). Best-effort:
/// leaves every non-clone (analyzer) entry untouched so that cache stays warm.
fn clear_clone_snapshots(cache_root: &Path) {
    let Ok(entries) = std::fs::read_dir(cache_root) else {
        return;
    };
    for entry in entries.flatten() {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.contains("-clone-"))
        {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

fn next_sample_sequence(root: &Path) -> Result<usize, LabError> {
    if !root.is_dir() {
        return Ok(0);
    }
    Ok(std::fs::read_dir(root)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|value| value == "json")
        })
        .count())
}

fn median(values: &[f64]) -> f64 {
    let middle = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[middle.saturating_sub(1)] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn copy_repository(source: &Path, destination: &Path) -> Result<(), LabError> {
    std::fs::create_dir_all(destination)?;
    let mut directories = vec![source.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name() == ".git" || entry.file_name() == ".lithograph" {
                continue;
            }
            let target = destination.join(path.strip_prefix(source).map_err(|error| {
                LabError::Invalid(format!("cannot copy scenario repository: {error}"))
            })?);
            if path.is_dir() {
                std::fs::create_dir_all(&target)?;
                directories.push(path);
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(path, target)?;
            }
        }
    }
    Ok(())
}

fn apply_scenario(root: &Path, operation: &ScenarioOperation) -> Result<(), LabError> {
    match operation {
        ScenarioOperation::AppendComment { path, text } => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(root.join(path))?;
            writeln!(file, "{text}")?;
        }
        ScenarioOperation::AddFile { path, content } => {
            let target = root.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, content)?;
        }
        ScenarioOperation::RenameFile { from, to } => {
            let target = root.join(to);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(root.join(from), target)?;
        }
        ScenarioOperation::ReplaceText { path, from, to } => {
            replace_fixture_text(&root.join(path), from, to)?;
        }
        ScenarioOperation::MoveFileAndReplace {
            from,
            to,
            update_path,
            old,
            new,
        } => {
            let target = root.join(to);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(root.join(from), target)?;
            replace_fixture_text(&root.join(update_path), old, new)?;
        }
        ScenarioOperation::PrependText { path, text } => {
            let target = root.join(path);
            let current = std::fs::read_to_string(&target)?;
            std::fs::write(target, format!("{text}{current}"))?;
        }
        ScenarioOperation::RewriteFile { path, content } => {
            std::fs::write(root.join(path), content)?;
        }
    }
    Ok(())
}

fn generated_python(seed: u32) -> String {
    let decorators = if seed.is_multiple_of(3) {
        "@staticmethod\n    "
    } else {
        ""
    };
    let annotation = if seed.is_multiple_of(2) {
        " -> int"
    } else {
        ""
    };
    let body = match seed % 4 {
        0 => "return value + 1",
        1 => "return sum(item for item in value if item)",
        2 => "match value:\n            case 0: return 0\n            case _: return 1",
        _ => {
            "try:\n            return value[0]\n        except (IndexError, TypeError):\n            return None"
        }
    };
    format!(
        "class Generated{seed}:\n    {decorators}def evaluate(value){annotation}:\n        {body}\n"
    )
}

fn shrink_generated_python(
    root: &Path,
    content: &str,
    original: &[crate::graph::GraphIssue],
) -> Result<String, LabError> {
    let signature = original.iter().map(|issue| issue.kind).collect::<Vec<_>>();
    let mut lines = content.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let mut candidate = lines.clone();
        candidate.remove(index);
        let candidate_text = format!("{}\n", candidate.join("\n"));
        let path = root.join("shrink.py");
        std::fs::write(&path, &candidate_text)?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let graph = crate::graph::GraphBuilder.build(root, &artifacts);
        let observed = GraphValidator
            .validate(&graph, &artifacts)
            .iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>();
        std::fs::remove_file(path)?;
        if observed == signature {
            lines = candidate;
        } else {
            index += 1;
        }
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn replace_fixture_text(path: &Path, from: &str, to: &str) -> Result<(), LabError> {
    let current = std::fs::read_to_string(path)?;
    if !current.contains(from) {
        return Err(LabError::Invalid(format!(
            "scenario replacement `{from}` was not found in {}",
            path.display()
        )));
    }
    std::fs::write(path, current.replace(from, to))?;
    Ok(())
}

fn differential_python_definitions(
    repo_root: &Path,
    graph: &Graph,
) -> Result<AssertionResult, LabError> {
    let script = r#"import ast, pathlib, sys
count = 0
for path in pathlib.Path(sys.argv[1]).rglob('*.py'):
    if any(part in {'.git', '.lithograph'} for part in path.parts):
        continue
    try:
        tree = ast.parse(path.read_text(encoding='utf-8'))
    except (OSError, UnicodeDecodeError, SyntaxError):
        continue
    count += sum(isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) for node in ast.walk(tree))
print(count)
"#;
    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(repo_root)
        .output()
        .map_err(|error| {
            LabError::Invalid(format!(
                "python3 is required for the PR differential oracle: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(LabError::Invalid(format!(
            "Python AST differential oracle failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let expected = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| LabError::Invalid(format!("invalid Python AST count: {error}")))?;
    let observed = graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(node, GraphNode::Symbol(symbol) if symbol.evidence.path.as_str().ends_with(".py"))
        })
        .count();
    Ok(AssertionResult {
        id: "differential-python-definitions".to_owned(),
        passed: expected == observed,
        stage: "definitions_and_imports".to_owned(),
        detail: format!(
            "expected Python AST and Lithograph definition counts to match; ast={expected}, graph={observed}"
        ),
        expected_failure: None,
    })
}

fn differential_cargo_metadata(repo_root: &Path, graph: &Graph) -> DifferentialResult {
    if !repo_root.join("Cargo.toml").is_file() {
        return DifferentialResult {
            name: "cargo_metadata_packages".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "no root Cargo.toml was present".to_owned(),
        };
    }
    let mut command = cargo_metadata::MetadataCommand::new();
    command.current_dir(repo_root).no_deps();
    let metadata = match command.exec() {
        Ok(metadata) => metadata,
        Err(error) => {
            return DifferentialResult {
                name: "cargo_metadata_packages".to_owned(),
                status: DifferentialStatus::Skipped,
                detail: format!("cargo metadata unavailable: {error}"),
            };
        }
    };
    let expected = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let observed = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Package(package) if !package.is_external => Some(package.name.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let missing = expected.difference(&observed).copied().collect::<Vec<_>>();
    DifferentialResult {
        name: "cargo_metadata_packages".to_owned(),
        status: if missing.is_empty() {
            DifferentialStatus::Passed
        } else {
            DifferentialStatus::Failed
        },
        detail: format!(
            "cargo metadata packages={}, graph internal packages={}, missing={missing:?}",
            expected.len(),
            observed.len()
        ),
    }
}

fn differential_typescript_compiler(repo_root: &Path, graph: &Graph) -> DifferentialResult {
    if !repo_root.join("package.json").is_file()
        && !repo_root.join("frontend/package.json").is_file()
    {
        return DifferentialResult {
            name: "typescript_imports".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "no TypeScript package root was present".to_owned(),
        };
    }
    let script = r#"let ts; try { ts = require('typescript'); } catch (_) { process.exit(42); }
const files = ts.sys.readDirectory(process.cwd(), ['.ts', '.tsx'], undefined, ['**/*']);
let imports = 0;
for (const file of files) {
  if (file.includes('/node_modules/') || file.includes('/dist/')) continue;
  const source = ts.createSourceFile(file, ts.sys.readFile(file) || '', ts.ScriptTarget.Latest, true);
  for (const statement of source.statements) if (ts.isImportDeclaration(statement)) imports++;
}
process.stdout.write(String(imports));"#;
    let output = match std::process::Command::new("node")
        .args(["-e", script])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return DifferentialResult {
                name: "typescript_imports".to_owned(),
                status: DifferentialStatus::Skipped,
                detail: format!("node unavailable: {error}"),
            };
        }
    };
    if output.status.code() == Some(42) {
        return DifferentialResult {
            name: "typescript_imports".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "the repository has no locally resolvable TypeScript compiler".to_owned(),
        };
    }
    let expected = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>();
    let Ok(expected) = expected else {
        return DifferentialResult {
            name: "typescript_imports".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: format!(
                "TypeScript compiler adapter failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    };
    let observed = graph
        .relations
        .iter()
        .filter(|relation| {
            relation.kind == RelationKind::Imports
                && relation.evidence.iter().any(|evidence| {
                    evidence.path.as_str().ends_with(".ts")
                        || evidence.path.as_str().ends_with(".tsx")
                })
        })
        .count();
    DifferentialResult {
        name: "typescript_imports".to_owned(),
        status: if expected == observed {
            DifferentialStatus::Passed
        } else {
            DifferentialStatus::Failed
        },
        detail: format!("TypeScript compiler imports={expected}, graph imports={observed}"),
    }
}

fn differential_scip(repo_root: &Path, graph: &Graph) -> DifferentialResult {
    let path = repo_root.join(".scip/lithograph-index.json");
    if !path.is_file() {
        return DifferentialResult {
            name: "scip_sentinels".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "optional normalized .scip/lithograph-index.json was not present".to_owned(),
        };
    }
    let value: Value = match std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(value) => value,
        None => {
            return DifferentialResult {
                name: "scip_sentinels".to_owned(),
                status: DifferentialStatus::Failed,
                detail: "normalized SCIP adapter input was invalid JSON".to_owned(),
            };
        }
    };
    let symbols = value
        .get("symbols")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id().as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let missing = symbols
        .iter()
        .filter(|symbol| !node_ids.contains(**symbol))
        .copied()
        .collect::<Vec<_>>();
    DifferentialResult {
        name: "scip_sentinels".to_owned(),
        status: if missing.is_empty() {
            DifferentialStatus::Passed
        } else {
            DifferentialStatus::Failed
        },
        detail: format!(
            "SCIP symbol sentinels={}, missing={missing:?}",
            symbols.len()
        ),
    }
}

fn apply_known_failures(
    assertions: &mut [AssertionResult],
    known_failures: &[KnownFailure],
) -> Result<(), LabError> {
    let today = utc_date();
    for known in known_failures {
        if !valid_date(&known.expires) {
            return Err(LabError::Invalid(format!(
                "known failure `{}` has invalid expiry `{}`",
                known.assertion_id, known.expires
            )));
        }
        let Some(assertion) = assertions
            .iter_mut()
            .find(|result| result.id == known.assertion_id)
        else {
            return Err(LabError::Invalid(format!(
                "known failure references missing assertion `{}`",
                known.assertion_id
            )));
        };
        if assertion.passed || known.expires < today {
            continue;
        }
        let issue_count = assertion.detail.matches("GraphIssue {").count();
        let signatures_match = known.signatures.iter().all(|signature| {
            assertion.detail.matches(&signature.contains).count() == signature.count
        });
        if issue_count == known.issue_count && signatures_match {
            assertion.expected_failure = Some(ExpectedFailureMatch {
                backlog: known.backlog.clone(),
                expires: known.expires.clone(),
            });
        }
    }
    Ok(())
}

fn valid_date(value: &str) -> bool {
    value.len() == 10
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| index == 4 || index == 7 || character.is_ascii_digit())
}

fn utc_date() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs() / 86_400) as i64;
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn diff_reports_first_changed_stage_and_assertion() {
        let run = RunManifest {
            schema_version: LAB_SCHEMA_VERSION,
            graph_pipeline_version: crate::graph::GRAPH_BUILD_PIPELINE_VERSION,
            run_id: "candidate".to_owned(),
            case_id: "fixture".to_owned(),
            suite: SuiteTier::Pr,
            source_revision: "fixture:1".to_owned(),
            inventory_hash: "inventory".to_owned(),
            graph_hash: "new".to_owned(),
            stages: vec![crate::graph::GraphBuildStageTrace {
                pass: crate::graph::GraphBuildPass::Structure,
                node_count: 1,
                relation_count: 0,
                graph_hash: "new-stage".to_owned(),
                duration_us: 0,
                component_durations_us: BTreeMap::new(),
                counters: BTreeMap::new(),
                decisions: Vec::new(),
                graph: None,
            }],
            assertions: vec![AssertionResult {
                id: "truth".to_owned(),
                passed: false,
                stage: "structure".to_owned(),
                detail: "seeded failure".to_owned(),
                expected_failure: None,
            }],
            metrics: Vec::new(),
            differentials: Vec::new(),
            observations: BTreeMap::new(),
            reproduce: "replay".to_owned(),
        };
        let baseline = BaselineRecord {
            schema_version: LAB_SCHEMA_VERSION,
            case_id: "fixture".to_owned(),
            source_revision: "fixture:1".to_owned(),
            graph_hash: "old".to_owned(),
            stage_hashes: BTreeMap::from([("structure".to_owned(), "old-stage".to_owned())]),
            assertions: BTreeMap::from([("truth".to_owned(), true)]),
            reason: "reviewed".to_owned(),
            previous_graph_hash: None,
            previous_reason: None,
        };
        let result = diff(&baseline, &run);
        assert_eq!(result.first_divergent_stage.as_deref(), Some("structure"));
        assert_eq!(result.changes.len(), 3);
    }

    #[test]
    fn acceptance_requires_reason_and_rejects_failed_runs() {
        let temp = tempfile::TempDir::new().unwrap_or_else(|_| unreachable!());
        let manifest_path = temp.path().join("lab/corpus.toml");
        std::fs::create_dir_all(manifest_path.parent().unwrap_or(temp.path()))
            .unwrap_or_else(|_| unreachable!());
        std::fs::write(&manifest_path, "schema_version = 2\ncases = []\n")
            .unwrap_or_else(|_| unreachable!());
        let corpus = Corpus::load(&manifest_path, &temp.path().join("cache"))
            .unwrap_or_else(|_| unreachable!());
        let lab = Lab::new(corpus, temp.path().join("out"));
        let run = RunManifest {
            schema_version: LAB_SCHEMA_VERSION,
            graph_pipeline_version: crate::graph::GRAPH_BUILD_PIPELINE_VERSION,
            run_id: "bad".to_owned(),
            case_id: "fixture".to_owned(),
            suite: SuiteTier::Pr,
            source_revision: "fixture:1".to_owned(),
            inventory_hash: String::new(),
            graph_hash: String::new(),
            stages: Vec::new(),
            assertions: vec![AssertionResult {
                id: "failed".to_owned(),
                passed: false,
                stage: "finalize".to_owned(),
                detail: String::new(),
                expected_failure: None,
            }],
            metrics: Vec::new(),
            differentials: Vec::new(),
            observations: BTreeMap::new(),
            reproduce: String::new(),
        };
        assert!(lab.accept_with_policy(&run, "", "missing", false).is_err());
        assert!(
            lab.accept_with_policy(&run, "reviewed", "missing", false)
                .is_err()
        );
    }

    #[test]
    fn acceptance_token_is_bound_to_reason_and_current_baseline()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let lab_dir = temp.path().join("lab");
        std::fs::create_dir_all(lab_dir.join("baselines"))?;
        std::fs::write(
            lab_dir.join("corpus.toml"),
            "schema_version = 2\ncases = []\n",
        )?;
        let corpus = Corpus::load(&lab_dir.join("corpus.toml"), &temp.path().join("cache"))?;
        let lab = Lab::new(corpus, temp.path().join("out"));
        let run = RunManifest {
            schema_version: LAB_SCHEMA_VERSION,
            graph_pipeline_version: crate::graph::GRAPH_BUILD_PIPELINE_VERSION,
            run_id: "reviewed-run".to_owned(),
            case_id: "fixture".to_owned(),
            suite: SuiteTier::Pr,
            source_revision: "fixture:1".to_owned(),
            inventory_hash: String::new(),
            graph_hash: "graph".to_owned(),
            stages: Vec::new(),
            assertions: Vec::new(),
            metrics: Vec::new(),
            differentials: Vec::new(),
            observations: BTreeMap::new(),
            reproduce: String::new(),
        };
        let review = lab.acceptance_review(&run, "first review")?;
        assert!(
            lab.accept_with_policy(&run, "changed reason", &review.confirmation_token, false)
                .is_err()
        );
        let accepted =
            lab.accept_with_policy(&run, "first review", &review.confirmation_token, false)?;
        assert_eq!(accepted.reason, "first review");
        let stale = lab.acceptance_review(&run, "second review")?;
        let replacement =
            lab.accept_with_policy(&run, "second review", &stale.confirmation_token, false)?;
        assert_eq!(replacement.previous_graph_hash.as_deref(), Some("graph"));
        assert_eq!(replacement.previous_reason.as_deref(), Some("first review"));
        Ok(())
    }

    #[test]
    fn migration_previews_v1_without_semantic_acceptance() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        let lab_dir = temp.path().join("lab");
        std::fs::create_dir_all(&lab_dir)?;
        std::fs::write(
            lab_dir.join("corpus.toml"),
            "schema_version = 2\ncases = []\n",
        )?;
        let artifact = temp.path().join("baseline.json");
        std::fs::write(
            &artifact,
            r#"{"schema_version":1,"stage_hashes":{"Structure":"abc"},"reason":"reviewed"}"#,
        )?;
        let corpus = Corpus::load(&lab_dir.join("corpus.toml"), &temp.path().join("cache"))?;
        let lab = Lab::new(corpus, temp.path().join("out"));
        let preview = lab.migrate(&artifact, false)?;
        assert!(!preview.applied);
        assert!(
            preview
                .changes
                .iter()
                .any(|change| change.contains("stage_hashes"))
        );
        assert!(std::fs::read_to_string(&artifact)?.contains("\"schema_version\":1"));
        let applied = lab.migrate(&artifact, true)?;
        assert!(applied.applied);
        let value: Value = serde_json::from_str(&std::fs::read_to_string(artifact)?)?;
        assert_eq!(value["schema_version"], 2);
        assert_eq!(value["reason"], "reviewed");
        Ok(())
    }

    #[test]
    fn run_persists_replay_explain_accept_and_check_artifacts()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let project = temp.path().join("project");
        let lab_dir = project.join("lab");
        let fixture = project.join("fixture");
        std::fs::create_dir_all(&lab_dir)?;
        std::fs::create_dir_all(&fixture)?;
        std::fs::write(fixture.join("lib.rs"), "pub fn answer() -> u32 { 42 }\n")?;
        std::fs::write(
            lab_dir.join("corpus.toml"),
            "schema_version = 2\n[[cases]]\nid = \"fixture\"\ntier = \"merge\"\nsource = \"fixture\"\npath = \"fixture\"\nlicense = \"MIT\"\nexpectations = \"fixture.json\"\n",
        )?;
        std::fs::write(
            lab_dir.join("fixture.json"),
            "{\"schema_version\":2,\"expectations\":[{\"kind\":\"graph_valid\",\"id\":\"valid\"}]}\n",
        )?;
        let corpus = Corpus::load(&lab_dir.join("corpus.toml"), &temp.path().join("cache"))?;
        let lab = Lab::new(corpus, temp.path().join("out"));
        let run_dir = lab.run(SuiteTier::Merge, Some("fixture"))?.remove(0);
        let run = lab.load_run(&run_dir)?;
        assert!(run.is_clean());
        for observation in [
            "community_participating_nodes",
            "community_selected_edges",
            "community_iterations",
            "community_nodes_reconsidered",
            "community_successful_moves",
            "community_neighbour_label_evaluations",
            "community_adjacency_us",
            "community_movement_us",
            "community_summary_us",
        ] {
            assert!(run.observations.contains_key(observation), "{observation}");
        }
        assert!(run.reproduce.starts_with("just baseline-replay "));
        assert!(run_dir.join("replay.json").is_file());
        assert_eq!(lab.replay(&run)?, run_dir);
        assert_eq!(lab.explain(&run, "valid")?["assertion"]["passed"], true);
        let review = lab.acceptance_review(&run, "reviewed fixture")?;
        lab.accept_with_policy(&run, "reviewed fixture", &review.confirmation_token, false)?;
        assert!(lab.check(&run)?.is_clean());
        assert!(
            lab.accept_with_policy(&run, "reviewed fixture", &review.confirmation_token, true)
                .is_err()
        );
        let mut failed = run.clone();
        failed.assertions[0].passed = false;
        failed.assertions[0].detail = "seeded failure in lib.rs".to_owned();
        let source_free = lab.minimize(&failed, None)?;
        assert_eq!(source_free.relevant_files, vec!["lib.rs"]);
        assert!(source_free.materialized_at.is_none());
        let local_fixture = temp.path().join("minimized-local");
        let materialized = lab.minimize(&failed, Some(&local_fixture))?;
        assert_eq!(
            materialized.failure_signature,
            source_free.failure_signature
        );
        assert!(local_fixture.join("lib.rs").is_file());
        Ok(())
    }

    #[test]
    fn utc_date_is_lexically_comparable() {
        let date = utc_date();
        assert!(valid_date(&date));
        assert!(date.as_str() >= "2020-01-01");
    }

    #[test]
    fn robust_performance_summary_uses_median_and_mad() {
        let make_run = |graph_ms| RunManifest {
            schema_version: LAB_SCHEMA_VERSION,
            graph_pipeline_version: crate::graph::GRAPH_BUILD_PIPELINE_VERSION,
            run_id: "same".to_owned(),
            case_id: "fixture".to_owned(),
            suite: SuiteTier::Nightly,
            source_revision: "commit".to_owned(),
            inventory_hash: String::new(),
            graph_hash: String::new(),
            stages: Vec::new(),
            assertions: Vec::new(),
            metrics: Vec::new(),
            differentials: Vec::new(),
            observations: BTreeMap::from([("graph_ms".to_owned(), graph_ms)]),
            reproduce: String::new(),
        };
        let summary = performance_summary(
            &[
                make_run(10),
                make_run(11),
                make_run(12),
                make_run(13),
                make_run(1000),
            ],
            BenchmarkMode::WarmCache,
        )
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(summary.metrics[0].median, 12.0);
        assert_eq!(summary.metrics[0].mad, 1.0);
    }

    #[test]
    fn reviewed_performance_budget_detects_relative_regression() {
        let summary = |median| PerformanceSummary {
            schema_version: LAB_SCHEMA_VERSION,
            case_id: "fixture".to_owned(),
            source_revision: "commit".to_owned(),
            machine: machine_fingerprint(),
            samples: 5,
            mode: BenchmarkMode::WarmCache,
            sample_files: Vec::new(),
            history_sequence: 0,
            regressions: Vec::new(),
            metrics: vec![RobustMetric {
                name: "graph_ms".to_owned(),
                median,
                mad: 1.0,
            }],
            graph_hash: None,
            community_scope: None,
            community_algorithm_version: None,
            reproduce: None,
        };
        let regressions = compare_performance(
            &summary(100.0),
            &summary(130.0),
            &[PerformanceBudget {
                case_id: "fixture".to_owned(),
                mode: BenchmarkMode::WarmCache,
                metric: "graph_ms".to_owned(),
                max_relative_increase: 0.2,
            }],
        )
        .unwrap_or_else(|_| unreachable!());
        assert_eq!(regressions.len(), 1);
        assert!(!regressions[0].passed);
        assert!((regressions[0].relative_increase - 0.3).abs() < f64::EPSILON);
        let mut focused = summary(100.0);
        focused.mode = BenchmarkMode::CommunityOnly;
        assert!(compare_performance(&summary(100.0), &focused, &[]).is_err());
    }

    #[test]
    fn community_only_benchmark_replays_verified_graph_without_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let project = temp.path().join("project");
        let lab_dir = project.join("lab");
        let fixture = project.join("fixture");
        std::fs::create_dir_all(&lab_dir)?;
        std::fs::create_dir_all(&fixture)?;
        std::fs::write(fixture.join("lib.rs"), "pub fn answer() -> u32 { 42 }\n")?;
        std::fs::write(
            lab_dir.join("corpus.toml"),
            "schema_version = 2\n[[cases]]\nid = \"fixture\"\ntier = \"nightly\"\nsource = \"fixture\"\npath = \"fixture\"\nlicense = \"MIT\"\nexpectations = \"fixture.json\"\n",
        )?;
        std::fs::write(
            lab_dir.join("fixture.json"),
            "{\"schema_version\":2,\"expectations\":[{\"kind\":\"graph_valid\",\"id\":\"valid\"}]}\n",
        )?;
        std::fs::write(
            lab_dir.join("performance-budgets.json"),
            "{\"schema_version\":2,\"budgets\":[]}",
        )?;
        let corpus = Corpus::load(&lab_dir.join("corpus.toml"), &temp.path().join("cache"))?;
        let lab = Lab::new(corpus, temp.path().join("out"));
        let run_dir = lab.run(SuiteTier::Nightly, Some("fixture"))?.remove(0);
        let run = lab.load_run(&run_dir)?;
        let review = lab.acceptance_review(&run, "community replay fixture")?;
        lab.accept_with_policy(
            &run,
            "community replay fixture",
            &review.confirmation_token,
            false,
        )?;
        std::fs::remove_dir_all(&fixture)?;

        let summary = lab
            .benchmark(
                SuiteTier::Nightly,
                Some("fixture"),
                5,
                BenchmarkMode::CommunityOnly,
                false,
            )?
            .remove(0);
        assert_eq!(summary.samples, 5);
        assert_eq!(summary.mode, BenchmarkMode::CommunityOnly);
        assert_eq!(summary.graph_hash.as_deref(), Some(run.graph_hash.as_str()));
        assert_eq!(
            summary.community_algorithm_version,
            Some(crate::graph::LEIDEN_ALGORITHM_VERSION)
        );
        assert_eq!(summary.sample_files.len(), 5);
        assert!(summary.reproduce.as_deref().is_some_and(|command| {
            command.contains("--mode community-only") && command.contains("--case fixture")
        }));
        assert!(
            summary
                .metrics
                .iter()
                .any(|metric| { metric.name == "community_summary_us" && metric.median >= 0.0 })
        );
        Ok(())
    }
}
