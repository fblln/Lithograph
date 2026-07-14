//! Failure minimization: produces a source-free slice of exactly the
//! artifacts, nodes, and relations relevant to a run's failing assertions,
//! shrinking the artifact set by bisection while requiring the failure
//! signature (the hashed set of failing assertions) stay identical, and
//! optionally materializes only that evidence into an explicitly local
//! directory.

use super::{Lab, LabError, apply_known_failures, evaluate, hash_json, read_required};
use crate::domain::Artifact;
use crate::graph::{CommunityScope, Graph, leiden_communities};
use crate::lab::model::{ExpectationSet, MinimizedFailureBundle, RunManifest};
use crate::storage::JsonStore;
use std::path::Path;

impl Lab {
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
