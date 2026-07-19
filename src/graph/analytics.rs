//! Deterministic graph analytics: degree, PageRank, and betweenness centrality.

use crate::graph::{Graph, GraphNodeId};

/// Builds a stable directed adjacency map, retaining nodes referenced only by edges.
fn adjacency(graph: &Graph) -> std::collections::BTreeMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut edges = std::collections::BTreeMap::<GraphNodeId, Vec<GraphNodeId>>::new();
    for node in &graph.nodes {
        edges.entry(node.id().clone()).or_default();
    }
    for relation in &graph.relations {
        edges
            .entry(relation.source.clone())
            .or_default()
            .push(relation.target.clone());
        edges.entry(relation.target.clone()).or_default();
    }
    for targets in edges.values_mut() {
        targets.sort();
        targets.dedup();
    }
    edges
}

/// Deterministic degree/fan metric baseline for a graph.
pub(crate) fn degree_metrics(graph: &Graph) -> Vec<(GraphNodeId, usize, usize)> {
    let mut values = std::collections::BTreeMap::<GraphNodeId, (usize, usize)>::new();
    for node in &graph.nodes {
        values.entry(node.id().clone()).or_default();
    }
    for relation in &graph.relations {
        values.entry(relation.source.clone()).or_default().1 += 1;
        values.entry(relation.target.clone()).or_default().0 += 1;
    }
    values
        .into_iter()
        .map(|(id, (incoming, outgoing))| (id, incoming, outgoing))
        .collect()
}

/// Deterministic fixed-iteration PageRank over the graph.
pub(crate) fn page_rank(graph: &Graph, iterations: usize) -> Vec<(GraphNodeId, f64)> {
    let outgoing = adjacency(graph);
    if outgoing.is_empty() {
        return Vec::new();
    }
    let nodes: Vec<_> = outgoing.keys().cloned().collect();
    let n = nodes.len() as f64;
    let mut ranks = std::collections::BTreeMap::new();
    for node in &nodes {
        ranks.insert(node.clone(), 1.0 / n);
    }
    for _ in 0..iterations {
        let mut next = std::collections::BTreeMap::new();
        for node in &nodes {
            next.insert(node.clone(), 0.15 / n);
        }
        for (source, targets) in &outgoing {
            let share = ranks[source] * 0.85 / targets.len().max(1) as f64;
            for target in targets {
                *next.entry(target.clone()).or_default() += share;
            }
        }
        ranks = next;
    }
    ranks.into_iter().collect()
}

/// Deterministic betweenness execution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BetweennessPolicy {
    /// Maximum node count for exact computation.
    pub exact_node_threshold: usize,
    /// Stable number of sampled sources above the exact threshold.
    pub sample_sources: usize,
}
impl Default for BetweennessPolicy {
    fn default() -> Self {
        Self {
            exact_node_threshold: 128,
            sample_sources: 32,
        }
    }
}

/// Computes directed betweenness centrality with deterministic source selection.
///
/// Exact mode traverses from every node. Above the policy threshold, it samples
/// evenly-spaced nodes from sorted identifiers so repeated runs rank identically.
pub(crate) fn betweenness(graph: &Graph, policy: BetweennessPolicy) -> Vec<(GraphNodeId, f64)> {
    let edges = adjacency(graph);
    let nodes: Vec<_> = edges.keys().cloned().collect();
    if nodes.is_empty() {
        return Vec::new();
    }
    let sources: Vec<_> = if nodes.len() <= policy.exact_node_threshold {
        nodes.clone()
    } else {
        let count = policy.sample_sources.clamp(1, nodes.len());
        (0..count)
            .map(|index| nodes[index * nodes.len() / count].clone())
            .collect()
    };
    let mut scores = std::collections::BTreeMap::<GraphNodeId, f64>::new();
    for node in &nodes {
        scores.insert(node.clone(), 0.0);
    }
    for source in sources {
        let mut predecessors = std::collections::BTreeMap::<GraphNodeId, Vec<GraphNodeId>>::new();
        let mut paths = std::collections::BTreeMap::<GraphNodeId, f64>::new();
        let mut distance = std::collections::BTreeMap::<GraphNodeId, usize>::new();
        let mut queue = std::collections::VecDeque::from([source.clone()]);
        let mut order = Vec::new();
        paths.insert(source.clone(), 1.0);
        distance.insert(source.clone(), 0);
        while let Some(node) = queue.pop_front() {
            order.push(node.clone());
            let next_distance = distance[&node] + 1;
            for next in &edges[&node] {
                if !distance.contains_key(next) {
                    distance.insert(next.clone(), next_distance);
                    queue.push_back(next.clone());
                }
                if distance[next] == next_distance {
                    *paths.entry(next.clone()).or_default() += paths[&node];
                    predecessors
                        .entry(next.clone())
                        .or_default()
                        .push(node.clone());
                }
            }
        }
        let mut dependency = std::collections::BTreeMap::<GraphNodeId, f64>::new();
        while let Some(node) = order.pop() {
            let node_dependency = dependency.get(&node).copied().unwrap_or_default();
            if let Some(parents) = predecessors.get(&node) {
                for parent in parents {
                    let contribution = paths[parent] / paths[&node] * (1.0 + node_dependency);
                    *dependency.entry(parent.clone()).or_default() += contribution;
                }
            }
            if node != source {
                *scores.entry(node).or_default() += node_dependency;
            }
        }
    }
    if nodes.len() > policy.exact_node_threshold {
        let multiplier = nodes.len() as f64 / policy.sample_sources.clamp(1, nodes.len()) as f64;
        for score in scores.values_mut() {
            *score *= multiplier;
        }
    }
    scores.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{BetweennessPolicy, betweenness, degree_metrics, page_rank};
    use crate::domain::Confidence;
    use crate::graph::{Graph, GraphNodeId, Relation, RelationKind};

    fn relation(id: &str, source: &str, target: &str) -> Relation {
        Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind: RelationKind::Calls,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        }
    }

    #[test]
    fn degree_metrics_report_fan_in_and_fan_out() {
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![relation("r1", "a", "b"), relation("r2", "c", "b")],
        };
        assert!(
            degree_metrics(&graph)
                .iter()
                .any(|(id, incoming, outgoing)| id.as_str() == "b"
                    && *incoming == 2
                    && *outgoing == 0)
        );
    }

    #[test]
    fn page_rank_is_deterministic() {
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![relation("r1", "a", "b"), relation("r2", "c", "b")],
        };
        assert_eq!(page_rank(&graph, 20), page_rank(&graph, 20));
    }

    #[test]
    fn betweenness_is_exact_on_a_small_graph() {
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![
                relation("r1", "a", "b"),
                relation("r2", "b", "a"),
                relation("r3", "b", "c"),
            ],
        };
        let betweenness = betweenness(
            &graph,
            BetweennessPolicy {
                exact_node_threshold: 3,
                sample_sources: 1,
            },
        );
        assert!(betweenness[1].1 > betweenness[0].1);
    }

    #[test]
    fn sampled_betweenness_has_a_stable_rank_order() {
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![relation("r1", "a", "b"), relation("r2", "b", "c")],
        };
        let policy = BetweennessPolicy {
            exact_node_threshold: 2,
            sample_sources: 2,
        };
        let first = betweenness(&graph, policy);
        assert_eq!(first, betweenness(&graph, policy));
        assert!(first[1].1 > first[0].1 && first[1].1 > first[2].1);
    }
}
