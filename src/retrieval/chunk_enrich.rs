//! Graph enrichment for source chunks (LIT-86.4).
//!
//! Attaches canonical graph context to a raw chunk: its artifact evidence,
//! the symbols whose spans overlap it, the owning module/package/service, an
//! injected architecture layer, deterministic tags, and a bounded, ordered set
//! of typed graph neighbors. Every reference is derived from the graph -- never
//! guessed from the chunk's embedding text -- so a semantic hit can always be
//! traced back to real nodes and repository bytes.
//!
//! Enrichment is a pure function of `(graph, chunk identity)`; it holds no
//! embedding. A graph-only change re-derives enrichment while the
//! content-hash-keyed vector (LIT-86.3) is reused unchanged, and a chunk edit
//! changes only that chunk's identity. This separation is what lets 86.3 reuse
//! vectors across graph-only updates.

// ponytail: consumed by the index build (LIT-86.3) and graph-constrained
// ranking (LIT-86.5). Drop this allow at first production wiring.
#![allow(dead_code)]

use crate::analysis::chunks::SourceChunk;
use crate::domain::{ArtifactId, EvidenceRef, RepoPath, SourceSpan};
use crate::graph::{ConfigNodeKind, Graph, GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Direction of a typed graph neighbor relative to the enriched chunk's nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum Direction {
    /// The chunk's node is the relation source.
    Outgoing,
    /// The chunk's node is the relation target.
    Incoming,
}

/// One bounded, typed graph neighbor of a chunk (AC#4).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct GraphNeighbor {
    /// Relation category.
    pub relation: RelationKind,
    /// Whether the chunk's node was the source or target.
    pub direction: Direction,
    /// The node on the other end of the relation.
    pub node: GraphNodeId,
}

/// A chunk plus its canonical graph context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnrichedChunk {
    /// Stable chunk identity (LIT-86.1), copied from the source chunk.
    pub chunk_id: String,
    /// Owning artifact identity.
    pub artifact_id: ArtifactId,
    /// Evidence span back to the exact repository bytes (AC#1).
    pub evidence: EvidenceRef,
    /// Symbols whose evidence span overlaps this chunk, sorted and unique.
    pub symbol_ids: Vec<GraphNodeId>,
    /// Owning module node, when the artifact belongs to one.
    pub module_id: Option<GraphNodeId>,
    /// Owning package node, when the artifact or module belongs to one.
    pub package_id: Option<GraphNodeId>,
    /// Owning service/job name, when a service boundary contains the artifact.
    pub service: Option<String>,
    /// Architecture layer, injected from the module planner / architecture
    /// index (never inferred here).
    pub layer: Option<String>,
    /// Deterministic, graph-derived tags.
    pub tags: Vec<String>,
    /// Bounded, deterministically ordered typed neighbors.
    pub neighbors: Vec<GraphNeighbor>,
}

/// Enriches `chunk` (belonging to `artifact_path`) with context from `graph`.
/// `layers` maps a module node id to its architecture layer name (empty when
/// unavailable); `max_neighbors` bounds the neighbor set.
pub(crate) fn enrich_chunk(
    graph: &Graph,
    artifact_path: &RepoPath,
    chunk: &SourceChunk,
    layers: &BTreeMap<GraphNodeId, String>,
    max_neighbors: usize,
) -> EnrichedChunk {
    let artifact_id = ArtifactId::from_path(artifact_path);
    let artifact_node_id = GraphNodeId::new(format!("artifact:{}", artifact_path.as_str()));

    // Evidence spans the chunk's line range; a chunk always has a valid
    // one-based inclusive range, so fall back to file-level evidence only if
    // the (never-expected) span construction fails.
    let evidence = match SourceSpan::new(chunk.start.line, chunk.end.line) {
        Ok(span) => EvidenceRef::file(artifact_id.clone(), artifact_path.clone()).with_span(span),
        Err(_) => EvidenceRef::file(artifact_id.clone(), artifact_path.clone()),
    };

    let symbol_ids = overlapping_symbol_ids(graph, artifact_path.as_str(), chunk);
    let module_id = related_target(graph, &artifact_node_id, RelationKind::BelongsToModule);
    // A package can be owned by the artifact directly or via its module.
    let package_id = related_target(graph, &artifact_node_id, RelationKind::BelongsToPackage)
        .or_else(|| {
            module_id
                .as_ref()
                .and_then(|module| related_target(graph, module, RelationKind::BelongsToPackage))
        });
    let service = owning_service(graph, &artifact_node_id, &symbol_ids);
    let layer = module_id
        .as_ref()
        .and_then(|module| layers.get(module).cloned());
    let tags = derive_tags(
        graph,
        &artifact_node_id,
        package_id.as_ref(),
        service.as_ref(),
    );
    let neighbors = bounded_neighbors(graph, &artifact_node_id, &symbol_ids, max_neighbors);

    EnrichedChunk {
        chunk_id: chunk.id.clone(),
        artifact_id,
        evidence,
        symbol_ids,
        module_id,
        package_id,
        service,
        layer,
        tags,
        neighbors,
    }
}

/// Symbol node ids whose evidence lies in `path` and whose line span overlaps
/// the chunk (AC#2). Interval overlap is inclusive on both ends, so a symbol
/// straddling a chunk boundary attaches to both chunks it touches; a symbol
/// with no span is skipped (it cannot be interval-matched).
fn overlapping_symbol_ids(graph: &Graph, path: &str, chunk: &SourceChunk) -> Vec<GraphNodeId> {
    let (chunk_start, chunk_end) = (chunk.start.line, chunk.end.line);
    let mut ids: Vec<GraphNodeId> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Symbol(symbol) if symbol.evidence.path.as_str() == path => {
                let span = symbol.evidence.span.as_ref()?;
                (span.start_line <= chunk_end && span.end_line >= chunk_start)
                    .then(|| symbol.id.clone())
            }
            _ => None,
        })
        .collect();
    ids.sort();
    ids.dedup();
    ids
}

/// The target of the first relation of `kind` whose source is `source`, if any.
/// Relations are sorted deterministically in the graph, so "first" is stable.
fn related_target(graph: &Graph, source: &GraphNodeId, kind: RelationKind) -> Option<GraphNodeId> {
    graph
        .relations
        .iter()
        .find(|relation| relation.kind == kind && &relation.source == source)
        .map(|relation| relation.target.clone())
}

/// Name of the service/job whose `Contains` relation reaches the artifact or
/// one of its symbols (AC#3 service boundary), if any.
fn owning_service(
    graph: &Graph,
    artifact_node_id: &GraphNodeId,
    symbol_ids: &[GraphNodeId],
) -> Option<String> {
    graph.relations.iter().find_map(|relation| {
        if relation.kind != RelationKind::Contains {
            return None;
        }
        let contains_ours =
            &relation.target == artifact_node_id || symbol_ids.contains(&relation.target);
        if !contains_ours {
            return None;
        }
        graph.nodes.iter().find_map(|node| match node {
            GraphNode::Config(config)
                if config.id == relation.source
                    && matches!(config.kind, ConfigNodeKind::Service | ConfigNodeKind::Job) =>
            {
                Some(config.name.clone())
            }
            _ => None,
        })
    })
}

/// Deterministic, graph-derived tags. Bounded and sorted; derived only from
/// canonical facts (package externality, service membership, node kind), never
/// from chunk text.
fn derive_tags(
    graph: &Graph,
    artifact_node_id: &GraphNodeId,
    package_id: Option<&GraphNodeId>,
    service: Option<&String>,
) -> Vec<String> {
    let mut tags = Vec::new();
    if service.is_some() {
        tags.push("in-service".to_owned());
    }
    if let Some(package_id) = package_id
        && let Some(GraphNode::Package(package)) =
            graph.nodes.iter().find(|node| node.id() == package_id)
    {
        tags.push(if package.is_external {
            "external-package".to_owned()
        } else {
            "local-package".to_owned()
        });
    }
    // Whether the artifact participates in any cross-service-boundary relation.
    if graph.relations.iter().any(|relation| {
        relation.kind == RelationKind::CrossesServiceBoundary
            && (&relation.source == artifact_node_id || &relation.target == artifact_node_id)
    }) {
        tags.push("crosses-service-boundary".to_owned());
    }
    tags.sort();
    tags.dedup();
    tags
}

/// Bounded, deterministically ordered typed neighbors of the chunk's nodes
/// (the artifact and its symbols). Records relation kind and direction; sorted
/// and truncated so the set is stable and size-bounded (AC#4).
fn bounded_neighbors(
    graph: &Graph,
    artifact_node_id: &GraphNodeId,
    symbol_ids: &[GraphNodeId],
    max_neighbors: usize,
) -> Vec<GraphNeighbor> {
    let is_ours = |id: &GraphNodeId| id == artifact_node_id || symbol_ids.contains(id);
    let mut neighbors: Vec<GraphNeighbor> = graph
        .relations
        .iter()
        .filter_map(|relation| {
            if is_ours(&relation.source) && !is_ours(&relation.target) {
                Some(GraphNeighbor {
                    relation: relation.kind,
                    direction: Direction::Outgoing,
                    node: relation.target.clone(),
                })
            } else if is_ours(&relation.target) && !is_ours(&relation.source) {
                Some(GraphNeighbor {
                    relation: relation.kind,
                    direction: Direction::Incoming,
                    node: relation.source.clone(),
                })
            } else {
                None
            }
        })
        .collect();
    neighbors.sort();
    neighbors.dedup();
    neighbors.truncate(max_neighbors);
    neighbors
}

#[cfg(test)]
mod tests {
    use super::{Direction, enrich_chunk};
    use crate::analysis::chunks::{ChunkConfig, ChunkParse, SourceChunk, chunk_source};
    use crate::domain::{Confidence, EvidenceRef, RepoPath, SourceSpan};
    use crate::graph::{
        ConfigNode, ConfigNodeKind, Graph, GraphNode, GraphNodeId, PackageNode, Relation,
        RelationKind, SymbolKind, SymbolNode,
    };
    use std::collections::BTreeMap;

    fn symbol(
        path: &str,
        name: &str,
        start: u32,
        end: u32,
    ) -> Result<GraphNode, Box<dyn std::error::Error>> {
        let repo = RepoPath::new(path)?;
        let artifact_id = crate::domain::ArtifactId::from_path(&repo);
        Ok(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(format!("symbol:{path}#{name}")),
            kind: SymbolKind::Function,
            qualified_name: name.to_owned(),
            doc: None,
            evidence: EvidenceRef::file(artifact_id, repo).with_span(SourceSpan::new(start, end)?),
        }))
    }

    fn relation(id: &str, source: &str, target: &str, kind: RelationKind) -> Relation {
        Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        }
    }

    /// One chunk covering lines 1..=6 of a file with two functions (lines 1-3
    /// and 4-6) attaches both symbols (AC#8 multiple symbols per chunk).
    fn one_chunk(path: &str, text: &str) -> SourceChunk {
        let mut chunks = chunk_source(
            path,
            text,
            &[],
            &ChunkParse::Syntax,
            &ChunkConfig {
                target_bytes: 10_000,
                min_bytes: 1,
                max_bytes: 10_000,
                overlap_bytes: 0,
            },
        );
        chunks.remove(0)
    }

    #[test]
    fn attaches_multiple_and_nested_symbols_by_interval() -> Result<(), Box<dyn std::error::Error>>
    {
        let path = "m.rs";
        let text = "fn a() {\n  helper();\n}\nfn b() {\n  other();\n}\n";
        let chunk = one_chunk(path, text);
        // outer a: lines 1-3, nested helper call symbol 2-2, b: 4-6.
        let graph = Graph {
            nodes: vec![
                symbol(path, "a", 1, 3)?,
                symbol(path, "helper", 2, 2)?,
                symbol(path, "b", 4, 6)?,
                // A symbol in another file must NOT attach.
                symbol("other.rs", "z", 1, 2)?,
            ],
            relations: vec![],
        };
        let enriched = enrich_chunk(&graph, &RepoPath::new(path)?, &chunk, &BTreeMap::new(), 16);
        assert_eq!(enriched.symbol_ids.len(), 3);
        assert!(
            enriched
                .symbol_ids
                .iter()
                .all(|id| id.as_str().contains("m.rs"))
        );
        // Evidence round-trips to the chunk's line span (AC#1).
        assert_eq!(enriched.evidence.span, Some(SourceSpan::new(1, 6)?));
        Ok(())
    }

    #[test]
    fn symbol_without_span_is_skipped() -> Result<(), Box<dyn std::error::Error>> {
        let path = "m.rs";
        let chunk = one_chunk(path, "fn a() {}\n");
        let repo = RepoPath::new(path)?;
        let artifact_id = crate::domain::ArtifactId::from_path(&repo);
        let graph = Graph {
            nodes: vec![GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new("symbol:m.rs#nospan"),
                kind: SymbolKind::Function,
                qualified_name: "nospan".to_owned(),
                doc: None,
                evidence: EvidenceRef::file(artifact_id, repo.clone()),
            })],
            relations: vec![],
        };
        let enriched = enrich_chunk(&graph, &repo, &chunk, &BTreeMap::new(), 16);
        assert!(enriched.symbol_ids.is_empty());
        Ok(())
    }

    #[test]
    fn derives_module_package_service_and_layer_from_graph()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = "svc/app.rs";
        let chunk = one_chunk(path, "fn handler() {}\n");
        let artifact = "artifact:svc/app.rs";
        let module = "module:svc";
        let package = "package:app";
        let graph = Graph {
            nodes: vec![
                symbol(path, "handler", 1, 1)?,
                GraphNode::Package(PackageNode {
                    id: GraphNodeId::new(package),
                    name: "app".to_owned(),
                    is_external: false,
                }),
                GraphNode::Config(ConfigNode {
                    id: GraphNodeId::new("config:web"),
                    kind: ConfigNodeKind::Service,
                    name: "web".to_owned(),
                    evidence: EvidenceRef::file(
                        crate::domain::ArtifactId::from_path(&RepoPath::new("docker-compose.yml")?),
                        RepoPath::new("docker-compose.yml")?,
                    ),
                }),
            ],
            relations: vec![
                relation("r1", artifact, module, RelationKind::BelongsToModule),
                relation("r2", module, package, RelationKind::BelongsToPackage),
                relation("r3", "config:web", artifact, RelationKind::Contains),
            ],
        };
        let mut layers = BTreeMap::new();
        layers.insert(GraphNodeId::new(module), "application".to_owned());

        let enriched = enrich_chunk(&graph, &RepoPath::new(path)?, &chunk, &layers, 16);
        assert_eq!(enriched.module_id, Some(GraphNodeId::new(module)));
        assert_eq!(enriched.package_id, Some(GraphNodeId::new(package)));
        assert_eq!(enriched.service.as_deref(), Some("web"));
        assert_eq!(enriched.layer.as_deref(), Some("application"));
        assert!(enriched.tags.contains(&"in-service".to_owned()));
        assert!(enriched.tags.contains(&"local-package".to_owned()));
        Ok(())
    }

    #[test]
    fn module_regrouping_updates_enrichment_without_touching_chunk_text()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = "m.rs";
        let chunk = one_chunk(path, "fn a() {}\n");
        let artifact = "artifact:m.rs";
        let repo = RepoPath::new(path)?;

        let before = Graph {
            nodes: vec![symbol(path, "a", 1, 1)?],
            relations: vec![relation(
                "r",
                artifact,
                "module:old",
                RelationKind::BelongsToModule,
            )],
        };
        let after = Graph {
            nodes: vec![symbol(path, "a", 1, 1)?],
            relations: vec![relation(
                "r",
                artifact,
                "module:new",
                RelationKind::BelongsToModule,
            )],
        };

        let e1 = enrich_chunk(&before, &repo, &chunk, &BTreeMap::new(), 16);
        let e2 = enrich_chunk(&after, &repo, &chunk, &BTreeMap::new(), 16);
        assert_eq!(e1.module_id, Some(GraphNodeId::new("module:old")));
        assert_eq!(e2.module_id, Some(GraphNodeId::new("module:new")));
        // Chunk identity (what drives embedding reuse) is unchanged (AC#5).
        assert_eq!(e1.chunk_id, e2.chunk_id);
        Ok(())
    }

    #[test]
    fn neighbors_are_bounded_ordered_and_directional() -> Result<(), Box<dyn std::error::Error>> {
        let path = "m.rs";
        let chunk = one_chunk(path, "fn a() {}\n");
        let artifact = "artifact:m.rs";
        let symbol_a = "symbol:m.rs#a";
        let graph = Graph {
            nodes: vec![symbol(path, "a", 1, 1)?],
            relations: vec![
                relation("r1", symbol_a, "unresolved:x", RelationKind::Calls),
                relation("r2", symbol_a, "symbol:other#y", RelationKind::Imports),
                relation("r3", "config:web", artifact, RelationKind::Contains),
            ],
        };
        let enriched = enrich_chunk(&graph, &RepoPath::new(path)?, &chunk, &BTreeMap::new(), 2);
        assert_eq!(enriched.neighbors.len(), 2, "bounded to max_neighbors");
        // Sorted: a deterministic, stable order.
        let mut sorted = enriched.neighbors.clone();
        sorted.sort();
        assert_eq!(enriched.neighbors, sorted);
        // The Contains from config:web is an incoming edge on the artifact.
        let incoming = enrich_chunk(&graph, &RepoPath::new(path)?, &chunk, &BTreeMap::new(), 16);
        assert!(incoming.neighbors.iter().any(|n| {
            n.direction == Direction::Incoming && n.node == GraphNodeId::new("config:web")
        }));
        // An unresolved target is a legitimate neighbor (AC#8 unresolved).
        assert!(
            incoming
                .neighbors
                .iter()
                .any(|n| n.node.as_str().starts_with("unresolved:"))
        );
        Ok(())
    }
}
