//! Baseline governance: comparing a run against its accepted case baseline,
//! and the reviewed, token-bound acceptance flow that is the only way to
//! update one. Acceptance always rejects dirty runs and CI writes, binds a
//! confirmation token to the run/reason/current-baseline/diff so stale or
//! blind acceptance is rejected, and writes atomically while retaining prior
//! review metadata.

use super::{
    Lab, LabError, atomic_json_write, hash_json, read_compatible, read_optional_compatible,
};
use crate::lab::model::{
    AcceptanceReview, BaselineChange, BaselineDiff, BaselineRecord, LAB_SCHEMA_VERSION, RunManifest,
};
use serde_json::json;
use std::collections::BTreeMap;

impl Lab {
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

    /// `accept` with an explicit CI policy flag so tests do not depend on the
    /// process environment. Also called directly by cross-module integration
    /// tests, hence `pub(super)`.
    pub(super) fn accept_with_policy(
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
}

/// Compares a baseline and run without filesystem access.
pub(super) fn diff(baseline: &BaselineRecord, run: &RunManifest) -> BaselineDiff {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lab::corpus::Corpus;
    use crate::lab::model::{AssertionResult, RunManifest, SuiteTier};

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
}
