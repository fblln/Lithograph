//! Versioned typed analytics records tied to one graph snapshot.

use crate::graph::{Graph, GraphNodeId, RelationKind};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};

/// A persisted analytics run over one immutable graph snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct MetricSnapshot {
    /// Stable analytics snapshot id.
    pub id: String,
    /// Graph snapshot id this result was computed from.
    pub graph_snapshot_id: String,
    /// Algorithm name.
    pub algorithm: String,
    /// Algorithm semantics version.
    pub algorithm_version: u32,
    /// Deterministic filter-scope hash.
    pub filter_scope: String,
    /// Creation metadata supplied by the caller.
    pub created_at: String,
}

/// One node-scoped metric value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct NodeMetric {
    /// Parent metric snapshot id.
    pub metric_snapshot_id: String,
    /// Measured graph node.
    pub node_id: GraphNodeId,
    /// Metric name.
    pub name: String,
    /// Numeric value.
    pub value: f64,
    /// Stable rank within the metric result.
    pub rank: u64,
}

/// Selects which relation kinds participate in a metric computation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MetricScope {
    /// Every relation kind participates.
    Combined,
    /// Only the listed relation kinds participate.
    RelationKinds(Vec<RelationKind>),
}

/// Builds a stable directed adjacency map, retaining nodes referenced only by edges.
fn scoped_adjacency(
    graph: &Graph,
    scope: &MetricScope,
) -> std::collections::BTreeMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut edges = std::collections::BTreeMap::<GraphNodeId, Vec<GraphNodeId>>::new();
    for node in &graph.nodes {
        edges.entry(node.id().clone()).or_default();
    }
    for relation in &graph.relations {
        if matches!(scope, MetricScope::RelationKinds(kinds) if !kinds.contains(&relation.kind)) {
            continue;
        }
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

/// Deterministic degree/fan metric baseline for a graph scope.
pub(crate) fn degree_metrics(
    graph: &Graph,
    scope: &MetricScope,
) -> Vec<(GraphNodeId, usize, usize)> {
    let mut values = std::collections::BTreeMap::<GraphNodeId, (usize, usize)>::new();
    for node in &graph.nodes {
        values.entry(node.id().clone()).or_default();
    }
    for relation in &graph.relations {
        if matches!(scope, MetricScope::RelationKinds(kinds) if !kinds.contains(&relation.kind)) {
            continue;
        }
        values.entry(relation.source.clone()).or_default().1 += 1;
        values.entry(relation.target.clone()).or_default().0 += 1;
    }
    values
        .into_iter()
        .map(|(id, (incoming, outgoing))| (id, incoming, outgoing))
        .collect()
}

/// Deterministic fixed-iteration PageRank over a selected relation scope.
pub(crate) fn page_rank(
    graph: &Graph,
    scope: &MetricScope,
    iterations: usize,
) -> Vec<(GraphNodeId, f64)> {
    let outgoing = scoped_adjacency(graph, scope);
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

/// Deterministic weak connectivity components for the selected graph scope.
pub(crate) fn connectivity_components(graph: &Graph, scope: &MetricScope) -> Vec<Vec<GraphNodeId>> {
    let mut adjacency = std::collections::BTreeMap::<GraphNodeId, Vec<GraphNodeId>>::new();
    for (source, targets) in scoped_adjacency(graph, scope) {
        adjacency.entry(source.clone()).or_default();
        for target in targets {
            adjacency.entry(target.clone()).or_default();
            adjacency
                .entry(source.clone())
                .or_default()
                .push(target.clone());
            adjacency.entry(target).or_default().push(source.clone());
        }
    }
    let mut seen = std::collections::BTreeSet::new();
    let mut groups = Vec::new();
    for root in adjacency.keys().cloned().collect::<Vec<_>>() {
        if !seen.insert(root.clone()) {
            continue;
        }
        let mut stack = vec![root];
        let mut group = Vec::new();
        while let Some(node) = stack.pop() {
            group.push(node.clone());
            for next in &adjacency[&node] {
                if seen.insert(next.clone()) {
                    stack.push(next.clone());
                }
            }
        }
        group.sort();
        groups.push(group);
    }
    groups
}

/// Deterministic directed strongly connected components (reachability based).
pub(crate) fn strongly_connected_components(
    graph: &Graph,
    scope: &MetricScope,
) -> Vec<Vec<GraphNodeId>> {
    let forward = scoped_adjacency(graph, scope);
    let nodes: Vec<_> = forward.keys().cloned().collect();
    let mut reverse = std::collections::BTreeMap::<GraphNodeId, Vec<GraphNodeId>>::new();
    for node in &nodes {
        reverse.entry(node.clone()).or_default();
    }
    for (source, targets) in &forward {
        for target in targets {
            reverse
                .entry(target.clone())
                .or_default()
                .push(source.clone());
        }
    }
    let reachable =
        |start: &GraphNodeId, edges: &std::collections::BTreeMap<GraphNodeId, Vec<GraphNodeId>>| {
            let mut seen = std::collections::BTreeSet::from([start.clone()]);
            let mut stack = vec![start.clone()];
            while let Some(node) = stack.pop() {
                for next in &edges[&node] {
                    if seen.insert(next.clone()) {
                        stack.push(next.clone());
                    }
                }
            }
            seen
        };
    let mut remaining = std::collections::BTreeSet::from_iter(nodes);
    let mut groups = Vec::new();
    while let Some(root) = remaining.iter().next().cloned() {
        let forward_set = reachable(&root, &forward);
        let reverse_set = reachable(&root, &reverse);
        let group: Vec<_> = forward_set.intersection(&reverse_set).cloned().collect();
        for node in &group {
            remaining.remove(node);
        }
        groups.push(group);
    }
    groups
}

/// Deterministic directed closeness centrality for the selected relation scope.
pub(crate) fn closeness(graph: &Graph, scope: &MetricScope) -> Vec<(GraphNodeId, f64)> {
    let edges = scoped_adjacency(graph, scope);
    let nodes: Vec<_> = edges.keys().cloned().collect();
    nodes
        .into_iter()
        .map(|root| {
            let mut distance = std::collections::BTreeMap::from([(root.clone(), 0usize)]);
            let mut queue = std::collections::VecDeque::from([root.clone()]);
            while let Some(node) = queue.pop_front() {
                let next_distance = distance[&node] + 1;
                for next in &edges[&node] {
                    if !distance.contains_key(next) {
                        distance.insert(next.clone(), next_distance);
                        queue.push_back(next.clone());
                    }
                }
            }
            let sum: usize = distance.values().sum();
            let score = if distance.len() > 1 && sum > 0 {
                (distance.len() - 1) as f64 / sum as f64
            } else {
                0.0
            };
            (root, score)
        })
        .collect()
}

/// Deterministic betweenness execution policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct BetweennessPolicy {
    /// Maximum scoped node count for exact computation.
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

/// Returns whether a graph uses exact or deterministic sampled betweenness.
pub(crate) fn betweenness_mode(node_count: usize, policy: BetweennessPolicy) -> &'static str {
    if node_count <= policy.exact_node_threshold {
        "exact"
    } else {
        "deterministic-sampled"
    }
}

/// Computes directed betweenness centrality with deterministic source selection.
///
/// Exact mode traverses from every node. Above the policy threshold, it samples
/// evenly-spaced nodes from sorted identifiers so repeated runs rank identically.
pub(crate) fn betweenness(
    graph: &Graph,
    scope: &MetricScope,
    policy: BetweennessPolicy,
) -> Vec<(GraphNodeId, f64)> {
    let edges = scoped_adjacency(graph, scope);
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

/// Versioned on-disk analytics snapshot store, separate from core graph data.
#[derive(Debug, Clone)]
pub(crate) struct MetricSnapshotStore {
    root: std::path::PathBuf,
}
impl MetricSnapshotStore {
    /// Creates a store rooted at `.lithograph/analytics` or an equivalent path.
    pub(crate) fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
    /// Persists a snapshot only when its versioned payload changes.
    pub(crate) fn save(
        &self,
        snapshot: &MetricSnapshot,
        metrics: &[NodeMetric],
    ) -> std::io::Result<bool> {
        let payload = serde_json::to_string(&(snapshot, metrics)).map_err(std::io::Error::other)?;
        let path = self.path(snapshot);
        let current: Option<String> = JsonStore.read(&path)?;
        if current.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }
    /// Loads the exact versioned snapshot and its node metrics.
    pub(crate) fn load(
        &self,
        snapshot: &MetricSnapshot,
    ) -> std::io::Result<Option<(MetricSnapshot, Vec<NodeMetric>)>> {
        let Some(payload): Option<String> = JsonStore.read(&self.path(snapshot))? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(std::io::Error::other)
    }
    fn path(&self, snapshot: &MetricSnapshot) -> std::path::PathBuf {
        self.root.join(format!(
            "{}.json",
            blake3::hash(snapshot.invalidation_key().as_bytes()).to_hex()
        ))
    }
}

impl MetricSnapshot {
    /// Deterministic invalidation key: recompute when any component changes.
    pub(crate) fn invalidation_key(&self) -> String {
        format!(
            "{}:{}:{}:{}",
            self.graph_snapshot_id, self.algorithm, self.algorithm_version, self.filter_scope
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BetweennessPolicy, MetricScope, MetricSnapshot, MetricSnapshotStore, NodeMetric,
        betweenness, betweenness_mode, closeness, degree_metrics, page_rank,
        strongly_connected_components,
    };
    use crate::domain::Confidence;
    use crate::graph::{Graph, GraphNodeId, Relation, RelationKind};
    #[test]
    fn invalidation_key_changes_for_version_or_scope() {
        let base = MetricSnapshot {
            id: "m1".to_owned(),
            graph_snapshot_id: "g1".to_owned(),
            algorithm: "degree".to_owned(),
            algorithm_version: 1,
            filter_scope: "all".to_owned(),
            created_at: "".to_owned(),
        };
        let mut changed = base.clone();
        changed.algorithm_version = 2;
        assert_ne!(base.invalidation_key(), changed.invalidation_key());
    }
    #[test]
    fn versioned_store_round_trips_and_skips_identical_payload()
    -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = MetricSnapshot {
            id: "m1".to_owned(),
            graph_snapshot_id: "g1".to_owned(),
            algorithm: "degree".to_owned(),
            algorithm_version: 1,
            filter_scope: "all".to_owned(),
            created_at: "".to_owned(),
        };
        let metrics = vec![NodeMetric {
            metric_snapshot_id: "m1".to_owned(),
            node_id: GraphNodeId::new("symbol:a"),
            name: "degree".to_owned(),
            value: 2.0,
            rank: 1,
        }];
        let temp = tempfile::TempDir::new()?;
        let store = MetricSnapshotStore::new(temp.path());
        assert!(store.save(&snapshot, &metrics)?);
        assert!(!store.save(&snapshot, &metrics)?);
        assert_eq!(store.load(&snapshot)?, Some((snapshot, metrics)));
        Ok(())
    }
    #[test]
    fn scoped_degree_metrics_report_fan_in_and_fan_out() {
        let relation = |id: &str, source: &str, target: &str, kind| Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        };
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![
                relation("r1", "a", "b", RelationKind::Calls),
                relation("r2", "c", "b", RelationKind::Imports),
            ],
        };
        assert!(degree_metrics(&graph, &MetricScope::Combined).iter().any(
            |(id, incoming, outgoing)| id.as_str() == "b" && *incoming == 2 && *outgoing == 0
        ));
        assert!(
            degree_metrics(
                &graph,
                &MetricScope::RelationKinds(vec![RelationKind::Calls])
            )
            .iter()
            .any(|(id, incoming, _)| id.as_str() == "b" && *incoming == 1)
        );
    }
    #[test]
    fn page_rank_is_deterministic() {
        let relation = |id: &str, source: &str, target: &str| Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind: RelationKind::Calls,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        };
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![relation("r1", "a", "b"), relation("r2", "c", "b")],
        };
        assert_eq!(
            page_rank(&graph, &MetricScope::Combined, 20),
            page_rank(&graph, &MetricScope::Combined, 20)
        );
    }
    #[test]
    fn betweenness_policy_switches_deterministically_at_threshold() {
        let policy = BetweennessPolicy {
            exact_node_threshold: 3,
            sample_sources: 2,
        };
        assert_eq!(betweenness_mode(3, policy), "exact");
        assert_eq!(betweenness_mode(4, policy), "deterministic-sampled");
    }
    #[test]
    fn centrality_and_scc_are_exact_on_a_small_graph() {
        let relation = |id: &str, source: &str, target: &str| Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind: RelationKind::Calls,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        };
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![
                relation("r1", "a", "b"),
                relation("r2", "b", "a"),
                relation("r3", "b", "c"),
            ],
        };
        assert_eq!(
            strongly_connected_components(&graph, &MetricScope::Combined),
            vec![
                vec![GraphNodeId::new("a"), GraphNodeId::new("b")],
                vec![GraphNodeId::new("c")],
            ]
        );
        let betweenness = betweenness(
            &graph,
            &MetricScope::Combined,
            BetweennessPolicy {
                exact_node_threshold: 3,
                sample_sources: 1,
            },
        );
        assert!(betweenness[1].1 > betweenness[0].1);
        assert!(closeness(&graph, &MetricScope::Combined)[1].1 > 0.0);
    }
    #[test]
    fn sampled_betweenness_has_a_stable_rank_order() {
        let relation = |id: &str, source: &str, target: &str| Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind: RelationKind::Calls,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        };
        let graph = Graph {
            nodes: Vec::new(),
            relations: vec![relation("r1", "a", "b"), relation("r2", "b", "c")],
        };
        let policy = BetweennessPolicy {
            exact_node_threshold: 2,
            sample_sources: 2,
        };
        let first = betweenness(&graph, &MetricScope::Combined, policy);
        assert_eq!(first, betweenness(&graph, &MetricScope::Combined, policy));
        assert!(first[1].1 > first[0].1 && first[1].1 > first[2].1);
    }
}
