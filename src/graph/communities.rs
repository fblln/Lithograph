//! Deterministic, scoped Leiden-style community summaries.

use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use crate::inventory::is_test_path;
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::time::Instant;

/// Version of the deterministic local-moving Leiden phase implemented here.
pub(crate) const LEIDEN_ALGORITHM_VERSION: u32 = 5;

/// Edge scope used while detecting communities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommunityScope {
    /// Consider every relation kind.
    Combined,
    /// Production-code architecture, excluding non-production bridge nodes
    /// and relation categories that primarily express file/package indexing.
    Architecture,
    /// Consider only these relation kinds.
    RelationKinds(Vec<RelationKind>),
    /// Consider relation kinds with explicit positive integer weights.
    WeightedRelationKinds(BTreeMap<RelationKind, u32>),
}

/// Scope used for repository architecture views and correctness evaluation.
///
/// It favors direct code structure and behavior while excluding package hubs,
/// unresolved references, and documentation/example/test artifacts. Those
/// facts remain queryable in the graph; they simply do not determine module
/// boundaries.
pub(crate) fn architecture_aware_scope() -> CommunityScope {
    CommunityScope::Architecture
}

/// Deterministic work counters plus non-canonical phase timings.
///
/// Counts can be compared across machines. Durations are deliberately kept
/// outside [`CommunitySummary`] so correctness hashes never depend on a clock.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommunityDiagnostics {
    /// Nodes incident to at least one selected relation.
    pub participating_nodes: u64,
    /// Relations admitted by the selected scope.
    pub selected_edges: u64,
    /// Deterministic movement work expressed as bounded active-queue sweeps.
    pub iterations: u64,
    /// Number of active nodes whose candidate labels were evaluated.
    pub nodes_reconsidered: u64,
    /// Number of strictly beneficial label changes.
    pub successful_moves: u64,
    /// Weighted neighbour-label observations made by movement.
    pub neighbour_label_evaluations: u64,
    /// Selected relations visited while constructing summaries.
    pub summary_edge_visits: u64,
    /// Whether the deterministic movement safety bound was reached.
    pub safety_bound_reached: bool,
    /// Wall-clock adjacency construction time; excluded from correctness data.
    pub adjacency_us: u64,
    /// Wall-clock local-movement time; excluded from correctness data.
    pub movement_us: u64,
    /// Wall-clock summary construction time; excluded from correctness data.
    pub summary_us: u64,
}

/// Community result with diagnostics suitable for lab and health consumers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CommunityAnalysis {
    /// Stable community summaries.
    pub communities: Vec<CommunitySummary>,
    /// Phase-level work and timing observations.
    pub diagnostics: CommunityDiagnostics,
    /// Whether summaries came from an exact versioned cache entry.
    pub cache_hit: bool,
}

/// Scope preset that keeps code, configuration, and environment neighborhoods
/// connected while giving direct semantic links more influence.
pub(crate) fn environment_aware_scope() -> CommunityScope {
    CommunityScope::WeightedRelationKinds(BTreeMap::from([
        (RelationKind::Contains, 1),
        (RelationKind::ReadsEnv, 3),
        (RelationKind::DefinesEnv, 3),
        (RelationKind::BindsConfig, 4),
        (RelationKind::ReferencesConfig, 3),
        (RelationKind::Calls, 2),
        (RelationKind::References, 1),
    ]))
}

/// A versioned summary of one detected community.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CommunitySummary {
    /// Stable id derived from its lexicographically first member.
    pub id: String,
    /// Human-readable stable label.
    pub label: String,
    /// Community members in stable order.
    pub members: Vec<GraphNodeId>,
    /// Intra-community density.
    pub cohesion: f64,
    /// Fraction of incident edges leaving the community.
    pub conductance: f64,
    /// Relation ids crossing the community boundary.
    pub boundary_edges: Vec<String>,
    /// Highest internal-degree members.
    pub representative_nodes: Vec<GraphNodeId>,
    /// Package names represented by members.
    pub dominant_packages: Vec<String>,
    /// Members that participate in boundary edges.
    pub bridge_nodes: Vec<GraphNodeId>,
}

/// A persisted, versioned community computation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CommunitySnapshot {
    /// Immutable graph snapshot that was analysed.
    pub graph_snapshot_id: String,
    /// Versioned deterministic Leiden semantics.
    pub algorithm_version: u32,
    /// Relation scope used by the computation.
    pub scope: CommunityScope,
    /// Stable community summaries for this snapshot.
    pub communities: Vec<CommunitySummary>,
}

/// Deterministically persists community results outside core graph facts.
#[derive(Debug, Clone)]
pub(crate) struct CommunitySnapshotStore {
    root: std::path::PathBuf,
}

impl CommunitySnapshotStore {
    /// Creates a store rooted at `.lithograph/analytics` or equivalent.
    pub(crate) fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes only a changed versioned community snapshot.
    pub(crate) fn save(&self, snapshot: &CommunitySnapshot) -> std::io::Result<bool> {
        let payload = serde_json::to_string(snapshot).map_err(std::io::Error::other)?;
        let path = self.path(snapshot);
        if matches!(
            JsonStore.read::<String>(&path),
            Ok(Some(existing)) if existing == payload
        ) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }

    /// Loads the exact persisted snapshot when present.
    pub(crate) fn load(
        &self,
        snapshot: &CommunitySnapshot,
    ) -> std::io::Result<Option<CommunitySnapshot>> {
        self.load_exact(&snapshot.graph_snapshot_id, &snapshot.scope)
    }

    /// Loads an exact graph/scope/version entry. Invalid payloads are treated
    /// as misses so callers can safely recompute and replace them.
    pub(crate) fn load_exact(
        &self,
        graph_snapshot_id: &str,
        scope: &CommunityScope,
    ) -> std::io::Result<Option<CommunitySnapshot>> {
        let expected_scope = normalized_scope(scope);
        let path = self.path_for(graph_snapshot_id, &expected_scope);
        let Some(payload): Option<String> = JsonStore.read(&path).ok().flatten() else {
            return Ok(None);
        };
        let Ok(snapshot) = serde_json::from_str::<CommunitySnapshot>(&payload) else {
            return Ok(None);
        };
        if snapshot.graph_snapshot_id != graph_snapshot_id
            || snapshot.algorithm_version != LEIDEN_ALGORITHM_VERSION
            || normalized_scope(&snapshot.scope) != expected_scope
        {
            return Ok(None);
        }
        Ok(Some(snapshot))
    }

    fn path(&self, snapshot: &CommunitySnapshot) -> std::path::PathBuf {
        self.path_for(&snapshot.graph_snapshot_id, &snapshot.scope)
    }

    fn path_for(&self, graph_snapshot_id: &str, scope: &CommunityScope) -> std::path::PathBuf {
        let key = format!(
            "{}:{}:{:?}",
            graph_snapshot_id,
            LEIDEN_ALGORITHM_VERSION,
            normalized_scope(scope)
        );
        self.root
            .join(format!("{}.json", blake3::hash(key.as_bytes()).to_hex()))
    }
}

/// Version of deterministic topic-label semantics over node documents.
pub(crate) const TOPIC_ALGORITHM_VERSION: u32 = 1;

/// Topic labels attached to one detected community.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CommunityTopic {
    /// Community identifier.
    pub community_id: String,
    /// Bounded labels ordered by descending score then token.
    pub labels: Vec<String>,
    /// Stable community membership copied from the community snapshot.
    pub members: Vec<GraphNodeId>,
}

/// Versioned topic/community overlay kept separate from graph resolver edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TopicSnapshot {
    /// Immutable graph snapshot that was analysed.
    pub graph_snapshot_id: String,
    /// Hash of graph and community inputs.
    pub input_hash: String,
    /// Versioned topic-label semantics.
    pub algorithm_version: u32,
    /// Stable topic labels and memberships.
    pub communities: Vec<CommunityTopic>,
}

/// Deterministically persists topic overlays and skips identical writes.
#[derive(Debug, Clone)]
pub(crate) struct TopicSnapshotStore {
    root: std::path::PathBuf,
}

impl TopicSnapshotStore {
    /// Creates a store rooted at `.lithograph/analytics` or equivalent.
    pub(crate) fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes a topic snapshot only when its content changes.
    pub(crate) fn save(&self, snapshot: &TopicSnapshot) -> std::io::Result<bool> {
        let payload = serde_json::to_string(snapshot).map_err(std::io::Error::other)?;
        let path = self.path(snapshot);
        if JsonStore.read::<String>(&path)?.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }

    /// Loads a previously persisted topic snapshot.
    pub(crate) fn load(&self, snapshot: &TopicSnapshot) -> std::io::Result<Option<TopicSnapshot>> {
        let Some(payload): Option<String> = JsonStore.read(&self.path(snapshot))? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(std::io::Error::other)
    }

    fn path(&self, snapshot: &TopicSnapshot) -> std::path::PathBuf {
        let key = format!(
            "{}:{}:{}",
            snapshot.graph_snapshot_id, snapshot.algorithm_version, snapshot.input_hash
        );
        self.root.join(format!(
            "topic-{}.json",
            blake3::hash(key.as_bytes()).to_hex()
        ))
    }
}

/// Labels communities from deterministic local node-document token evidence.
pub(crate) fn label_topic_snapshot(
    graph_snapshot_id: impl Into<String>,
    graph: &Graph,
    communities: &[CommunitySummary],
) -> TopicSnapshot {
    let documents = crate::semantic_search::collect_documents(graph);
    let document_map: BTreeMap<GraphNodeId, &crate::semantic_search::NodeDocument> = documents
        .iter()
        .map(|document| (document.id.clone(), document))
        .collect();
    let mut document_frequency = BTreeMap::<String, usize>::new();
    for document in &documents {
        let unique: BTreeSet<String> = crate::fts::tokenize(&document.text)
            .into_iter()
            .filter(|token| token.len() >= 3)
            .collect();
        for token in unique {
            *document_frequency.entry(token).or_default() += 1;
        }
    }
    let mut topics = communities
        .iter()
        .map(|community| {
            let mut term_frequency = BTreeMap::<String, usize>::new();
            for member in &community.members {
                let Some(document) = document_map.get(member) else {
                    continue;
                };
                for token in crate::fts::tokenize(&document.text)
                    .into_iter()
                    .filter(|token| token.len() >= 3)
                {
                    *term_frequency.entry(token).or_default() += 1;
                }
            }
            let mut ranked: Vec<(String, usize)> = term_frequency
                .into_iter()
                .map(|(token, frequency)| {
                    let document_count = document_frequency.get(&token).copied().unwrap_or(0);
                    let score = frequency.saturating_mul(1000) / document_count.max(1);
                    (token, score)
                })
                .collect();
            ranked.sort_by(|left, right| right.1.cmp(&left.1).then(left.0.cmp(&right.0)));
            CommunityTopic {
                community_id: community.id.clone(),
                labels: ranked.into_iter().take(5).map(|(token, _)| token).collect(),
                members: community.members.clone(),
            }
        })
        .collect::<Vec<_>>();
    topics.sort_by(|left, right| left.community_id.cmp(&right.community_id));
    let graph_snapshot_id = graph_snapshot_id.into();
    let input_hash = blake3::hash(&serde_json::to_vec(&(graph, &topics)).unwrap_or_default())
        .to_hex()
        .to_string();
    TopicSnapshot {
        graph_snapshot_id,
        input_hash,
        algorithm_version: TOPIC_ALGORITHM_VERSION,
        communities: topics,
    }
}

/// Detects communities using Leiden's deterministic local-moving phase.
///
/// The graph is treated as weighted-undirected for modularity movement. Nodes
/// visit in sorted-id order and ties select the smallest label, eliminating
/// random ordering from the usual Leiden implementation.
pub(crate) fn leiden_communities(graph: &Graph, scope: &CommunityScope) -> Vec<CommunitySummary> {
    leiden_communities_with_diagnostics(graph, scope).communities
}

/// Shared cache-aware entry point for lab, health, and query consumers.
pub(crate) fn analyze_communities(
    graph: &Graph,
    scope: &CommunityScope,
    store: Option<&CommunitySnapshotStore>,
) -> std::io::Result<CommunityAnalysis> {
    let normalized_scope = normalized_scope(scope);
    let graph_payload = graph.to_json().map_err(std::io::Error::other)?;
    let graph_snapshot_id = blake3::hash(graph_payload.as_bytes()).to_hex().to_string();
    if let Some(snapshot) = store.and_then(|store| {
        store
            .load_exact(&graph_snapshot_id, &normalized_scope)
            .ok()
            .flatten()
    }) {
        return Ok(CommunityAnalysis {
            communities: snapshot.communities,
            diagnostics: CommunityDiagnostics::default(),
            cache_hit: true,
        });
    }
    let analysis = leiden_communities_with_diagnostics(graph, &normalized_scope);
    if let Some(store) = store {
        store.save(&CommunitySnapshot {
            graph_snapshot_id,
            algorithm_version: LEIDEN_ALGORITHM_VERSION,
            scope: normalized_scope,
            communities: analysis.communities.clone(),
        })?;
    }
    Ok(analysis)
}

/// Detects communities and reports deterministic work plus per-phase timings.
pub(crate) fn leiden_communities_with_diagnostics(
    graph: &Graph,
    scope: &CommunityScope,
) -> CommunityAnalysis {
    let adjacency_start = Instant::now();
    let graph_nodes: BTreeMap<_, _> = graph.nodes.iter().map(|node| (node.id(), node)).collect();
    let selected: Vec<_> = graph
        .relations
        .iter()
        .filter(|edge| relation_in_scope(edge, &graph_nodes, scope))
        .collect();
    let node_ids: Vec<_> = selected
        .iter()
        .flat_map(|edge| [edge.source.clone(), edge.target.clone()])
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let node_indices: BTreeMap<_, _> = node_ids
        .iter()
        .cloned()
        .enumerate()
        .map(|(index, id)| (id, index))
        .collect();
    let indexed_edges: Vec<_> = selected
        .iter()
        .map(|edge| {
            (
                node_indices[&edge.source],
                node_indices[&edge.target],
                *edge,
            )
        })
        .collect();

    // Ordered maps are used only while aggregating parallel relations. The hot
    // movement loop below operates exclusively on compact sorted vectors.
    let mut adjacency_maps = vec![BTreeMap::<usize, u64>::new(); node_ids.len()];
    for (source, target, edge) in &indexed_edges {
        let weight = u64::from(relation_weight(edge.kind, scope));
        *adjacency_maps[*source].entry(*target).or_default() += weight;
        *adjacency_maps[*target].entry(*source).or_default() += weight;
    }
    let adjacency: Vec<Vec<(usize, u64)>> = adjacency_maps
        .into_iter()
        .map(|neighbors| neighbors.into_iter().collect())
        .collect();
    let degrees: Vec<u64> = adjacency
        .iter()
        .map(|neighbors| neighbors.iter().map(|(_, weight)| *weight).sum())
        .collect();
    let total_degree: u64 = degrees.iter().sum();
    let adjacency_us = micros(adjacency_start.elapsed());

    let movement_start = Instant::now();
    let mut labels: Vec<usize> = (0..node_ids.len()).collect();
    let mut volumes = degrees.clone();
    let mut active: BTreeSet<usize> = (0..node_ids.len()).collect();
    let mut label_weights = vec![0u64; node_ids.len()];
    let mut touched = Vec::<usize>::new();
    let mut nodes_reconsidered = 0u64;
    let mut successful_moves = 0u64;
    let mut neighbour_label_evaluations = 0u64;
    let safety_bound = node_ids
        .len()
        .max(1)
        .saturating_mul(indexed_edges.len().max(node_ids.len()).max(1));
    while let Some(node) = active.pop_first() {
        if nodes_reconsidered as usize >= safety_bound {
            break;
        }
        nodes_reconsidered += 1;
        let degree = degrees[node];
        let current = labels[node];
        volumes[current] = volumes[current].saturating_sub(degree);
        for (neighbor, weight) in &adjacency[node] {
            let label = labels[*neighbor];
            if label_weights[label] == 0 {
                touched.push(label);
            }
            label_weights[label] += *weight;
            neighbour_label_evaluations += 1;
        }
        if label_weights[current] == 0 {
            touched.push(current);
        }
        let score = |label: usize, internal_weight: u64| -> i128 {
            i128::from(internal_weight) * i128::from(total_degree)
                - i128::from(degree) * i128::from(volumes[label])
        };
        let staying_score = score(current, label_weights[current]);
        let mut next = current;
        let mut next_score = staying_score;
        for label in touched.iter().copied() {
            let candidate_score = score(label, label_weights[label]);
            if is_better_candidate(label, candidate_score, next, next_score, staying_score) {
                next = label;
                next_score = candidate_score;
            }
        }
        for label in touched.drain(..) {
            label_weights[label] = 0;
        }
        if next != current && next_score > staying_score {
            labels[node] = next;
            successful_moves += 1;
            for (neighbor, _) in &adjacency[node] {
                active.insert(*neighbor);
            }
        }
        volumes[labels[node]] += degree;
    }
    let safety_bound_reached = !active.is_empty();
    let iterations = nodes_reconsidered
        .saturating_add(node_ids.len().saturating_sub(1) as u64)
        .checked_div(node_ids.len().max(1) as u64)
        .unwrap_or(0);
    let movement_us = micros(movement_start.elapsed());

    let summary_start = Instant::now();
    let mut groups = vec![Vec::<usize>::new(); node_ids.len()];
    for (node, label) in labels.iter().copied().enumerate() {
        groups[label].push(node);
    }
    let node_packages: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Package(package) => Some((package.id.clone(), package.name.clone())),
            _ => None,
        })
        .collect();
    let retained: Vec<bool> = groups.iter().map(|members| members.len() >= 2).collect();
    let mut intra_counts = vec![0usize; node_ids.len()];
    let mut internal_degrees = vec![0usize; node_ids.len()];
    let mut boundaries = vec![Vec::<String>::new(); node_ids.len()];
    let mut bridges = vec![BTreeSet::<usize>::new(); node_ids.len()];
    for (source, target, edge) in &indexed_edges {
        let source_group = labels[*source];
        let target_group = labels[*target];
        if source_group == target_group {
            if retained[source_group] {
                intra_counts[source_group] += 1;
                internal_degrees[*source] += 1;
                internal_degrees[*target] += 1;
            }
        } else {
            if retained[source_group] {
                boundaries[source_group].push(edge.id.clone());
                bridges[source_group].insert(*source);
            }
            if retained[target_group] {
                boundaries[target_group].push(edge.id.clone());
                bridges[target_group].insert(*target);
            }
        }
    }
    let mut summaries: Vec<_> = groups
        .into_iter()
        .enumerate()
        .filter(|(_, group)| group.len() >= 2)
        .map(|(label, members)| {
            let mut representatives = members.clone();
            representatives.sort_by(|a, b| {
                internal_degrees[*b]
                    .cmp(&internal_degrees[*a])
                    .then(node_ids[*a].cmp(&node_ids[*b]))
            });
            // The preceding filter guarantees a non-empty set; retaining a
            // total fallback keeps the analytics path non-panicking if this
            // helper is ever reused with a different caller.
            let first = members
                .first()
                .map(|index| node_ids[*index].to_string())
                .unwrap_or_default();
            let n = members.len() as f64;
            CommunitySummary {
                id: format!("leiden:{first}"),
                label: format!("Community {first}"),
                members: members
                    .iter()
                    .map(|index| node_ids[*index].clone())
                    .collect(),
                cohesion: stable_ratio(intra_counts[label] as f64, n * (n - 1.0) / 2.0).min(1.0),
                conductance: stable_ratio(
                    boundaries[label].len() as f64,
                    (2.0 * intra_counts[label] as f64 + boundaries[label].len() as f64).max(1.0),
                ),
                boundary_edges: std::mem::take(&mut boundaries[label]),
                representative_nodes: representatives
                    .into_iter()
                    .take(5)
                    .map(|index| node_ids[index].clone())
                    .collect(),
                dominant_packages: members
                    .iter()
                    .filter_map(|index| node_packages.get(&node_ids[*index]).cloned())
                    .collect(),
                bridge_nodes: std::mem::take(&mut bridges[label])
                    .into_iter()
                    .map(|index| node_ids[index].clone())
                    .collect(),
            }
        })
        .collect();
    summaries.sort_by(|a, b| b.members.len().cmp(&a.members.len()).then(a.id.cmp(&b.id)));
    let summary_us = micros(summary_start.elapsed());
    CommunityAnalysis {
        communities: summaries,
        diagnostics: CommunityDiagnostics {
            participating_nodes: node_ids.len() as u64,
            selected_edges: selected.len() as u64,
            iterations,
            nodes_reconsidered,
            successful_moves,
            neighbour_label_evaluations,
            summary_edge_visits: selected.len() as u64,
            safety_bound_reached,
            adjacency_us,
            movement_us,
            summary_us,
        },
        cache_hit: false,
    }
}

fn micros(duration: std::time::Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn stable_ratio(numerator: f64, denominator: f64) -> f64 {
    const SCALE: f64 = 1_000_000_000_000.0;
    ((numerator / denominator.max(1.0)) * SCALE).round() / SCALE
}

fn is_better_candidate(
    candidate_label: usize,
    candidate_score: i128,
    best_label: usize,
    best_score: i128,
    staying_score: i128,
) -> bool {
    candidate_score > best_score
        || (candidate_score == best_score
            && candidate_label < best_label
            && candidate_score > staying_score)
}

fn in_scope(kind: RelationKind, scope: &CommunityScope) -> bool {
    relation_weight(kind, scope) > 0
}

fn relation_in_scope(
    edge: &crate::graph::Relation,
    nodes: &BTreeMap<&GraphNodeId, &GraphNode>,
    scope: &CommunityScope,
) -> bool {
    if !in_scope(edge.kind, scope) {
        return false;
    }
    !matches!(scope, CommunityScope::Architecture)
        || nodes
            .get(&edge.source)
            .zip(nodes.get(&edge.target))
            .is_some_and(|(source, target)| {
                is_production_code_node(source) && is_production_code_node(target)
            })
}

fn is_production_code_node(node: &GraphNode) -> bool {
    let path = match node {
        GraphNode::Artifact(node) => &node.path,
        GraphNode::Symbol(node) => node.evidence.path.as_str(),
        _ => return false,
    };
    !is_auxiliary_path(path)
}

fn is_auxiliary_path(path: &str) -> bool {
    is_test_path(path)
        || path.split('/').any(|component| {
            matches!(
                component,
                "docs"
                    | "examples"
                    | "example"
                    | "benches"
                    | "benchmark"
                    | "benchmarks"
                    | "sample"
                    | "samples"
                    | "integration"
                    | "e2e"
            )
        })
}

fn relation_weight(kind: RelationKind, scope: &CommunityScope) -> u32 {
    match scope {
        CommunityScope::Combined => 1,
        CommunityScope::Architecture => match kind {
            RelationKind::Contains | RelationKind::HasMethod | RelationKind::MemberOf => 4,
            RelationKind::Calls
            | RelationKind::DataFlows
            | RelationKind::Implements
            | RelationKind::Inherits
            | RelationKind::HandlesRoute => 3,
            RelationKind::Imports => 1,
            _ => 0,
        },
        CommunityScope::RelationKinds(kinds) => u32::from(kinds.contains(&kind)),
        CommunityScope::WeightedRelationKinds(weights) => weights.get(&kind).copied().unwrap_or(0),
    }
}

fn normalized_scope(scope: &CommunityScope) -> CommunityScope {
    match scope {
        CommunityScope::Combined => CommunityScope::Combined,
        CommunityScope::Architecture => CommunityScope::Architecture,
        CommunityScope::RelationKinds(kinds) => {
            let mut kinds = kinds.clone();
            kinds.sort();
            kinds.dedup();
            CommunityScope::RelationKinds(kinds)
        }
        CommunityScope::WeightedRelationKinds(weights) => CommunityScope::WeightedRelationKinds(
            weights
                .iter()
                .filter_map(|(kind, weight)| (*weight > 0).then_some((*kind, *weight)))
                .collect(),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Confidence;
    use crate::graph::Relation;

    fn edge(id: &str, from: &str, to: &str, kind: RelationKind) -> Relation {
        Relation {
            id: id.into(),
            source: GraphNodeId::new(from),
            target: GraphNodeId::new(to),
            kind,
            confidence: Confidence::High,
            evidence: vec![],
            provenance: None,
        }
    }
    #[test]
    fn leiden_groups_clusters_and_reports_boundary_edges() {
        let graph = Graph {
            nodes: vec![],
            relations: vec![
                edge("ab", "a", "b", RelationKind::Calls),
                edge("bc", "b", "c", RelationKind::Calls),
                edge("ca", "c", "a", RelationKind::Calls),
                edge("cd", "c", "d", RelationKind::Calls),
                edge("de", "d", "e", RelationKind::Calls),
                edge("ef", "e", "f", RelationKind::Calls),
                edge("fd", "f", "d", RelationKind::Calls),
            ],
        };
        let communities = leiden_communities(&graph, &CommunityScope::Combined);
        assert_eq!(communities.len(), 2);
        assert_eq!(
            communities[0].members,
            vec![
                GraphNodeId::new("a"),
                GraphNodeId::new("b"),
                GraphNodeId::new("c")
            ]
        );
        assert_eq!(communities[0].boundary_edges, vec!["cd"]);
        assert_eq!(communities[0].bridge_nodes, vec![GraphNodeId::new("c")]);
        assert_eq!(
            communities,
            leiden_communities(&graph, &CommunityScope::Combined)
        );
    }

    #[test]
    fn environment_scope_applies_explicit_relation_weights_deterministically() {
        let graph = Graph {
            nodes: vec![],
            relations: vec![
                edge("read", "code", "env", RelationKind::ReadsEnv),
                edge("bind", "env", "config", RelationKind::BindsConfig),
                edge("call", "code", "other", RelationKind::Calls),
            ],
        };
        let scope = environment_aware_scope();
        let first = leiden_communities(&graph, &scope);
        let second = leiden_communities(&graph, &scope);
        assert_eq!(first, second);
        assert!(first.iter().any(|community| {
            community.members.contains(&GraphNodeId::new("env"))
                && community.members.contains(&GraphNodeId::new("config"))
        }));
    }

    #[test]
    fn architecture_scope_keeps_direct_production_code_groups_separate_from_auxiliary_nodes()
    -> Result<(), Box<dyn std::error::Error>> {
        fn artifact(path: &str) -> Result<GraphNode, Box<dyn std::error::Error>> {
            let path = crate::domain::RepoPath::new(path)?;
            let evidence = crate::domain::EvidenceRef::file(
                crate::domain::ArtifactId::from_path(&path),
                path.clone(),
            );
            Ok(GraphNode::Artifact(crate::graph::ArtifactNode {
                id: GraphNodeId::new(format!("artifact:{path}")),
                path: path.to_string(),
                category: crate::domain::ArtifactCategory::SourceCode,
                evidence,
            }))
        }

        let production_path = crate::domain::RepoPath::new("src/service.rs")?;
        let production_evidence = crate::domain::EvidenceRef::file(
            crate::domain::ArtifactId::from_path(&production_path),
            production_path,
        );
        let graph = Graph {
            nodes: vec![
                artifact("src/service.rs")?,
                GraphNode::Symbol(crate::graph::SymbolNode {
                    id: GraphNodeId::new("symbol:src/service.rs#service::run"),
                    kind: crate::graph::SymbolKind::Function,
                    qualified_name: "service::run".to_owned(),
                    doc: None,
                    evidence: production_evidence,
                }),
                artifact("tests/service_test.rs")?,
                artifact("docs/service.md")?,
            ],
            relations: vec![
                edge(
                    "contains",
                    "artifact:src/service.rs",
                    "symbol:src/service.rs#service::run",
                    RelationKind::Contains,
                ),
                edge(
                    "test-call",
                    "artifact:tests/service_test.rs",
                    "symbol:src/service.rs#service::run",
                    RelationKind::Calls,
                ),
                edge(
                    "doc-import",
                    "artifact:docs/service.md",
                    "symbol:src/service.rs#service::run",
                    RelationKind::Imports,
                ),
                edge(
                    "reference",
                    "artifact:src/service.rs",
                    "symbol:src/service.rs#service::run",
                    RelationKind::References,
                ),
            ],
        };

        let analysis = leiden_communities_with_diagnostics(&graph, &architecture_aware_scope());
        assert_eq!(analysis.diagnostics.selected_edges, 1);
        assert_eq!(analysis.communities.len(), 1);
        assert_eq!(
            analysis.communities[0].members,
            vec![
                GraphNodeId::new("artifact:src/service.rs"),
                GraphNodeId::new("symbol:src/service.rs#service::run"),
            ]
        );
        Ok(())
    }

    #[test]
    fn topic_snapshot_labels_are_bounded_and_noop_persisted_writes_are_stable()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = crate::domain::RepoPath::new("src/service.py")?;
        let evidence =
            crate::domain::EvidenceRef::file(crate::domain::ArtifactId::from_path(&path), path);
        let graph = Graph {
            nodes: vec![
                GraphNode::Artifact(crate::graph::ArtifactNode {
                    id: GraphNodeId::new("artifact:src/service.py"),
                    path: "src/service.py".to_owned(),
                    category: crate::domain::ArtifactCategory::SourceCode,
                    evidence: evidence.clone(),
                }),
                GraphNode::Config(crate::graph::ConfigNode {
                    id: GraphNodeId::new("config-key:service.url"),
                    kind: crate::graph::ConfigNodeKind::Key,
                    name: "service.url".to_owned(),
                    evidence,
                }),
            ],
            relations: vec![edge(
                "contains",
                "artifact:src/service.py",
                "config-key:service.url",
                RelationKind::Contains,
            )],
        };
        let communities = vec![CommunitySummary {
            id: "leiden:artifact:src/service.py".to_owned(),
            label: "Community service".to_owned(),
            members: vec![
                GraphNodeId::new("artifact:src/service.py"),
                GraphNodeId::new("config-key:service.url"),
            ],
            cohesion: 1.0,
            conductance: 0.0,
            boundary_edges: vec![],
            representative_nodes: vec![GraphNodeId::new("artifact:src/service.py")],
            dominant_packages: vec![],
            bridge_nodes: vec![],
        }];
        let snapshot = label_topic_snapshot("graph-1", &graph, &communities);
        assert_eq!(
            snapshot,
            label_topic_snapshot("graph-1", &graph, &communities)
        );
        assert!(
            snapshot
                .communities
                .iter()
                .all(|community| community.labels.len() <= 5)
        );
        assert!(
            snapshot
                .communities
                .iter()
                .flat_map(|community| community.labels.iter())
                .any(|label| label == "service")
        );

        // Keep the directory alive while exercising no-op write behavior.
        let root = tempfile::TempDir::new()?;
        let store = TopicSnapshotStore::new(root.path());
        assert!(store.save(&snapshot)?);
        assert!(!store.save(&snapshot)?);
        assert_eq!(store.load(&snapshot)?, Some(snapshot));
        Ok(())
    }
    #[test]
    fn scoped_edges_change_community_membership() {
        let graph = Graph {
            nodes: vec![],
            relations: vec![
                edge("ab", "a", "b", RelationKind::Calls),
                edge("bc", "b", "c", RelationKind::Imports),
            ],
        };
        let communities = leiden_communities(
            &graph,
            &CommunityScope::RelationKinds(vec![RelationKind::Calls]),
        );
        assert_eq!(
            communities[0].members,
            vec![GraphNodeId::new("a"), GraphNodeId::new("b")]
        );
    }

    #[test]
    fn versioned_community_snapshot_round_trips_without_rewriting()
    -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = CommunitySnapshot {
            graph_snapshot_id: "graph-1".to_owned(),
            algorithm_version: LEIDEN_ALGORITHM_VERSION,
            scope: CommunityScope::Combined,
            communities: vec![CommunitySummary {
                id: "leiden:a".to_owned(),
                label: "Community a".to_owned(),
                members: vec![GraphNodeId::new("a"), GraphNodeId::new("b")],
                cohesion: 1.0,
                conductance: 0.0,
                boundary_edges: Vec::new(),
                representative_nodes: vec![GraphNodeId::new("a")],
                dominant_packages: Vec::new(),
                bridge_nodes: Vec::new(),
            }],
        };
        let temp = tempfile::TempDir::new()?;
        let store = CommunitySnapshotStore::new(temp.path());
        assert!(store.save(&snapshot)?);
        assert!(!store.save(&snapshot)?);
        assert_eq!(store.load(&snapshot)?, Some(snapshot));
        Ok(())
    }

    #[test]
    fn diagnostics_count_active_movement_and_one_summary_edge_visit() {
        let mut relations = Vec::new();
        for index in 0..64 {
            relations.push(edge(
                &format!("edge-{index}"),
                &format!("left-{index:03}"),
                &format!("right-{index:03}"),
                RelationKind::Calls,
            ));
        }
        let analysis = leiden_communities_with_diagnostics(
            &Graph {
                nodes: vec![],
                relations,
            },
            &CommunityScope::Combined,
        );
        assert_eq!(analysis.diagnostics.participating_nodes, 128);
        assert_eq!(analysis.diagnostics.selected_edges, 64);
        assert_eq!(analysis.diagnostics.summary_edge_visits, 64);
        assert!(analysis.diagnostics.nodes_reconsidered >= 128);
        assert!(analysis.diagnostics.neighbour_label_evaluations >= 128);
        assert!(!analysis.diagnostics.safety_bound_reached);
        assert_eq!(analysis.communities.len(), 64);
    }

    #[test]
    fn movement_requires_strict_gain_and_converges_deterministically() {
        let graph = Graph {
            nodes: vec![],
            relations: vec![
                edge("aa", "a", "a", RelationKind::Calls),
                edge("bc", "b", "c", RelationKind::Calls),
                edge("bd", "b", "d", RelationKind::Calls),
                edge("cd", "c", "d", RelationKind::Calls),
            ],
        };
        let first = leiden_communities_with_diagnostics(&graph, &CommunityScope::Combined);
        let second = leiden_communities_with_diagnostics(&graph, &CommunityScope::Combined);
        assert_eq!(first.communities, second.communities);
        assert_eq!(first.diagnostics.successful_moves, 2);
        assert!(!first.diagnostics.safety_bound_reached);
        assert!(first.communities.iter().any(|community| {
            community.members
                == vec![
                    GraphNodeId::new("b"),
                    GraphNodeId::new("c"),
                    GraphNodeId::new("d"),
                ]
        }));
    }

    #[test]
    fn movement_ties_choose_the_smallest_stable_label_only_above_staying() {
        assert!(is_better_candidate(2, 11, 3, 11, 10));
        assert!(!is_better_candidate(3, 11, 2, 11, 10));
        assert!(!is_better_candidate(2, 10, 3, 10, 10));
        assert!(!is_better_candidate(2, 9, 3, 10, 8));
    }

    #[test]
    fn cache_identity_normalizes_scope_and_recovers_from_invalid_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = Graph {
            nodes: vec![],
            relations: vec![edge("ab", "a", "b", RelationKind::Calls)],
        };
        let temp = tempfile::TempDir::new()?;
        let store = CommunitySnapshotStore::new(temp.path());
        let reordered_scope =
            CommunityScope::RelationKinds(vec![RelationKind::Calls, RelationKind::Calls]);
        let first = analyze_communities(&graph, &reordered_scope, Some(&store))?;
        assert!(!first.cache_hit);
        let normalized_scope = CommunityScope::RelationKinds(vec![RelationKind::Calls]);
        let second = analyze_communities(&graph, &normalized_scope, Some(&store))?;
        assert!(second.cache_hit);
        assert_eq!(first.communities, second.communities);
        assert_eq!(second.diagnostics.nodes_reconsidered, 0);

        let graph_id = blake3::hash(graph.to_json()?.as_bytes())
            .to_hex()
            .to_string();
        let stale = CommunitySnapshot {
            graph_snapshot_id: graph_id,
            algorithm_version: LEIDEN_ALGORITHM_VERSION - 1,
            scope: normalized_scope.clone(),
            communities: Vec::new(),
        };
        assert!(store.save(&stale)?);
        let recovered = analyze_communities(&graph, &normalized_scope, Some(&store))?;
        assert!(!recovered.cache_hit);
        assert_eq!(recovered.communities, first.communities);

        std::fs::write(store.path(&stale), "not-json")?;
        let recovered = analyze_communities(&graph, &normalized_scope, Some(&store))?;
        assert!(!recovered.cache_hit);
        assert_eq!(recovered.communities, first.communities);
        Ok(())
    }
}
