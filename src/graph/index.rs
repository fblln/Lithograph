//! Queryable knowledge index over the typed semantic graph.

use crate::graph::{ConfigNodeKind, Graph, GraphNode, GraphNodeId, Relation, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Deterministic graph schema summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSchema {
    /// Counts by graph node label.
    pub node_labels: Vec<LabelCount>,
    /// Counts by relation type.
    pub edge_types: Vec<TypeCount>,
    /// Observed source/edge/target patterns.
    pub relationship_patterns: Vec<String>,
}

/// Count for one node label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelCount {
    /// Node label.
    pub label: String,
    /// Number of nodes with this label.
    pub count: usize,
}

/// Count for one edge type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeCount {
    /// Relation type.
    pub edge_type: String,
    /// Number of relations with this type.
    pub count: usize,
}

/// Structured graph search parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SearchParams {
    /// Optional node label filter, e.g. `Symbol`, `Artifact`, or `Package`.
    pub label: Option<String>,
    /// Optional case-insensitive substring matched against node names, ids, and paths.
    pub query: Option<String>,
    /// Maximum result count. Defaults to 10 when zero.
    pub limit: usize,
}

/// One graph search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Graph node id.
    pub id: GraphNodeId,
    /// Node label.
    pub label: String,
    /// Human-readable name.
    pub name: String,
    /// Repository-relative file path when the node has one.
    pub file_path: Option<String>,
    /// Inbound relation count.
    pub in_degree: usize,
    /// Outbound relation count.
    pub out_degree: usize,
}

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

/// Architecture-oriented graph summary inspired by codebase-memory-style queries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureSummary {
    /// Graph schema counts.
    pub schema: GraphSchema,
    /// Local and external package nodes.
    pub packages: Vec<PackageSummary>,
    /// Entry points inferred from commands, containers, and high-degree source symbols.
    pub entry_points: Vec<SearchResult>,
    /// High-degree graph nodes.
    pub hotspots: Vec<SearchResult>,
    /// External packages, env vars, and unresolved references.
    pub boundaries: Vec<SearchResult>,
    /// Existing architecture or decision documentation nodes.
    pub architecture_docs: Vec<SearchResult>,
    /// HTTP routes, gRPC/protobuf RPCs, GraphQL fields, and Compose
    /// services (LIT-22.3.4 AC3): every `Config` node whose kind is
    /// `Route` or `Service`.
    pub service_links: Vec<SearchResult>,
}

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

/// Package summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageSummary {
    /// Package name.
    pub name: String,
    /// True when external to the repository.
    pub is_external: bool,
    /// Inbound relation count.
    pub in_degree: usize,
    /// Outbound relation count.
    pub out_degree: usize,
}

/// Queryable knowledge index over one graph snapshot.
#[derive(Debug, Clone, Copy)]
pub struct KnowledgeIndex<'a> {
    graph: &'a Graph,
}

impl<'a> KnowledgeIndex<'a> {
    /// Creates an index over a graph snapshot.
    pub fn new(graph: &'a Graph) -> Self {
        Self { graph }
    }

    /// Returns deterministic graph schema counts.
    pub fn schema(&self) -> GraphSchema {
        let node_by_id = self.node_by_id();
        let mut node_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut edge_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut patterns: BTreeMap<String, usize> = BTreeMap::new();

        for node in &self.graph.nodes {
            *node_counts.entry(node_label(node).to_owned()).or_default() += 1;
        }
        for relation in &self.graph.relations {
            let edge = format!("{:?}", relation.kind);
            *edge_counts.entry(edge.clone()).or_default() += 1;
            let source = node_by_id
                .get(&relation.source)
                .map_or("Unknown", |node| node_label(node));
            let target = node_by_id
                .get(&relation.target)
                .map_or("Unknown", |node| node_label(node));
            *patterns
                .entry(format!("({source})-[{edge}]->({target})"))
                .or_default() += 1;
        }

        GraphSchema {
            node_labels: node_counts
                .into_iter()
                .map(|(label, count)| LabelCount { label, count })
                .collect(),
            edge_types: edge_counts
                .into_iter()
                .map(|(edge_type, count)| TypeCount { edge_type, count })
                .collect(),
            relationship_patterns: patterns
                .into_iter()
                .map(|(pattern, count)| format!("{pattern} [{count}x]"))
                .collect(),
        }
    }

    /// Searches nodes by label and substring query.
    pub fn search(&self, params: &SearchParams) -> Vec<SearchResult> {
        let degree = self.degree_index();
        let query = params.query.as_ref().map(|query| query.to_lowercase());
        let label = params.label.as_ref().map(|label| label.to_lowercase());
        let limit = default_limit(params.limit);

        let mut results: Vec<SearchResult> = self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                label
                    .as_ref()
                    .is_none_or(|wanted| node_label(node).to_lowercase() == *wanted)
            })
            .filter(|node| {
                query
                    .as_ref()
                    .is_none_or(|wanted| node_search_text(node).contains(wanted))
            })
            .map(|node| search_result(node, &degree))
            .collect();
        results.sort_by(|a, b| {
            (b.in_degree + b.out_degree)
                .cmp(&(a.in_degree + a.out_degree))
                .then(a.label.cmp(&b.label))
                .then(a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        results.truncate(limit);
        results
    }

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

    /// Symbol nodes with no inbound relation anywhere in the graph -- never
    /// called, implemented, referenced, or used. A heuristic (it can't see
    /// true entry points like a `main` function or reflection-based
    /// dynamic dispatch), not a certainty; callers should treat the result
    /// as candidates to review, not a definite deletion list.
    pub fn find_dead_code(&self) -> Vec<SearchResult> {
        // `Contains` (an artifact/class defining this symbol) is structural,
        // not a use -- every symbol has exactly one, so counting it would
        // make every symbol look "referenced" and this method useless.
        let mut referenced: BTreeSet<&GraphNodeId> = BTreeSet::new();
        for relation in &self.graph.relations {
            if relation.kind != RelationKind::Contains {
                referenced.insert(&relation.target);
            }
        }
        let degree = self.degree_index();
        let mut dead: Vec<SearchResult> = self
            .graph
            .nodes
            .iter()
            .filter(|node| matches!(node, GraphNode::Symbol(_)) && !referenced.contains(node.id()))
            .map(|node| search_result(node, &degree))
            .collect();
        dead.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        dead
    }

    /// Groups the graph into functional architecture communities: connected
    /// components over [`CLUSTER_EDGE_KINDS`] edges (LIT-22.5.1). Only
    /// components with 2+ members are reported -- an isolated node touched
    /// by no qualifying edge isn't a "community." Deterministic: cluster
    /// ids and member order derive only from node/relation ids already
    /// sorted by [`Graph::to_json`]'s invariants, never from map iteration
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

    /// Returns a compact architecture summary over the graph.
    pub fn architecture(&self) -> ArchitectureSummary {
        let degree = self.degree_index();
        let mut packages = Vec::new();
        let mut entry_points = Vec::new();
        let mut boundaries = Vec::new();
        let mut architecture_docs = Vec::new();
        let mut service_links = Vec::new();
        let mut all_results = Vec::new();

        for node in &self.graph.nodes {
            let result = search_result(node, &degree);
            match node {
                GraphNode::Package(package) => {
                    packages.push(PackageSummary {
                        name: package.name.clone(),
                        is_external: package.is_external,
                        in_degree: result.in_degree,
                        out_degree: result.out_degree,
                    });
                    if package.is_external {
                        boundaries.push(result.clone());
                    }
                }
                GraphNode::Config(config)
                    if matches!(config.kind, ConfigNodeKind::Route | ConfigNodeKind::Service) =>
                {
                    service_links.push(result.clone());
                }
                GraphNode::Command(_) | GraphNode::Container(_) => {
                    entry_points.push(result.clone())
                }
                GraphNode::EnvVar(_) | GraphNode::Unresolved(_) => boundaries.push(result.clone()),
                GraphNode::Documentation(doc)
                    if doc.title.to_lowercase().contains("architecture")
                        || doc.title.to_lowercase().contains("decision") =>
                {
                    architecture_docs.push(result.clone());
                }
                GraphNode::Artifact(artifact)
                    if artifact.path.to_lowercase().contains("architecture")
                        || artifact.path.to_lowercase().contains("adr") =>
                {
                    architecture_docs.push(result.clone());
                }
                _ => {}
            }
            all_results.push(result);
        }

        all_results.sort_by(|a, b| {
            (b.in_degree + b.out_degree)
                .cmp(&(a.in_degree + a.out_degree))
                .then(a.name.cmp(&b.name))
        });
        let mut hotspots = all_results;
        hotspots.truncate(10);
        entry_points.sort_by(|a, b| a.name.cmp(&b.name));
        entry_points.truncate(20);
        boundaries.sort_by(|a, b| a.label.cmp(&b.label).then(a.name.cmp(&b.name)));
        boundaries.truncate(30);
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        architecture_docs.sort_by(|a, b| a.name.cmp(&b.name));
        service_links.sort_by(|a, b| a.name.cmp(&b.name));

        ArchitectureSummary {
            schema: self.schema(),
            packages,
            entry_points,
            hotspots,
            boundaries,
            architecture_docs,
            service_links,
        }
    }

    /// Typed package-map lookup for import resolvers (LIT-22.2.4 AC3):
    /// returns every package `package_name` declares a `DependsOnPackage`
    /// edge to, local or external. `package_name` matches a `Package` node's
    /// name exactly (e.g. a registry id from a manifest analyzer), not a
    /// substring.
    pub fn package_dependencies(&self, package_name: &str) -> Vec<PackageSummary> {
        let degree = self.degree_index();
        let node_by_id = self.node_by_id();
        let Some(source_id) = self.graph.nodes.iter().find_map(|node| match node {
            GraphNode::Package(package) if package.name == package_name => Some(node.id()),
            _ => None,
        }) else {
            return Vec::new();
        };

        let mut dependencies: Vec<PackageSummary> = self
            .graph
            .relations
            .iter()
            .filter(|relation| {
                relation.kind == RelationKind::DependsOnPackage && &relation.source == source_id
            })
            .filter_map(|relation| {
                let node = node_by_id.get(&relation.target)?;
                let GraphNode::Package(package) = node else {
                    return None;
                };
                let (in_degree, out_degree) = degree.get(node.id()).copied().unwrap_or((0, 0));
                Some(PackageSummary {
                    name: package.name.clone(),
                    is_external: package.is_external,
                    in_degree,
                    out_degree,
                })
            })
            .collect();
        dependencies.sort_by(|a, b| a.name.cmp(&b.name));
        dependencies.dedup_by(|a, b| a.name == b.name);
        dependencies
    }

    fn find_root(&self, query: &str) -> Option<&GraphNode> {
        let query_lower = query.to_lowercase();
        self.graph
            .nodes
            .iter()
            .find(|node| node.id().as_str() == query)
            .or_else(|| {
                self.graph
                    .nodes
                    .iter()
                    .find(|node| node_name(node) == query)
            })
            .or_else(|| {
                self.graph
                    .nodes
                    .iter()
                    .find(|node| node_search_text(node).contains(&query_lower))
            })
    }

    fn node_by_id(&self) -> BTreeMap<&GraphNodeId, &GraphNode> {
        self.graph
            .nodes
            .iter()
            .map(|node| (node.id(), node))
            .collect()
    }

    fn degree_index(&self) -> BTreeMap<&GraphNodeId, (usize, usize)> {
        let mut degree = BTreeMap::new();
        for node in &self.graph.nodes {
            degree.insert(node.id(), (0usize, 0usize));
        }
        for relation in &self.graph.relations {
            if let Some((_, out_degree)) = degree.get_mut(&relation.source) {
                *out_degree += 1;
            }
            if let Some((in_degree, _)) = degree.get_mut(&relation.target) {
                *in_degree += 1;
            }
        }
        degree
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

fn default_limit(limit: usize) -> usize {
    if limit == 0 { 10 } else { limit.min(100) }
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

fn search_result(
    node: &GraphNode,
    degree: &BTreeMap<&GraphNodeId, (usize, usize)>,
) -> SearchResult {
    let (in_degree, out_degree) = degree.get(node.id()).copied().unwrap_or((0, 0));
    SearchResult {
        id: node.id().clone(),
        label: node_label(node).to_owned(),
        name: node_name(node),
        file_path: node_file_path(node),
        in_degree,
        out_degree,
    }
}

fn node_label(node: &GraphNode) -> &'static str {
    match node {
        GraphNode::Artifact(_) => "Artifact",
        GraphNode::Symbol(_) => "Symbol",
        GraphNode::Config(_) => "Config",
        GraphNode::Documentation(_) => "Documentation",
        GraphNode::Container(_) => "Container",
        GraphNode::Command(_) => "Command",
        GraphNode::EnvVar(_) => "EnvVar",
        GraphNode::Module(_) => "Module",
        GraphNode::Package(_) => "Package",
        GraphNode::Unresolved(_) => "Unresolved",
    }
}

fn node_name(node: &GraphNode) -> String {
    match node {
        GraphNode::Artifact(node) => node.path.clone(),
        GraphNode::Symbol(node) => node.qualified_name.clone(),
        GraphNode::Config(node) => node.name.clone(),
        GraphNode::Documentation(node) => node.title.clone(),
        GraphNode::Container(node) => node.reference.clone(),
        GraphNode::Command(node) => node.text.clone(),
        GraphNode::EnvVar(node) => node.name.clone(),
        GraphNode::Module(node) => node.path.clone(),
        GraphNode::Package(node) => node.name.clone(),
        GraphNode::Unresolved(node) => node.value.clone(),
    }
}

fn node_file_path(node: &GraphNode) -> Option<String> {
    match node {
        GraphNode::Artifact(node) => Some(node.path.clone()),
        GraphNode::Symbol(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Config(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Documentation(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Command(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Module(node) => Some(node.evidence.path.as_str().to_owned()),
        _ => None,
    }
}

fn node_search_text(node: &GraphNode) -> String {
    let mut text = format!("{} {} {}", node.id(), node_label(node), node_name(node));
    if let Some(path) = node_file_path(node) {
        text.push(' ');
        text.push_str(&path);
    }
    text.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::{ArchitectureCluster, KnowledgeIndex, SearchParams, TraceDirection, TraceParams};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_graph() -> Result<crate::graph::Graph, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        Ok(GraphBuilder.build(&root, &artifacts))
    }

    #[test]
    fn schema_search_trace_and_architecture_are_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let index = KnowledgeIndex::new(&graph);

        let schema = index.schema();
        assert!(
            schema
                .node_labels
                .iter()
                .any(|label| label.label == "Artifact")
        );
        assert!(
            schema
                .edge_types
                .iter()
                .any(|edge| edge.edge_type == "Contains")
        );

        let search = index.search(&SearchParams {
            label: Some("Artifact".to_owned()),
            query: Some("python".to_owned()),
            limit: 5,
        });
        assert!(!search.is_empty());

        let trace = index
            .trace(&TraceParams {
                query: search[0].id.as_str().to_owned(),
                depth: 1,
                direction: TraceDirection::Both,
            })
            .ok_or("missing trace result")?;
        assert_eq!(trace.root.id, search[0].id);
        assert!(!trace.visited.is_empty());

        let architecture = index.architecture();
        assert!(!architecture.hotspots.is_empty());
        assert_eq!(architecture.schema, schema);

        Ok(())
    }

    /// LIT-22.2.4 AC3: `package_dependencies` is the typed API an import
    /// resolver uses to look up a package's declared dependencies.
    #[test]
    fn package_dependencies_looks_up_declared_dependencies_by_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name": "acme-web", "dependencies": {"react": "^18.0.0", "lodash": "^4.0.0"}}"#,
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let dependencies = index.package_dependencies("acme-web");
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies.iter().all(|dependency| dependency.is_external));
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.name == "react")
        );
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.name == "lodash")
        );

        assert!(index.package_dependencies("does-not-exist").is_empty());

        Ok(())
    }

    /// LIT-22.3.4 AC3: HTTP routes and gRPC/GraphQL facts surface in
    /// `architecture().service_links`.
    #[test]
    fn architecture_summary_includes_service_links() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;
        std::fs::write(
            temp.path().join("api.proto"),
            "service Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply) {}\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let architecture = KnowledgeIndex::new(&graph).architecture();

        let names: Vec<&str> = architecture
            .service_links
            .iter()
            .map(|link| link.name.as_str())
            .collect();
        assert!(names.contains(&"GET /users/{id}"));
        assert!(names.contains(&"Greeter.SayHello"));

        Ok(())
    }

    /// LIT-22.4.1 AC1: `find_dead_code` flags an uncalled function and
    /// excludes a called one; `impact_analysis` always traces inbound
    /// regardless of the `direction` passed in, and reports no results for
    /// a query matching no node.
    #[test]
    fn find_dead_code_and_impact_analysis_behave_as_documented()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("app.py"),
            "def used():\n    return 1\n\n\ndef unused():\n    return 2\n\n\ndef caller():\n    return used()\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let dead_names: Vec<String> = index
            .find_dead_code()
            .into_iter()
            .map(|result| result.name)
            .collect();
        assert!(dead_names.iter().any(|name| name.ends_with("::unused")));
        assert!(!dead_names.iter().any(|name| name.ends_with("::used")));

        let used_id = index
            .search(&SearchParams {
                label: Some("Symbol".to_owned()),
                query: Some("app::used".to_owned()),
                limit: 1,
            })
            .into_iter()
            .next()
            .ok_or("missing used() symbol")?
            .id;
        let impact = index
            .impact_analysis(&TraceParams {
                query: used_id.as_str().to_owned(),
                depth: 2,
                direction: TraceDirection::Outbound,
            })
            .ok_or("missing impact result")?;
        // `Calls` relations are attributed to the containing artifact, not
        // the specific calling symbol, so `used()`'s only caller is
        // `app.py` itself, not a `caller` symbol node.
        assert!(
            impact
                .visited
                .iter()
                .any(|hop| hop.node.file_path.as_deref() == Some("app.py"))
        );

        assert!(
            index
                .impact_analysis(&TraceParams {
                    query: "no-such-node".to_owned(),
                    depth: 1,
                    direction: TraceDirection::Both,
                })
                .is_none()
        );

        Ok(())
    }

    /// LIT-22.5.1 AC1/AC4: a small connected graph (a function calling
    /// another in the same file) produces one cluster with real cohesion
    /// and edge-type evidence.
    #[test]
    fn clusters_group_a_small_connected_call_graph() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("app.py"),
            "def used():\n    return 1\n\n\ndef caller():\n    return used()\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let clusters = KnowledgeIndex::new(&graph).clusters();

        assert_eq!(clusters.len(), 1);
        let cluster = &clusters[0];
        assert!(cluster.members.len() >= 2);
        assert!(cluster.cohesion > 0.0);
        assert!(cluster.edge_types.contains(&"Calls".to_owned()));
        assert!(!cluster.top_nodes.is_empty());

        Ok(())
    }

    /// LIT-22.5.1 AC1/AC3/AC4: two unrelated package manifests produce two
    /// disjoint, cross-package clusters (an artifact belonging to its own
    /// local package, which depends on an external one), and clustering
    /// twice over the same unchanged graph is byte-identical.
    #[test]
    fn clusters_separate_disconnected_cross_package_communities()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("a"))?;
        std::fs::create_dir_all(temp.path().join("b"))?;
        std::fs::write(
            temp.path().join("a/package.json"),
            r#"{"name": "pkg-a", "dependencies": {"left-pad": "^1.0.0"}}"#,
        )?;
        std::fs::write(
            temp.path().join("b/package.json"),
            r#"{"name": "pkg-b", "dependencies": {"right-pad": "^1.0.0"}}"#,
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let clusters = index.clusters();
        assert_eq!(clusters.len(), 2);

        let cluster_of = |clusters: &[ArchitectureCluster], package: &str| {
            clusters
                .iter()
                .position(|cluster| cluster.packages.iter().any(|name| name == package))
        };
        let cluster_a = cluster_of(&clusters, "pkg-a").ok_or("missing pkg-a cluster")?;
        let cluster_b = cluster_of(&clusters, "pkg-b").ok_or("missing pkg-b cluster")?;
        assert_ne!(cluster_a, cluster_b);
        assert_eq!(cluster_of(&clusters, "left-pad"), Some(cluster_a));
        assert_eq!(cluster_of(&clusters, "right-pad"), Some(cluster_b));
        for cluster in &clusters {
            assert!(cluster.edge_types.contains(&"DependsOnPackage".to_owned()));
        }

        assert_eq!(clusters, index.clusters());

        Ok(())
    }
}
