//! Server-side deterministic graph layout and focused-subgraph extraction
//! (LIT-24.16). Positions a bounded, budgeted slice of the graph so a UI
//! never has to load or lay out an entire repository graph by default.
//!
//! Two request shapes share one code path: an overview request (no
//! `center_node`) covers the whole graph from a deterministic pseudo-root,
//! while a detail request positions a hop-limited neighborhood around a
//! resolved node. Both are budgeted independently by node count and edge
//! count -- edge count, not node count, is what first blows up rendering
//! and layout cost, so it gets its own budget rather than riding on the
//! node budget.

use crate::graph::index::{node_file_path, node_label, node_name};
use crate::graph::{Graph, GraphNode, GraphNodeId, KnowledgeIndex, Relation, RelationKind};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Versioned deterministic layout semantics. Bumped whenever positioning
/// or budgeting rules change, so a cached result computed under an older
/// version is never served as if it were current.
pub const LAYOUT_ALGORITHM_VERSION: u32 = 1;

/// Default node budget applied when `max_nodes` is unset.
const DEFAULT_NODE_BUDGET: usize = 150;
/// Hard ceiling on a caller-requested node budget.
const MAX_NODE_BUDGET: usize = 2000;
/// Default edge budget applied when `max_edges` is unset. Deliberately
/// larger relative to the node default than a 1:1 ratio, since a bounded
/// node set can still carry a much larger edge count.
const DEFAULT_EDGE_BUDGET: usize = 400;
/// Hard ceiling on a caller-requested edge budget.
const MAX_EDGE_BUDGET: usize = 6000;
/// Default and maximum hop radius for a focused (detail) request.
const DEFAULT_RADIUS: usize = 2;
const MAX_RADIUS: usize = 5;
/// Pixel spacing between concentric hop rings in the positioned output.
const RING_SPACING: f64 = 120.0;

/// One layout/subgraph request. `center_node` absent selects overview
/// mode (the whole graph, budgeted, laid out from a deterministic
/// pseudo-root); present selects detail mode (a focused neighborhood
/// around the node it resolves to).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LayoutRequest {
    /// Node id, exact name, or substring used to resolve the focus node
    /// (matched the same flexible way as `trace_path`). `None` selects
    /// overview mode.
    #[serde(default)]
    pub center_node: Option<String>,
    /// Hop radius from `center_node`. Ignored in overview mode. Defaults
    /// to 2 when zero, clamped to 5.
    #[serde(default)]
    pub radius: usize,
    /// Maximum nodes returned. Defaults to 150 when zero, clamped to 2000.
    #[serde(default)]
    pub max_nodes: usize,
    /// Maximum edges returned. Defaults to 400 when zero, clamped to 6000.
    #[serde(default)]
    pub max_edges: usize,
    /// Node label allowlist (e.g. `"Symbol"`, `"Artifact"`), matched
    /// case-insensitively. Empty means no filter. The resolved focus node
    /// is always included regardless of this filter.
    #[serde(default)]
    pub node_labels: BTreeSet<String>,
    /// Relation kind allowlist restricting which edges are traversed and
    /// returned. Empty means no filter.
    #[serde(default)]
    pub edge_types: BTreeSet<RelationKind>,
}

/// One positioned node in a layout result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PositionedNode {
    /// Node id.
    pub id: GraphNodeId,
    /// Node label (e.g. `"Symbol"`, `"Artifact"`).
    pub label: String,
    /// Human-readable name.
    pub name: String,
    /// Repository-relative file path when the node has one.
    pub file_path: Option<String>,
    /// Inbound relation count in the full graph (not just the returned slice).
    pub in_degree: usize,
    /// Outbound relation count in the full graph (not just the returned slice).
    pub out_degree: usize,
    /// Deterministic layout x coordinate.
    pub x: f64,
    /// Deterministic layout y coordinate.
    pub y: f64,
    /// Hop distance from the layout's origin (0 for the center/pseudo-root).
    pub hop: usize,
}

/// One relation included in a layout result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutEdge {
    /// Source node id.
    pub source: GraphNodeId,
    /// Target node id.
    pub target: GraphNodeId,
    /// Relation kind.
    pub kind: RelationKind,
}

/// Explicit budget accounting for one layout response. Every truncation is
/// reported, never silent (LIT-24.16 AC3/AC5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LayoutBudget {
    /// Node budget actually applied (after defaulting/clamping).
    pub node_budget: usize,
    /// Edge budget actually applied (after defaulting/clamping).
    pub edge_budget: usize,
    /// Nodes matching the request's scope and filters, before truncation.
    pub nodes_available: usize,
    /// Edges matching the request's scope and filters, before truncation.
    pub edges_available: usize,
    /// Nodes actually returned.
    pub nodes_returned: usize,
    /// Edges actually returned.
    pub edges_returned: usize,
    /// True when `nodes_available > nodes_returned`.
    pub nodes_truncated: bool,
    /// True when `edges_available > edges_returned`.
    pub edges_truncated: bool,
}

/// A computed layout: a budgeted, positioned graph slice.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutResult {
    /// Content-derived id of the graph this layout was computed from.
    pub graph_snapshot_id: String,
    /// Layout algorithm version this result was computed under.
    pub algorithm_version: u32,
    /// Resolved focus node id, or `None` for an overview request.
    pub center_node: Option<GraphNodeId>,
    /// Positioned nodes.
    pub nodes: Vec<PositionedNode>,
    /// Relations between returned nodes.
    pub edges: Vec<LayoutEdge>,
    /// Budget accounting for this response.
    pub budget: LayoutBudget,
}

/// A persisted, versioned layout computation, keyed by graph snapshot,
/// algorithm version, and the exact request that produced it (AC2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSnapshot {
    /// Content-derived id of the graph this layout was computed from.
    pub graph_snapshot_id: String,
    /// Layout algorithm version this result was computed under.
    pub algorithm_version: u32,
    /// The exact request that produced `result`.
    pub request: LayoutRequest,
    /// The computed layout.
    pub result: LayoutResult,
}

/// Deterministically persists layout results and skips identical writes,
/// so a repeated request against an unchanged graph is a cache hit instead
/// of a recomputation.
#[derive(Debug, Clone)]
pub struct LayoutSnapshotStore {
    root: std::path::PathBuf,
}

impl LayoutSnapshotStore {
    /// Creates a store rooted at (typically) `.lithograph/layout`.
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes a layout snapshot only when its content changes.
    pub fn save(&self, snapshot: &LayoutSnapshot) -> std::io::Result<bool> {
        let path = self.path(
            &snapshot.graph_snapshot_id,
            snapshot.algorithm_version,
            &snapshot.request,
        );
        JsonStore.write_if_changed(&path, snapshot)
    }

    /// Loads a previously persisted layout snapshot for this exact key.
    pub fn load(
        &self,
        graph_snapshot_id: &str,
        algorithm_version: u32,
        request: &LayoutRequest,
    ) -> std::io::Result<Option<LayoutSnapshot>> {
        JsonStore.read(&self.path(graph_snapshot_id, algorithm_version, request))
    }

    fn path(
        &self,
        graph_snapshot_id: &str,
        algorithm_version: u32,
        request: &LayoutRequest,
    ) -> std::path::PathBuf {
        let key = format!("{graph_snapshot_id}:{algorithm_version}:{request:?}");
        self.root
            .join(format!("{}.json", blake3::hash(key.as_bytes()).to_hex()))
    }
}

/// Computes a layout, transparently serving a cached result when the graph
/// snapshot, algorithm version, and exact request all match a previous
/// computation (AC2). A missing or unreadable cache entry is treated as a
/// miss, not a failure -- a corrupt cache must never block computing a
/// fresh result, and a failed cache write must never fail the request.
pub fn compute_layout_cached(
    graph: &Graph,
    request: &LayoutRequest,
    store: &LayoutSnapshotStore,
) -> Result<LayoutResult, String> {
    let snapshot_id = graph_snapshot_id(graph)?;
    if let Ok(Some(cached)) = store.load(&snapshot_id, LAYOUT_ALGORITHM_VERSION, request) {
        return Ok(cached.result);
    }
    let result = compute_layout(graph, request)?;
    let snapshot = LayoutSnapshot {
        graph_snapshot_id: snapshot_id,
        algorithm_version: LAYOUT_ALGORITHM_VERSION,
        request: request.clone(),
        result: result.clone(),
    };
    let _ = store.save(&snapshot);
    Ok(result)
}

/// Computes a layout without consulting or populating any cache.
pub fn compute_layout(graph: &Graph, request: &LayoutRequest) -> Result<LayoutResult, String> {
    let snapshot_id = graph_snapshot_id(graph)?;

    if request.center_node.is_none() && graph.nodes.is_empty() {
        return Ok(empty_result(snapshot_id, request));
    }

    let index = KnowledgeIndex::new(graph);
    let node_by_id = index.node_by_id();
    let degree = index.degree_index();

    let origin = match &request.center_node {
        Some(query) => Some(
            index
                .find_root(query)
                .ok_or_else(|| format!("no graph node matched `{query}`"))?
                .id()
                .clone(),
        ),
        None => None,
    };

    let label_filter: Option<BTreeSet<String>> = if request.node_labels.is_empty() {
        None
    } else {
        Some(
            request
                .node_labels
                .iter()
                .map(|label| label.to_lowercase())
                .collect(),
        )
    };
    let edge_filter: Option<&BTreeSet<RelationKind>> = if request.edge_types.is_empty() {
        None
    } else {
        Some(&request.edge_types)
    };

    let adjacency = build_adjacency(graph, edge_filter);
    let hops = match &origin {
        Some(center) => {
            let radius = if request.radius == 0 {
                DEFAULT_RADIUS
            } else {
                request.radius.min(MAX_RADIUS)
            };
            bfs_hops(center, &adjacency, radius)
        }
        None => full_hop_ranking(graph, &pseudo_root(graph, &degree), &adjacency),
    };
    // In overview mode with an edge filter, some nodes may share no
    // filtered edge with anything; `full_hop_ranking` still assigns them a
    // hop (its overflow ring), so they remain valid layout candidates.

    let mut candidates: Vec<&GraphNodeId> = hops
        .keys()
        .filter(|id| {
            origin.as_ref() == Some(*id)
                || label_filter.as_ref().is_none_or(|labels| {
                    node_by_id
                        .get(*id)
                        .is_some_and(|node| labels.contains(&node_label(node).to_lowercase()))
                })
        })
        .collect();
    // Deterministic priority: closest hop first, then most-connected, then
    // id -- so truncating to the node budget always keeps the closest and
    // best-connected nodes rather than an arbitrary subset.
    candidates.sort_by(|a, b| {
        hops[*a]
            .cmp(&hops[*b])
            .then_with(|| total_degree(&degree, b).cmp(&total_degree(&degree, a)))
            .then_with(|| a.cmp(b))
    });

    let nodes_available = candidates.len();
    let node_budget = resolve_node_budget(request.max_nodes);
    let nodes_truncated = nodes_available > node_budget;
    candidates.truncate(node_budget);
    let selected: BTreeSet<GraphNodeId> = candidates.iter().map(|id| (*id).clone()).collect();

    let mut edges: Vec<&Relation> = graph
        .relations
        .iter()
        .filter(|relation| {
            selected.contains(&relation.source) && selected.contains(&relation.target)
        })
        .filter(|relation| edge_filter.is_none_or(|kinds| kinds.contains(&relation.kind)))
        .collect();
    edges.sort_by(|a, b| {
        a.source
            .cmp(&b.source)
            .then(a.kind.cmp(&b.kind))
            .then(a.target.cmp(&b.target))
    });
    let edges_available = edges.len();
    let edge_budget = resolve_edge_budget(request.max_edges);
    let edges_truncated = edges_available > edge_budget;
    edges.truncate(edge_budget);

    let positions = radial_positions(&candidates, &hops);
    let nodes: Vec<PositionedNode> = candidates
        .iter()
        .filter_map(|id| {
            let node = node_by_id.get(*id)?;
            let (in_degree, out_degree) = degree.get(*id).copied().unwrap_or((0, 0));
            let (x, y) = positions.get(*id).copied().unwrap_or((0.0, 0.0));
            Some(PositionedNode {
                id: (*id).clone(),
                label: node_label(node).to_owned(),
                name: node_name(node),
                file_path: node_file_path(node),
                in_degree,
                out_degree,
                x,
                y,
                hop: hops[*id],
            })
        })
        .collect();
    let layout_edges: Vec<LayoutEdge> = edges
        .iter()
        .map(|relation| LayoutEdge {
            source: relation.source.clone(),
            target: relation.target.clone(),
            kind: relation.kind,
        })
        .collect();

    let nodes_returned = nodes.len();
    let edges_returned = layout_edges.len();
    Ok(LayoutResult {
        graph_snapshot_id: snapshot_id,
        algorithm_version: LAYOUT_ALGORITHM_VERSION,
        center_node: origin,
        nodes,
        edges: layout_edges,
        budget: LayoutBudget {
            node_budget,
            edge_budget,
            nodes_available,
            edges_available,
            nodes_returned,
            edges_returned,
            nodes_truncated,
            edges_truncated,
        },
    })
}

fn empty_result(graph_snapshot_id: String, request: &LayoutRequest) -> LayoutResult {
    LayoutResult {
        graph_snapshot_id,
        algorithm_version: LAYOUT_ALGORITHM_VERSION,
        center_node: None,
        nodes: vec![],
        edges: vec![],
        budget: LayoutBudget {
            node_budget: resolve_node_budget(request.max_nodes),
            edge_budget: resolve_edge_budget(request.max_edges),
            nodes_available: 0,
            edges_available: 0,
            nodes_returned: 0,
            edges_returned: 0,
            nodes_truncated: false,
            edges_truncated: false,
        },
    }
}

fn resolve_node_budget(requested: usize) -> usize {
    if requested == 0 {
        DEFAULT_NODE_BUDGET
    } else {
        requested.min(MAX_NODE_BUDGET)
    }
}

fn resolve_edge_budget(requested: usize) -> usize {
    if requested == 0 {
        DEFAULT_EDGE_BUDGET
    } else {
        requested.min(MAX_EDGE_BUDGET)
    }
}

fn total_degree(degree: &BTreeMap<&GraphNodeId, (usize, usize)>, id: &GraphNodeId) -> usize {
    degree
        .get(id)
        .map(|(in_d, out_d)| in_d + out_d)
        .unwrap_or(0)
}

/// Undirected adjacency honoring an optional edge-kind filter. Traversal
/// for neighborhood/radius purposes is always undirected, matching
/// `trace_path`'s `TraceDirection::Both` default and the graph-explorer
/// prototype's `neighborsWithin`; direction-sensitive traversal is already
/// covered separately by `trace_path`/`impact_analysis`.
fn build_adjacency(
    graph: &Graph,
    edge_filter: Option<&BTreeSet<RelationKind>>,
) -> BTreeMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut adjacency: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
    for relation in &graph.relations {
        if edge_filter.is_some_and(|kinds| !kinds.contains(&relation.kind)) {
            continue;
        }
        adjacency
            .entry(relation.source.clone())
            .or_default()
            .push(relation.target.clone());
        adjacency
            .entry(relation.target.clone())
            .or_default()
            .push(relation.source.clone());
    }
    for neighbors in adjacency.values_mut() {
        neighbors.sort();
        neighbors.dedup();
    }
    adjacency
}

fn bfs_hops(
    root: &GraphNodeId,
    adjacency: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    max_depth: usize,
) -> BTreeMap<GraphNodeId, usize> {
    let mut hops = BTreeMap::new();
    hops.insert(root.clone(), 0usize);
    let mut queue = VecDeque::new();
    queue.push_back((root.clone(), 0usize));
    while let Some((id, hop)) = queue.pop_front() {
        if hop >= max_depth {
            continue;
        }
        for next in adjacency.get(&id).into_iter().flatten() {
            if !hops.contains_key(next) {
                hops.insert(next.clone(), hop + 1);
                queue.push_back((next.clone(), hop + 1));
            }
        }
    }
    hops
}

/// Hop-ranks every node in the graph from `root`, not just those within a
/// bounded radius. Nodes unreachable from `root` (disconnected components)
/// are placed in one overflow ring beyond the farthest reached hop,
/// ordered deterministically by id, so overview mode still positions and
/// considers every node in the graph rather than silently dropping
/// disconnected ones.
fn full_hop_ranking(
    graph: &Graph,
    root: &GraphNodeId,
    adjacency: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
) -> BTreeMap<GraphNodeId, usize> {
    let mut hops = bfs_hops(root, adjacency, usize::MAX);
    let overflow_hop = hops.values().max().copied().unwrap_or(0) + 1;
    for node in &graph.nodes {
        hops.entry(node.id().clone()).or_insert(overflow_hop);
    }
    hops
}

/// Deterministic pseudo-root for overview layouts: the highest total-degree
/// node, tie-broken by id so the choice never depends on iteration order.
fn pseudo_root(graph: &Graph, degree: &BTreeMap<&GraphNodeId, (usize, usize)>) -> GraphNodeId {
    let mut nodes: Vec<&GraphNodeId> = graph.nodes.iter().map(GraphNode::id).collect();
    nodes.sort_by(|a, b| {
        total_degree(degree, b)
            .cmp(&total_degree(degree, a))
            .then(a.cmp(b))
    });
    // `graph.nodes` is non-empty on every call site (empty graphs take the
    // dedicated `empty_result` path before reaching here).
    nodes
        .first()
        .copied()
        .cloned()
        .unwrap_or_else(|| GraphNodeId::new(""))
}

/// Concentric-ring layout: nodes at hop 0 sit at the origin, and each
/// successive hop ring sits at `hop * RING_SPACING` from the origin with
/// its members evenly spaced by angle in deterministic (sorted-id) order.
///
/// ponytail: this is a plain geometric placement, not a force-directed
/// simulation -- it avoids edge crossings within a ring but not across
/// rings. Deliberate: it is O(n log n), fully deterministic (needed for
/// caching and tests), and requires no client-visible iteration state.
/// Upgrade path if visual quality on dense graphs becomes a problem: run a
/// bounded force-relaxation pass over these positions as a second stage.
fn radial_positions(
    ordered_ids: &[&GraphNodeId],
    hops: &BTreeMap<GraphNodeId, usize>,
) -> BTreeMap<GraphNodeId, (f64, f64)> {
    let mut by_hop: BTreeMap<usize, Vec<&GraphNodeId>> = BTreeMap::new();
    for id in ordered_ids {
        by_hop.entry(hops[*id]).or_default().push(id);
    }
    let mut positions = BTreeMap::new();
    for (hop, mut ring) in by_hop {
        ring.sort();
        if hop == 0 {
            for id in ring {
                positions.insert((*id).clone(), (0.0, 0.0));
            }
            continue;
        }
        let radius = hop as f64 * RING_SPACING;
        let count = ring.len().max(1) as f64;
        for (index, id) in ring.into_iter().enumerate() {
            let angle = 2.0 * std::f64::consts::PI * (index as f64) / count;
            positions.insert((*id).clone(), (radius * angle.cos(), radius * angle.sin()));
        }
    }
    positions
}

fn graph_snapshot_id(graph: &Graph) -> Result<String, String> {
    let payload = graph
        .to_json()
        .map_err(|error| format!("failed to hash graph snapshot: {error}"))?;
    Ok(format!(
        "blake3:{}",
        blake3::hash(payload.as_bytes()).to_hex()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactCategory, ArtifactId, Confidence, EvidenceRef, RepoPath};
    use crate::graph::{ArtifactNode, SymbolNode};

    fn evidence_for(path: &str) -> Result<EvidenceRef, Box<dyn std::error::Error>> {
        let repo_path = RepoPath::new(path)?;
        Ok(EvidenceRef::file(
            ArtifactId::from_path(&repo_path),
            repo_path,
        ))
    }

    fn artifact(id: &str, path: &str) -> Result<GraphNode, Box<dyn std::error::Error>> {
        Ok(GraphNode::Artifact(ArtifactNode {
            id: GraphNodeId::new(id),
            path: path.to_owned(),
            category: ArtifactCategory::SourceCode,
            evidence: evidence_for(path)?,
        }))
    }

    fn symbol(id: &str, name: &str, path: &str) -> Result<GraphNode, Box<dyn std::error::Error>> {
        Ok(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(id),
            kind: crate::graph::SymbolKind::Function,
            qualified_name: name.to_owned(),
            doc: None,
            evidence: evidence_for(path)?,
        }))
    }

    fn edge(id: &str, source: &str, target: &str, kind: RelationKind) -> Relation {
        Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence: Confidence::High,
            evidence: vec![],
            provenance: None,
        }
    }

    /// `a -> b -> c -> d` chain plus a disconnected `e`.
    fn chain_graph() -> Result<Graph, Box<dyn std::error::Error>> {
        Ok(Graph {
            nodes: vec![
                artifact("a", "a.rs")?,
                artifact("b", "b.rs")?,
                artifact("c", "c.rs")?,
                symbol("d", "D", "d.rs")?,
                artifact("e", "e.rs")?,
            ],
            relations: vec![
                edge("ab", "a", "b", RelationKind::Calls),
                edge("bc", "b", "c", RelationKind::Calls),
                edge("cd", "c", "d", RelationKind::Contains),
            ],
        })
    }

    #[test]
    fn focused_neighborhood_respects_radius_and_includes_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest {
            center_node: Some("b".into()),
            radius: 1,
            ..Default::default()
        };
        let result = compute_layout(&graph, &request)?;
        let ids: BTreeSet<String> = result
            .nodes
            .iter()
            .map(|node| node.id.as_str().to_owned())
            .collect();
        assert_eq!(
            ids,
            BTreeSet::from(["a".to_owned(), "b".to_owned(), "c".to_owned()])
        );
        assert_eq!(result.center_node, Some(GraphNodeId::new("b")));
        let center = result
            .nodes
            .iter()
            .find(|n| n.id.as_str() == "b")
            .ok_or("missing center")?;
        assert_eq!(center.hop, 0);
        assert_eq!((center.x, center.y), (0.0, 0.0));
        Ok(())
    }

    #[test]
    fn node_label_filter_excludes_non_matching_but_keeps_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest {
            center_node: Some("c".into()),
            radius: 2,
            node_labels: BTreeSet::from(["Artifact".to_owned()]),
            ..Default::default()
        };
        let result = compute_layout(&graph, &request)?;
        let ids: BTreeSet<String> = result
            .nodes
            .iter()
            .map(|node| node.id.as_str().to_owned())
            .collect();
        // "d" is a Symbol node one hop from "c"; filtered out even though reachable.
        assert!(!ids.contains("d"));
        assert!(ids.contains("c"));
        assert!(ids.contains("a"));
        Ok(())
    }

    #[test]
    fn edge_type_filter_restricts_traversal_and_output() -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest {
            center_node: Some("a".into()),
            radius: 3,
            edge_types: BTreeSet::from([RelationKind::Contains]),
            ..Default::default()
        };
        let result = compute_layout(&graph, &request)?;
        // Only the "cd" edge is `Contains`; "a" can't reach anything through it.
        let ids: BTreeSet<String> = result
            .nodes
            .iter()
            .map(|node| node.id.as_str().to_owned())
            .collect();
        assert_eq!(ids, BTreeSet::from(["a".to_owned()]));
        assert!(result.edges.is_empty());
        Ok(())
    }

    #[test]
    fn overview_positions_every_node_including_disconnected_components()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let result = compute_layout(&graph, &LayoutRequest::default())?;
        assert_eq!(result.nodes.len(), graph.nodes.len());
        assert!(result.nodes.iter().any(|n| n.id.as_str() == "e"));
        assert_eq!(result.center_node, None);
        Ok(())
    }

    #[test]
    fn budget_truncates_deterministically_and_reports_totals()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest {
            max_nodes: 2,
            ..Default::default()
        };
        let first = compute_layout(&graph, &request)?;
        let second = compute_layout(&graph, &request)?;
        assert_eq!(
            first, second,
            "layout must be deterministic across identical requests"
        );
        assert_eq!(first.nodes.len(), 2);
        assert_eq!(first.budget.node_budget, 2);
        assert_eq!(first.budget.nodes_available, graph.nodes.len());
        assert_eq!(first.budget.nodes_returned, 2);
        assert!(first.budget.nodes_truncated);
        Ok(())
    }

    #[test]
    fn edge_budget_is_independent_of_node_budget() -> Result<(), Box<dyn std::error::Error>> {
        // A 4-node clique carries 6 undirected pairs' worth of relations,
        // well inside the node budget but over a tight edge budget.
        let mut graph = Graph::default();
        for id in ["a", "b", "c", "d"] {
            graph.nodes.push(artifact(id, &format!("{id}.rs"))?);
        }
        let pairs = [
            ("a", "b"),
            ("a", "c"),
            ("a", "d"),
            ("b", "c"),
            ("b", "d"),
            ("c", "d"),
        ];
        for (index, (source, target)) in pairs.iter().enumerate() {
            graph.relations.push(edge(
                &format!("e{index}"),
                source,
                target,
                RelationKind::Calls,
            ));
        }
        let request = LayoutRequest {
            max_edges: 2,
            ..Default::default()
        };
        let result = compute_layout(&graph, &request)?;
        assert!(!result.budget.nodes_truncated);
        assert_eq!(result.budget.edges_available, 6);
        assert_eq!(result.budget.edge_budget, 2);
        assert_eq!(result.edges.len(), 2);
        assert!(result.budget.edges_truncated);
        Ok(())
    }

    #[test]
    fn unknown_center_node_query_errors() -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest {
            center_node: Some("does-not-exist".into()),
            ..Default::default()
        };
        assert!(compute_layout(&graph, &request).is_err());
        Ok(())
    }

    #[test]
    fn empty_graph_overview_returns_empty_result_without_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let result = compute_layout(&Graph::default(), &LayoutRequest::default())?;
        assert!(result.nodes.is_empty());
        assert!(result.edges.is_empty());
        assert!(!result.budget.nodes_truncated);
        Ok(())
    }

    #[test]
    fn large_graph_respects_node_and_edge_limits() -> Result<(), Box<dyn std::error::Error>> {
        let mut graph = Graph::default();
        for index in 0..500 {
            graph
                .nodes
                .push(artifact(&format!("n{index}"), &format!("n{index}.rs"))?);
        }
        for index in 0..499 {
            graph.relations.push(edge(
                &format!("e{index}"),
                &format!("n{index}"),
                &format!("n{}", index + 1),
                RelationKind::Calls,
            ));
        }
        let result = compute_layout(&graph, &LayoutRequest::default())?;
        assert_eq!(result.budget.nodes_available, 500);
        assert_eq!(result.budget.node_budget, DEFAULT_NODE_BUDGET);
        assert_eq!(result.nodes.len(), DEFAULT_NODE_BUDGET);
        assert!(result.budget.nodes_truncated);
        Ok(())
    }

    #[test]
    fn compute_layout_cached_serves_a_seeded_result_without_recomputing()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest::default();
        let temp = tempfile::TempDir::new()?;
        let store = LayoutSnapshotStore::new(temp.path());
        let snapshot_id = graph_snapshot_id(&graph)?;

        let mut seeded = compute_layout(&graph, &request)?;
        // A marker value a fresh computation would never produce, proving
        // a served result came from the cache rather than recomputation.
        seeded.algorithm_version = 999;
        store.save(&LayoutSnapshot {
            graph_snapshot_id: snapshot_id,
            algorithm_version: LAYOUT_ALGORITHM_VERSION,
            request: request.clone(),
            result: seeded,
        })?;

        let served = compute_layout_cached(&graph, &request, &store)?;
        assert_eq!(served.algorithm_version, 999);
        Ok(())
    }

    #[test]
    fn layout_snapshot_store_round_trips_and_skips_identical_writes()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = chain_graph()?;
        let request = LayoutRequest::default();
        let temp = tempfile::TempDir::new()?;
        let store = LayoutSnapshotStore::new(temp.path());
        let snapshot = LayoutSnapshot {
            graph_snapshot_id: graph_snapshot_id(&graph)?,
            algorithm_version: LAYOUT_ALGORITHM_VERSION,
            request: request.clone(),
            result: compute_layout(&graph, &request)?,
        };
        assert!(store.save(&snapshot)?);
        assert!(
            !store.save(&snapshot)?,
            "identical content must skip the write"
        );
        assert_eq!(
            store.load(
                &snapshot.graph_snapshot_id,
                snapshot.algorithm_version,
                &snapshot.request
            )?,
            Some(snapshot)
        );
        Ok(())
    }
}
