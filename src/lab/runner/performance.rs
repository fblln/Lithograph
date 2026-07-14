//! Performance benchmarking: repeated warm samples reduced to robust
//! median/MAD summaries, stored separately from deterministic correctness
//! baselines and gated only by reviewed relative budgets. `community-only`
//! mode instead replays a verified persisted graph and measures only the
//! community adjacency/movement/summary phases, clearing snapshot caches so
//! samples measure the implementation rather than a cache hit.

use super::{Lab, LabError, hash_json, process_rss_kib, read_compatible, read_required};
use crate::graph::{CommunityScope, CommunitySummary, Graph};
use crate::lab::model::*;
use crate::storage::JsonStore;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

impl Lab {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::corpus::Corpus;

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
