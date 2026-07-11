//! Deterministic, scoped Leiden-style community summaries.

use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Version of the deterministic local-moving Leiden phase implemented here.
pub const LEIDEN_ALGORITHM_VERSION: u32 = 1;

/// Edge scope used while detecting communities.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommunityScope {
    /// Consider every relation kind.
    Combined,
    /// Consider only these relation kinds.
    RelationKinds(Vec<RelationKind>),
    /// Consider relation kinds with explicit positive integer weights.
    WeightedRelationKinds(BTreeMap<RelationKind, u32>),
}

/// Scope preset that keeps code, configuration, and environment neighborhoods
/// connected while giving direct semantic links more influence.
pub fn environment_aware_scope() -> CommunityScope {
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
pub struct CommunitySummary {
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
pub struct CommunitySnapshot {
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
pub struct CommunitySnapshotStore {
    root: std::path::PathBuf,
}

impl CommunitySnapshotStore {
    /// Creates a store rooted at `.lithograph/analytics` or equivalent.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes only a changed versioned community snapshot.
    pub fn save(&self, snapshot: &CommunitySnapshot) -> std::io::Result<bool> {
        let payload = serde_json::to_string(snapshot).map_err(std::io::Error::other)?;
        let path = self.path(snapshot);
        if JsonStore.read::<String>(&path)?.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }

    /// Loads the exact persisted snapshot when present.
    pub fn load(&self, snapshot: &CommunitySnapshot) -> std::io::Result<Option<CommunitySnapshot>> {
        let Some(payload): Option<String> = JsonStore.read(&self.path(snapshot))? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(std::io::Error::other)
    }

    fn path(&self, snapshot: &CommunitySnapshot) -> std::path::PathBuf {
        let key = format!(
            "{}:{}:{:?}",
            snapshot.graph_snapshot_id, snapshot.algorithm_version, snapshot.scope
        );
        self.root
            .join(format!("{}.json", blake3::hash(key.as_bytes()).to_hex()))
    }
}

/// Version of deterministic topic-label semantics over node documents.
pub const TOPIC_ALGORITHM_VERSION: u32 = 1;

/// Topic labels attached to one detected community.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommunityTopic {
    /// Community identifier.
    pub community_id: String,
    /// Bounded labels ordered by descending score then token.
    pub labels: Vec<String>,
    /// Stable community membership copied from the community snapshot.
    pub members: Vec<GraphNodeId>,
}

/// Versioned topic/community overlay kept separate from graph resolver edges.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopicSnapshot {
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
pub struct TopicSnapshotStore {
    root: std::path::PathBuf,
}

impl TopicSnapshotStore {
    /// Creates a store rooted at `.lithograph/analytics` or equivalent.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes a topic snapshot only when its content changes.
    pub fn save(&self, snapshot: &TopicSnapshot) -> std::io::Result<bool> {
        let payload = serde_json::to_string(snapshot).map_err(std::io::Error::other)?;
        let path = self.path(snapshot);
        if JsonStore.read::<String>(&path)?.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }

    /// Loads a previously persisted topic snapshot.
    pub fn load(&self, snapshot: &TopicSnapshot) -> std::io::Result<Option<TopicSnapshot>> {
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
pub fn label_topic_snapshot(
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
pub fn leiden_communities(graph: &Graph, scope: &CommunityScope) -> Vec<CommunitySummary> {
    let mut adjacency = BTreeMap::<GraphNodeId, BTreeSet<GraphNodeId>>::new();
    let selected: Vec<_> = graph
        .relations
        .iter()
        .filter(|edge| in_scope(edge.kind, scope))
        .collect();
    let mut edge_weights = BTreeMap::<(GraphNodeId, GraphNodeId), u32>::new();
    for edge in &selected {
        adjacency
            .entry(edge.source.clone())
            .or_default()
            .insert(edge.target.clone());
        adjacency
            .entry(edge.target.clone())
            .or_default()
            .insert(edge.source.clone());
        let weight = relation_weight(edge.kind, scope);
        *edge_weights
            .entry((edge.source.clone(), edge.target.clone()))
            .or_default() += weight;
        *edge_weights
            .entry((edge.target.clone(), edge.source.clone()))
            .or_default() += weight;
    }
    let mut labels: BTreeMap<_, _> = adjacency
        .keys()
        .cloned()
        .map(|id| (id.clone(), id))
        .collect();
    let total_degree = adjacency
        .iter()
        .map(|(node, neighbors)| {
            neighbors
                .iter()
                .map(|neighbor| edge_weights[&(node.clone(), neighbor.clone())])
                .sum::<u32>()
        })
        .sum::<u32>() as f64;
    let mut volumes: BTreeMap<_, u32> = adjacency
        .iter()
        .map(|(node, neighbors)| {
            (
                node.clone(),
                neighbors
                    .iter()
                    .map(|neighbor| edge_weights[&(node.clone(), neighbor.clone())])
                    .sum(),
            )
        })
        .collect();
    for _ in 0..adjacency.len().max(1) {
        let mut moved = false;
        for node in adjacency.keys() {
            let degree: u32 = adjacency[node]
                .iter()
                .map(|neighbor| edge_weights[&(node.clone(), neighbor.clone())])
                .sum();
            let current = labels[node].clone();
            *volumes.entry(current.clone()).or_default() -= degree;
            let mut counts = BTreeMap::<GraphNodeId, u32>::new();
            for neighbor in &adjacency[node] {
                *counts.entry(labels[neighbor].clone()).or_default() +=
                    edge_weights[&(node.clone(), neighbor.clone())];
            }
            let next = counts
                .into_iter()
                .max_by(|a, b| {
                    let gain_a = a.1 as f64 - degree as f64 * volumes[&a.0] as f64 / total_degree;
                    let gain_b = b.1 as f64 - degree as f64 * volumes[&b.0] as f64 / total_degree;
                    gain_a.total_cmp(&gain_b).then_with(|| b.0.cmp(&a.0))
                })
                .map_or(current.clone(), |(label, _)| label);
            if current != next {
                labels.insert(node.clone(), next);
                moved = true;
            }
            *volumes.entry(labels[node].clone()).or_default() += degree;
        }
        if !moved {
            break;
        }
    }
    let mut groups = BTreeMap::<GraphNodeId, BTreeSet<GraphNodeId>>::new();
    for (node, label) in labels {
        groups.entry(label).or_default().insert(node);
    }
    let node_packages: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Package(package) => Some((package.id.clone(), package.name.clone())),
            _ => None,
        })
        .collect();
    let mut summaries: Vec<_> = groups
        .into_values()
        .filter(|group| group.len() >= 2)
        .map(|members| {
            let intra: Vec<_> = selected
                .iter()
                .filter(|edge| members.contains(&edge.source) && members.contains(&edge.target))
                .collect();
            let boundary: Vec<_> = selected
                .iter()
                .filter(|edge| members.contains(&edge.source) != members.contains(&edge.target))
                .collect();
            let mut internal_degree = BTreeMap::<GraphNodeId, usize>::new();
            let mut bridges = BTreeSet::new();
            for edge in &intra {
                *internal_degree.entry(edge.source.clone()).or_default() += 1;
                *internal_degree.entry(edge.target.clone()).or_default() += 1;
            }
            for edge in &boundary {
                if members.contains(&edge.source) {
                    bridges.insert(edge.source.clone());
                }
                if members.contains(&edge.target) {
                    bridges.insert(edge.target.clone());
                }
            }
            let mut representatives: Vec<_> = members.iter().cloned().collect();
            representatives
                .sort_by(|a, b| internal_degree[b].cmp(&internal_degree[a]).then(a.cmp(b)));
            // The preceding filter guarantees a non-empty set; retaining a
            // total fallback keeps the analytics path non-panicking if this
            // helper is ever reused with a different caller.
            let first = members
                .iter()
                .next()
                .map(ToString::to_string)
                .unwrap_or_default();
            let n = members.len() as f64;
            CommunitySummary {
                id: format!("leiden:{first}"),
                label: format!("Community {first}"),
                members: members.iter().cloned().collect(),
                cohesion: (intra.len() as f64 / (n * (n - 1.0) / 2.0)).min(1.0),
                conductance: boundary.len() as f64
                    / (2.0 * intra.len() as f64 + boundary.len() as f64).max(1.0),
                boundary_edges: boundary.iter().map(|edge| edge.id.clone()).collect(),
                representative_nodes: representatives.into_iter().take(5).collect(),
                dominant_packages: members
                    .iter()
                    .filter_map(|id| node_packages.get(id).cloned())
                    .collect(),
                bridge_nodes: bridges.into_iter().collect(),
            }
        })
        .collect();
    summaries.sort_by(|a, b| b.members.len().cmp(&a.members.len()).then(a.id.cmp(&b.id)));
    summaries
}

fn in_scope(kind: RelationKind, scope: &CommunityScope) -> bool {
    relation_weight(kind, scope) > 0
}

fn relation_weight(kind: RelationKind, scope: &CommunityScope) -> u32 {
    match scope {
        CommunityScope::Combined => 1,
        CommunityScope::RelationKinds(kinds) => u32::from(kinds.contains(&kind)),
        CommunityScope::WeightedRelationKinds(weights) => weights.get(&kind).copied().unwrap_or(0),
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
}
