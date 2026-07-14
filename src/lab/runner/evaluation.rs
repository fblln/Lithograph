//! Expectation evaluation and metric derivation: turns a graph, its
//! communities, and a case's expectation set into pass/fail assertions and
//! aggregate precision/recall/ranking metrics. Shared by the correctness
//! suite, mutation scenarios, and failure minimization.

use crate::domain::Artifact;
use crate::graph::{
    CommunitySummary, Graph, GraphNode, GraphValidator, RelationKind, filter_classes,
};
use crate::lab::metrics::{ConfusionMatrix, mean_reciprocal_rank};
use crate::lab::model::{AssertionResult, Expectation, MetricResult};
use std::collections::BTreeMap;

/// Evaluates every expectation against a built graph, its artifacts, and its
/// communities, producing one assertion result per expectation.
pub(super) fn evaluate(
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

/// Returns the substring selectors needed to focus a full graph-build trace
/// on the evidence relevant to every currently-failing expectation.
pub(super) fn failed_trace_selectors(
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
pub(super) struct CommunityScopeComparison {
    pub(super) ari: f64,
    pub(super) nmi: f64,
    pub(super) pair_accuracy: f64,
    pub(super) mean_cohesion: f64,
    pub(super) mean_conductance: f64,
}

/// Compares the production community scope against a candidate scope on
/// clustering agreement (ARI/NMI), curated pair accuracy, and mean
/// cohesion/conductance, without changing the production default.
pub(super) fn compare_community_scopes(
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

/// Scales a `0.0..=1.0` fraction to fixed-point millionths for
/// timestamp/machine-free correctness observations.
pub(super) fn millionths(value: f64) -> u64 {
    (value.clamp(0.0, 1.0) * 1_000_000.0).round() as u64
}

/// Aggregates precision/recall/ranking metrics across all expectations and
/// assertions for one run.
pub(super) fn derive_metrics(
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
