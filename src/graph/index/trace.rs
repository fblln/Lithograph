//! Graph traversal (trace / impact analysis).

use super::KnowledgeIndex;
use super::common::search_result;
use super::search::SearchResult;
use crate::graph::{GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Trace traversal direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum TraceDirection {
    /// Follow inbound relations.
    Inbound,
    /// Follow outbound relations.
    Outbound,
    /// Follow both inbound and outbound relations.
    #[default]
    Both,
}

/// Trace traversal parameters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceParams {
    /// Node id, exact name, or query substring used to choose the root node.
    pub query: String,
    /// Traversal depth. Defaults to 2 when zero.
    #[serde(default)]
    pub depth: usize,
    /// Traversal direction.
    #[serde(default)]
    pub direction: TraceDirection,
}

/// Graph trace output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceResult {
    /// Root search result.
    pub root: SearchResult,
    /// Visited nodes with hop distance.
    pub visited: Vec<NodeHop>,
    /// Relations connecting visited nodes.
    pub relations: Vec<TraceRelation>,
}

/// One visited node and its hop distance from the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeHop {
    /// Node information.
    pub node: SearchResult,
    /// Hop distance from the root.
    pub hop: usize,
}

/// One relation included in a trace result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TraceRelation {
    /// Source node id.
    pub source: GraphNodeId,
    /// Target node id.
    pub target: GraphNodeId,
    /// Relation kind.
    pub kind: RelationKind,
}

impl<'a> KnowledgeIndex<'a> {
    /// Traces the graph around the first node matching `params.query`.
    pub fn trace(&self, params: &TraceParams) -> Option<TraceResult> {
        let root = self.find_root(params.query.as_str())?;
        let degree = self.degree_index();
        let adjacency = self.adjacency(params.direction);
        let max_depth = if params.depth == 0 {
            2
        } else {
            params.depth.min(5)
        };
        let mut seen: BTreeSet<GraphNodeId> = BTreeSet::new();
        let mut queue = VecDeque::new();
        seen.insert(root.id().clone());
        queue.push_back((root.id().clone(), 0usize));

        while let Some((id, hop)) = queue.pop_front() {
            if hop >= max_depth {
                continue;
            }
            for next in adjacency.get(&id).into_iter().flatten() {
                if seen.insert(next.clone()) {
                    queue.push_back((next.clone(), hop + 1));
                }
            }
        }

        let node_by_id = self.node_by_id();
        let mut visited: Vec<NodeHop> = seen
            .iter()
            .filter_map(|id| node_by_id.get(id).map(|node| (id, node)))
            .map(|(id, node)| NodeHop {
                node: search_result(node, &degree),
                hop: shortest_hop(root.id(), id, &adjacency, max_depth).unwrap_or(0),
            })
            .collect();
        visited.sort_by(|a, b| a.hop.cmp(&b.hop).then(a.node.id.cmp(&b.node.id)));

        let relations = self
            .graph
            .relations
            .iter()
            .filter(|relation| seen.contains(&relation.source) && seen.contains(&relation.target))
            .map(|relation| TraceRelation {
                source: relation.source.clone(),
                target: relation.target.clone(),
                kind: relation.kind,
            })
            .collect();

        Some(TraceResult {
            root: search_result(root, &degree),
            visited,
            relations,
        })
    }

    /// Traces everything that (transitively) depends on the node matching
    /// `params.query` -- "what breaks if this changes." A thin wrapper over
    /// [`Self::trace`] that always uses [`TraceDirection::Inbound`]
    /// regardless of `params.direction`, since "impact" only ever means
    /// upstream dependents, never downstream dependencies.
    pub fn impact_analysis(&self, params: &TraceParams) -> Option<TraceResult> {
        self.trace(&TraceParams {
            query: params.query.clone(),
            depth: params.depth,
            direction: TraceDirection::Inbound,
        })
    }

    fn adjacency(&self, direction: TraceDirection) -> BTreeMap<GraphNodeId, Vec<GraphNodeId>> {
        let mut adjacency: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
        for relation in &self.graph.relations {
            if matches!(direction, TraceDirection::Outbound | TraceDirection::Both) {
                adjacency
                    .entry(relation.source.clone())
                    .or_default()
                    .push(relation.target.clone());
            }
            if matches!(direction, TraceDirection::Inbound | TraceDirection::Both) {
                adjacency
                    .entry(relation.target.clone())
                    .or_default()
                    .push(relation.source.clone());
            }
        }
        for neighbors in adjacency.values_mut() {
            neighbors.sort();
            neighbors.dedup();
        }
        adjacency
    }
}

fn shortest_hop(
    root: &GraphNodeId,
    target: &GraphNodeId,
    adjacency: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    max_depth: usize,
) -> Option<usize> {
    if root == target {
        return Some(0);
    }
    let mut seen = BTreeSet::new();
    let mut queue = VecDeque::new();
    seen.insert(root.clone());
    queue.push_back((root.clone(), 0usize));
    while let Some((id, hop)) = queue.pop_front() {
        if hop >= max_depth {
            continue;
        }
        for next in adjacency.get(&id).into_iter().flatten() {
            if next == target {
                return Some(hop + 1);
            }
            if seen.insert(next.clone()) {
                queue.push_back((next.clone(), hop + 1));
            }
        }
    }
    None
}
