//! Graph traversal (trace / impact analysis).

use super::KnowledgeIndex;
use super::common::search_result;
use super::search::SearchResult;
use crate::graph::{GraphNodeId, Relation, RelationKind, RelationResolution};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Trace traversal direction.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum TraceDirection {
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
pub(crate) struct TraceParams {
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
pub(crate) struct TraceResult {
    /// Root search result.
    pub root: SearchResult,
    /// Visited nodes with hop distance.
    pub visited: Vec<NodeHop>,
    /// Relations connecting visited nodes.
    pub relations: Vec<TraceRelation>,
}

/// One visited node and its hop distance from the root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NodeHop {
    /// Node information.
    pub node: SearchResult,
    /// Hop distance from the root.
    pub hop: usize,
}

/// Shortest chain of relations between two nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PathResult {
    /// Node the path starts from.
    pub start: SearchResult,
    /// Each step away from `start`, in order. Empty when both ends resolve
    /// to the same node.
    pub hops: Vec<PathHop>,
}

/// One step along a [`PathResult`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PathHop {
    /// Node reached by this step.
    pub node: SearchResult,
    /// Kind of the relation traversed.
    pub kind: RelationKind,
    /// Whether the relation points from the previous node to this one.
    /// Paths follow relations in both directions, so a hop may traverse a
    /// relation against its direction.
    pub forward: bool,
    /// How the traversed relation was resolved, when it records provenance.
    /// Distinguishes a proven connection from a syntax-only guess, which is
    /// the difference between trusting a path and checking it.
    pub resolution: Option<RelationResolution>,
}

/// One relation included in a trace result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TraceRelation {
    /// Source node id.
    pub source: GraphNodeId,
    /// Target node id.
    pub target: GraphNodeId,
    /// Relation kind.
    pub kind: RelationKind,
}

impl<'a> KnowledgeIndex<'a> {
    /// Traces the graph around the first node matching `params.query`.
    pub(crate) fn trace(&self, params: &TraceParams) -> Option<TraceResult> {
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

    /// Finds the shortest relation chain connecting the nodes matching
    /// `from` and `to`, or `None` when either end matches no node or no
    /// chain joins them.
    ///
    /// Relations are followed in both directions: "how do these two things
    /// connect" is a question about the undirected shape of the graph, and a
    /// caller can read each hop's [`PathHop::forward`] to see which way the
    /// underlying relation actually points. Ties are broken by node id so a
    /// given graph always yields the same path (LIT-47).
    pub(crate) fn shortest_path(&self, from: &str, to: &str) -> Option<PathResult> {
        let start = self.find_root(from)?;
        let end = self.find_root(to)?;
        let degree = self.degree_index();
        let start_id = start.id().clone();
        let end_id = end.id().clone();

        // Breadth-first from `start`, remembering how each node was first
        // reached so the chain can be walked back once `end` is found. BFS
        // over an unweighted graph yields a minimal-hop chain.
        let adjacency = self.adjacency(TraceDirection::Both);
        let mut came_from: BTreeMap<GraphNodeId, GraphNodeId> = BTreeMap::new();
        let mut seen: BTreeSet<GraphNodeId> = BTreeSet::new();
        let mut queue = VecDeque::new();
        seen.insert(start_id.clone());
        queue.push_back(start_id.clone());
        while let Some(id) = queue.pop_front() {
            if id == end_id {
                break;
            }
            for next in adjacency.get(&id).into_iter().flatten() {
                if seen.insert(next.clone()) {
                    came_from.insert(next.clone(), id.clone());
                    queue.push_back(next.clone());
                }
            }
        }
        if start_id != end_id && !came_from.contains_key(&end_id) {
            return None;
        }

        let mut ids = vec![end_id.clone()];
        while let Some(previous) = came_from.get(ids.last()?) {
            ids.push(previous.clone());
        }
        ids.reverse();

        let hops = ids
            .windows(2)
            .filter_map(|pair| {
                let relation = self.connecting_relation(&pair[0], &pair[1])?;
                let node = self.graph.nodes.iter().find(|node| node.id() == &pair[1])?;
                Some(PathHop {
                    node: search_result(node, &degree),
                    kind: relation.kind,
                    forward: relation.source == pair[0],
                    resolution: relation
                        .provenance
                        .as_ref()
                        .map(|provenance| provenance.resolution),
                })
            })
            .collect::<Vec<_>>();
        if hops.len() + 1 != ids.len() {
            return None;
        }

        Some(PathResult {
            start: search_result(
                self.graph
                    .nodes
                    .iter()
                    .find(|node| node.id() == &start_id)?,
                &degree,
            ),
            hops,
        })
    }

    /// The relation joining two adjacent nodes, in either direction. Picks
    /// the lowest relation id when several connect the same pair, so the
    /// reported hop is stable rather than dependent on relation order.
    fn connecting_relation(&self, left: &GraphNodeId, right: &GraphNodeId) -> Option<&'a Relation> {
        self.graph
            .relations
            .iter()
            .filter(|relation| {
                (&relation.source == left && &relation.target == right)
                    || (&relation.source == right && &relation.target == left)
            })
            .min_by(|a, b| a.id.cmp(&b.id))
    }

    /// Traces everything that (transitively) depends on the node matching
    /// `params.query` -- "what breaks if this changes." A thin wrapper over
    /// [`Self::trace`] that always uses [`TraceDirection::Inbound`]
    /// regardless of `params.direction`, since "impact" only ever means
    /// upstream dependents, never downstream dependencies.
    pub(crate) fn impact_analysis(&self, params: &TraceParams) -> Option<TraceResult> {
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
