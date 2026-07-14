//! Functional architecture communities (connected components over
//! call/import/package-dependency edges).

use super::KnowledgeIndex;
use super::common::search_result;
use super::search::SearchResult;
use crate::graph::{GraphNode, GraphNodeId, Relation, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Relation kinds counted as "functional" architecture edges for
/// [`KnowledgeIndex::clusters`] (LIT-22.5.1 AC1) -- call, import, and
/// package-dependency relations connect nodes that work together.
/// Structural edges (`Contains`, `BelongsToModule`) are deliberately
/// excluded: every symbol in a file already shares a `Contains` edge with
/// its artifact, which would collapse every cluster into "one per file."
const CLUSTER_EDGE_KINDS: &[RelationKind] = &[
    RelationKind::Calls,
    RelationKind::Imports,
    RelationKind::DependsOnPackage,
    RelationKind::BelongsToPackage,
];

/// Maximum members surfaced in [`ArchitectureCluster::top_nodes`].
const CLUSTER_TOP_NODES: usize = 5;

/// One functional architecture community (LIT-22.5.1): a connected
/// component over [`CLUSTER_EDGE_KINDS`] edges.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureCluster {
    /// Stable id, derived from the cluster's lexicographically-smallest
    /// member id (deterministic across runs of an unchanged graph).
    pub id: String,
    /// Every member node id.
    pub members: Vec<GraphNodeId>,
    /// The highest-degree members, most-connected first.
    pub top_nodes: Vec<SearchResult>,
    /// Package names touched by this cluster's members.
    pub packages: Vec<String>,
    /// Relation kinds observed between two members of this cluster
    /// (debug-formatted, e.g. `"Calls"`, `"Imports"`).
    pub edge_types: Vec<String>,
    /// Intra-cluster edge density: actual edges between members divided by
    /// the maximum possible (`members * (members - 1) / 2`), clamped to
    /// `[0.0, 1.0]`. Higher means more tightly interconnected.
    pub cohesion: f64,
}

impl<'a> KnowledgeIndex<'a> {
    /// Groups the graph into functional architecture communities: connected
    /// components over [`CLUSTER_EDGE_KINDS`] edges (LIT-22.5.1). Only
    /// components with 2+ members are reported -- an isolated node touched
    /// by no qualifying edge isn't a "community." Deterministic: cluster
    /// ids and member order derive only from node/relation ids already
    /// sorted by `Graph::to_json`'s invariants, never from map iteration
    /// order or wall-clock/random tie-breaks.
    pub fn clusters(&self) -> Vec<ArchitectureCluster> {
        let mut parent: BTreeMap<GraphNodeId, GraphNodeId> = BTreeMap::new();
        for relation in &self.graph.relations {
            if !CLUSTER_EDGE_KINDS.contains(&relation.kind) {
                continue;
            }
            parent
                .entry(relation.source.clone())
                .or_insert_with(|| relation.source.clone());
            parent
                .entry(relation.target.clone())
                .or_insert_with(|| relation.target.clone());
            union(&mut parent, &relation.source, &relation.target);
        }

        let member_ids: Vec<GraphNodeId> = parent.keys().cloned().collect();
        let mut groups: BTreeMap<GraphNodeId, BTreeSet<GraphNodeId>> = BTreeMap::new();
        for id in &member_ids {
            let root = find_root(&mut parent, id);
            groups.entry(root).or_default().insert(id.clone());
        }

        let node_by_id = self.node_by_id();
        let degree = self.degree_index();
        let mut clusters: Vec<ArchitectureCluster> = groups
            .into_values()
            .filter(|members| members.len() >= 2)
            .map(|members| build_cluster(&members, &self.graph.relations, &node_by_id, &degree))
            .collect();
        clusters.sort_by(|a, b| b.members.len().cmp(&a.members.len()).then(a.id.cmp(&b.id)));
        clusters
    }
}

/// Union-find root lookup with path compression.
fn find_root(parent: &mut BTreeMap<GraphNodeId, GraphNodeId>, id: &GraphNodeId) -> GraphNodeId {
    let mut root = id.clone();
    while let Some(next) = parent.get(&root) {
        if next == &root {
            break;
        }
        root = next.clone();
    }
    let mut node = id.clone();
    while let Some(next) = parent.get(&node).cloned() {
        if next == node {
            break;
        }
        parent.insert(node, root.clone());
        node = next;
    }
    root
}

/// Union-find merge. Always attaches the lexicographically-larger root
/// under the smaller one, so the result never depends on relation
/// iteration order -- only on the node ids themselves.
fn union(parent: &mut BTreeMap<GraphNodeId, GraphNodeId>, a: &GraphNodeId, b: &GraphNodeId) {
    let root_a = find_root(parent, a);
    let root_b = find_root(parent, b);
    if root_a == root_b {
        return;
    }
    let (keep, merge) = if root_a <= root_b {
        (root_a, root_b)
    } else {
        (root_b, root_a)
    };
    parent.insert(merge, keep);
}

fn build_cluster(
    members: &BTreeSet<GraphNodeId>,
    relations: &[Relation],
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
    degree: &BTreeMap<&GraphNodeId, (usize, usize)>,
) -> ArchitectureCluster {
    let intra_edges: Vec<&Relation> = relations
        .iter()
        .filter(|relation| members.contains(&relation.source) && members.contains(&relation.target))
        .collect();
    let edge_types: BTreeSet<String> = intra_edges
        .iter()
        .map(|relation| format!("{:?}", relation.kind))
        .collect();
    let packages: BTreeSet<String> = members
        .iter()
        .filter_map(|id| node_by_id.get(id))
        .filter_map(|node| match node {
            GraphNode::Package(package) => Some(package.name.clone()),
            _ => None,
        })
        .collect();

    let mut member_results: Vec<SearchResult> = members
        .iter()
        .filter_map(|id| node_by_id.get(id))
        .map(|node| search_result(node, degree))
        .collect();
    member_results.sort_by(|a, b| {
        (b.in_degree + b.out_degree)
            .cmp(&(a.in_degree + a.out_degree))
            .then(a.id.cmp(&b.id))
    });
    let top_nodes = member_results.into_iter().take(CLUSTER_TOP_NODES).collect();

    let member_count = members.len() as f64;
    let max_edges = member_count * (member_count - 1.0) / 2.0;
    let cohesion = if max_edges > 0.0 {
        (intra_edges.len() as f64 / max_edges).min(1.0)
    } else {
        0.0
    };

    // Safe: `members` is non-empty whenever `build_cluster` is called (the
    // caller already filtered to `members.len() >= 2`), and `BTreeSet`
    // iterates in sorted order, so this is always the lexicographically
    // smallest member id.
    let id = members
        .iter()
        .next()
        .map_or_else(String::new, |first| format!("cluster:{first}"));

    ArchitectureCluster {
        id,
        members: members.iter().cloned().collect(),
        top_nodes,
        packages: packages.into_iter().collect(),
        edge_types: edge_types.into_iter().collect(),
        cohesion,
    }
}
