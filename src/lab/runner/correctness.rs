//! The correctness suite: builds one case's graph with a full trace, runs
//! every analysis phase the graph explorer and MCP surfaces depend on,
//! evaluates expectations plus PR-only differential/mutation extras, applies
//! known-failure suppressions with an exact signature and expiry, derives
//! aggregate metrics, and persists the content-addressed run. This is what
//! `just baseline-pr`/`baseline-merge`/`baseline-nightly` execute per case.

use super::{
    Lab, LabError, compare_community_scopes, derive_metrics, differential_cargo_metadata,
    differential_python_definitions, differential_scip, differential_typescript_compiler, evaluate,
    failed_trace_selectors, hash_json, millionths, process_rss_kib, read_required,
};
use crate::domain::Artifact;
use crate::graph::{
    CommunitySnapshotStore, CommunitySummary, Graph, GraphBuildTraceConfig, GraphBuildTraceDetail,
    GraphValidator, analyze_communities, architecture_aware_scope, environment_aware_scope,
    filter_classes,
};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::lab::model::*;
use crate::storage::JsonStore;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

impl Lab {
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

    /// Also called from the performance benchmark to reuse the exact
    /// correctness execution path under warm/cold analyzer caches.
    pub(super) fn run_case_with_cache(
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
            &architecture_aware_scope(),
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
}

pub(super) fn millis(duration: std::time::Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

/// Applies known-failure suppressions in place. A suppression only takes
/// effect while unexpired and while its exact issue count and signature
/// substrings still match, so a defect signature drifting silently reverts
/// to a hard failure instead of masking a new regression.
pub(super) fn apply_known_failures(
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

/// Returns today's UTC date as `YYYY-MM-DD` via a dependency-free civil-date
/// computation (days since the epoch -> proleptic Gregorian calendar).
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
    use crate::lab::corpus::Corpus;

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
}
