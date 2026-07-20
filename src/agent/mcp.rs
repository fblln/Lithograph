//! Minimal deterministic MCP-like JSON-line server over generated wiki data.

use crate::agent::ask::WikiSearch;
use crate::docs::docs_model::{GraphDocument, GraphDocumentSection};
use crate::docs::graph_docs::generate_graph_docs;
use crate::docs::subsystem_docs::{
    SubsystemDocumentStore, generate_subsystem_doc, generate_subsystem_doc_for_nodes,
    list_subsystems,
};
use crate::graph::analytics::{BetweennessPolicy, betweenness, degree_metrics, page_rank};
use crate::graph::index::{node_label, node_name};
use crate::graph::{
    ArchitectureAspect, Graph, GraphNode, GraphNodeId, GraphStore, KnowledgeIndex, LayoutRequest,
    LayoutSnapshotStore, Relation, RelationKind, SearchParams, TagIndex, TraceDirection,
    TraceParams, cluster_display_tags, compute_layout_cached, derive_tags, relation_display_tags,
    resolve_expression, tension_display_tags,
};
use crate::graph::{HealthThresholds, detect_health, score_tensions};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::knowledge::research_feedback::{
    AnswerOutcome, AnswerResultInput, ResearchFeedbackStore, unix_timestamp_now,
};
use crate::plan::ModulePlanner;
use crate::resolve::explain_environment;
use crate::retrieval::search::{CodeSearch, CodeSearchParams};
use crate::run::{RepositorySnapshot, RunMetadata};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::{Component, Path, PathBuf};

/// One MCP tool's stable name and human-readable purpose (LIT-22.8.1
/// AC1/AC2): deterministic and schema-like enough for a caller to
/// discover what's available without guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) struct McpToolInfo {
    /// Stable tool name, passed as `McpRequest.tool`.
    pub name: &'static str,
    /// What the tool does and, where not obvious, its required params.
    pub description: &'static str,
}

/// Every tool this server implements, in a stable order. The single
/// source of truth for both the `list_tools` response and the wiki
/// export's tool listing (AC4), so the two can never silently drift
/// apart as new tools are added.
pub(crate) const MCP_TOOLS: &[McpToolInfo] = &[
    McpToolInfo {
        name: "list_tools",
        description: "Lists every tool this server implements.",
    },
    McpToolInfo {
        name: "read_wiki_structure",
        description: "Lists generated wiki pages (id, path, title).",
    },
    McpToolInfo {
        name: "read_wiki_contents",
        description: "Returns every generated wiki page's full content.",
    },
    McpToolInfo {
        name: "read_research_memory",
        description: "Returns the versioned research AgentMemory index.",
    },
    McpToolInfo {
        name: "save_research_result",
        description: "Records an answer outcome. Params: question, answer, cited_node_ids, outcome, correction?, recorded_at?.",
    },
    McpToolInfo {
        name: "reflect_research",
        description: "Reflects saved outcomes over current graph nodes. Params: now?.",
    },
    McpToolInfo {
        name: "read_research_lessons",
        description: "Returns the versioned reflected research lessons artifact.",
    },
    McpToolInfo {
        name: "list_documentable_subsystems",
        description: "Lists graph-backed subsystem identifiers suitable for documentation.",
    },
    McpToolInfo {
        name: "generate_subsystem_document",
        description: "Generates deterministic graph-backed subsystem documentation. Params: subsystem, instruction?.",
    },
    McpToolInfo {
        name: "get_subsystem_graph_context",
        description: "Returns graph-derived subsystem documentation context. Params: subsystem.",
    },
    McpToolInfo {
        name: "refine_subsystem_document",
        description: "Refines graph-backed subsystem documentation. Params: subsystem, instruction.",
    },
    McpToolInfo {
        name: "validate_subsystem_document",
        description: "Validates cited subsystem-document graph evidence. Params: subsystem.",
    },
    McpToolInfo {
        name: "resolve_tag_expression",
        description: "Resolves a tag expression. Params: expression.",
    },
    McpToolInfo {
        name: "get_tag_facets",
        description: "Returns deterministic tag facet counts.",
    },
    McpToolInfo {
        name: "list_tags",
        description: "Lists all graph tags.",
    },
    McpToolInfo {
        name: "search_tag_prefix",
        description: "Searches tags by prefix. Params: prefix.",
    },
    McpToolInfo {
        name: "get_tagged_subgraph",
        description: "Returns nodes and relations selected by a tag expression. Params: expression.",
    },
    McpToolInfo {
        name: "get_graph_layout",
        description: "Returns a budgeted, positioned graph slice: overview (no center_node) or a focused neighborhood. Params: center_node?, radius?, max_nodes?, max_edges?, node_labels?, node_ids?, edge_types?, hide_unresolved? (bool, default false: exclude Unresolved nodes and their edges).",
    },
    McpToolInfo {
        name: "get_graph_analytics",
        description: "Returns deterministic node metrics and health findings for graph overlays.",
    },
    McpToolInfo {
        name: "get_repository_tensions",
        description: "Returns typed, explainable repository tensions for dashboard hotspots.",
    },
    McpToolInfo {
        name: "get_node_detail",
        description: "Returns typed node evidence, bounded source excerpt, relation provenance, definitions, references, and related docs. Params: node_id.",
    },
    McpToolInfo {
        name: "get_graph_document",
        description: "Returns the current evidence-linked architecture and operations document model plus Markdown.",
    },
    McpToolInfo {
        name: "regenerate_graph_document",
        description: "Deterministically regenerates the current evidence-linked architecture and operations document model plus Markdown.",
    },
    McpToolInfo {
        name: "get_graph_schema",
        description: "Returns the knowledge graph's node/relation label schema.",
    },
    McpToolInfo {
        name: "search_graph",
        description: "Searches graph nodes. Params: label?, query?, limit?.",
    },
    McpToolInfo {
        name: "explain_env",
        description: "Explains environment-variable config links and code users. Params: variable?.",
    },
    McpToolInfo {
        name: "search_code",
        description: "Grep-like code search. Params: query, path_contains?, language?, module_id?, package?, graph_node_id?, limit?.",
    },
    McpToolInfo {
        name: "search_fulltext",
        description: "BM25 full-text search over symbols/docs/paths/facts. Params: query, limit?.",
    },
    McpToolInfo {
        name: "search_semantic",
        description: "Semantic search (deterministic offline embeddings) blended with graph connectivity. Params: query, limit?.",
    },
    McpToolInfo {
        name: "query_graph",
        description: "Narrow Cypher-like MATCH/WHERE/RETURN query. Params: query, e.g. `MATCH (a:Symbol)-[:Calls]->(b:Symbol) WHERE a.name CONTAINS \"foo\" RETURN a, b`.",
    },
    McpToolInfo {
        name: "trace_path",
        description: "Traces relations from a node. Params: query, direction?, depth?.",
    },
    McpToolInfo {
        name: "impact_analysis",
        description: "Traces what depends on a node (blast radius). Params: query, depth?.",
    },
    McpToolInfo {
        name: "find_dead_code",
        description: "Lists graph symbols with no inbound references.",
    },
    McpToolInfo {
        name: "detect_changes",
        description: "Lists artifact paths changed since the last recorded snapshot.",
    },
    McpToolInfo {
        name: "get_run_metrics",
        description: "Returns the last recorded run's metrics: stage timings, graph size, cache hit rate, and estimated prompt tokens.",
    },
    McpToolInfo {
        name: "detect_drift",
        description: "Scans generated docs for drift against current repository facts.",
    },
    McpToolInfo {
        name: "get_architecture",
        description: "Returns the architecture summary. Params: aspects?.",
    },
    McpToolInfo {
        name: "create_adr",
        description: "Creates an ADR. Params: title, context, decision, consequences?.",
    },
    McpToolInfo {
        name: "get_adr",
        description: "Reads one ADR. Params: id.",
    },
    McpToolInfo {
        name: "update_adr",
        description: "Updates an ADR. Params: id, section?+value?, status?.",
    },
    McpToolInfo {
        name: "delete_adr",
        description: "Deletes an ADR. Params: id.",
    },
    McpToolInfo {
        name: "list_adrs",
        description: "Lists every ADR's id, title, and status.",
    },
    McpToolInfo {
        name: "ask_question",
        description: "Answers a question from generated wiki content. Params: question.",
    },
];

/// One request accepted by the local server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct McpRequest {
    /// Client-provided request id.
    pub id: Value,
    /// Tool name: read_wiki_structure, read_wiki_contents, graph tools, or ask_question.
    pub tool: String,
    /// Optional request parameters.
    #[serde(default)]
    pub params: Value,
}

/// One response emitted by the local server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct McpResponse {
    /// Echoed request id.
    pub id: Value,
    /// Response payload on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error text on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Typed, bounded detail payload for one graph node.  The explorer consumes
/// this endpoint instead of deriving evidence from the budgeted layout: a
/// layout is a visual projection and may omit the relations needed to explain
/// a node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NodeDetail {
    id: String,
    label: String,
    name: String,
    evidence: Vec<NodeEvidence>,
    source: SourceExcerpt,
    definitions: Vec<RelatedNode>,
    references: Vec<RelatedRelation>,
    related_docs: Vec<RelatedNode>,
    tags: Vec<crate::graph::GraphTag>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct NodeEvidence {
    path: String,
    start_line: Option<u32>,
    end_line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct SourceExcerpt {
    status: SourceStatus,
    text: Option<String>,
    message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SourceStatus {
    Available,
    Missing,
    Opaque,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RelatedNode {
    id: String,
    label: String,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct RelatedRelation {
    id: String,
    direction: &'static str,
    kind: RelationKind,
    counterpart: RelatedNode,
    evidence: Vec<NodeEvidence>,
    resolver_strategy: Option<String>,
    confidence: crate::domain::Confidence,
    tags: Vec<crate::graph::GraphTag>,
}

/// Deterministic wiki MCP handler.
#[derive(Debug, Clone)]
pub(crate) struct WikiMcpServer {
    repo_root: PathBuf,
}

impl WikiMcpServer {
    /// Creates a server bound to one generated repository.
    pub(crate) fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
        }
    }

    /// Handles one request without live model or network calls.
    pub(crate) fn handle(&self, request: McpRequest) -> McpResponse {
        match self.handle_result(&request) {
            Ok(result) => McpResponse {
                id: request.id,
                result: Some(result),
                error: None,
            },
            Err(error) => McpResponse {
                id: request.id,
                result: None,
                error: Some(error.to_string()),
            },
        }
    }

    fn handle_result(&self, request: &McpRequest) -> Result<Value, Box<dyn std::error::Error>> {
        let export = WikiSearch.export(&self.repo_root, None)?;
        match request.tool.as_str() {
            "list_tools" => Ok(serde_json::to_value(MCP_TOOLS)?),
            "read_wiki_structure" => Ok(serde_json::to_value(export.structure)?),
            "read_wiki_contents" => Ok(serde_json::to_value(export.contents)?),
            "read_research_memory" => {
                let path = self
                    .repo_root
                    .join(".lithograph/research/agent-memory.json");
                let value: Value = serde_json::from_str(&std::fs::read_to_string(path)?)?;
                Ok(value)
            }
            "save_research_result" => {
                let question = required_string_param(&request.params, "question")?;
                let answer = required_string_param(&request.params, "answer")?;
                let outcome = required_string_param(&request.params, "outcome")?
                    .parse::<AnswerOutcome>()
                    .map_err(std::io::Error::other)?;
                let cited_node_ids = request
                    .params
                    .get("cited_node_ids")
                    .and_then(Value::as_array)
                    .ok_or("save_research_result requires params.cited_node_ids")?
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or("cited_node_ids must contain only strings")
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let correction = request
                    .params
                    .get("correction")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let recorded_at = optional_u64_param(&request.params, "recorded_at")?
                    .map_or_else(unix_timestamp_now, Ok)?;
                Ok(serde_json::to_value(
                    ResearchFeedbackStore::new(&self.repo_root).save_result(AnswerResultInput {
                        question,
                        answer,
                        cited_node_ids,
                        outcome,
                        correction,
                        recorded_at,
                    })?,
                )?)
            }
            "reflect_research" => {
                let now = optional_u64_param(&request.params, "now")?
                    .map_or_else(unix_timestamp_now, Ok)?;
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(
                    ResearchFeedbackStore::new(&self.repo_root).reflect(&graph, now)?,
                )?)
            }
            "read_research_lessons" => Ok(serde_json::to_value(
                ResearchFeedbackStore::new(&self.repo_root).read_lessons()?,
            )?),
            "list_documentable_subsystems" => {
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(list_subsystems(&graph))?)
            }
            "resolve_tag_expression" => {
                let graph = self.load_graph()?;
                let expression = request
                    .params
                    .get("expression")
                    .and_then(Value::as_str)
                    .ok_or("missing expression")?;
                let index = TagIndex::new(derive_tags(&graph, "current"));
                Ok(serde_json::to_value(
                    resolve_expression(&index, expression).map_err(std::io::Error::other)?,
                )?)
            }
            "get_tag_facets" => {
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(
                    TagIndex::new(derive_tags(&graph, "current")).facets(),
                )?)
            }
            "list_tags" => {
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(
                    TagIndex::new(derive_tags(&graph, "current")).all(),
                )?)
            }
            "search_tag_prefix" => {
                let graph = self.load_graph()?;
                let prefix = request
                    .params
                    .get("prefix")
                    .and_then(Value::as_str)
                    .ok_or("missing prefix")?;
                Ok(serde_json::to_value(
                    TagIndex::new(derive_tags(&graph, "current")).search_prefix(prefix),
                )?)
            }
            "get_tagged_subgraph" => {
                let graph = self.load_graph()?;
                let expression = request
                    .params
                    .get("expression")
                    .and_then(Value::as_str)
                    .ok_or("missing expression")?;
                let selected =
                    resolve_expression(&TagIndex::new(derive_tags(&graph, "current")), expression)
                        .map_err(std::io::Error::other)?;
                let nodes: Vec<_> = graph
                    .nodes
                    .iter()
                    .filter(|node| selected.iter().any(|id| id == node.id().as_str()))
                    .collect();
                let relations: Vec<_> = graph
                    .relations
                    .iter()
                    .filter(|edge| {
                        selected
                            .iter()
                            .any(|id| id == edge.source.as_str() || id == edge.target.as_str())
                    })
                    .collect();
                Ok(json!({"nodes": nodes, "relations": relations, "expression": expression}))
            }
            "generate_subsystem_document" => {
                let graph = self.load_graph()?;
                let snapshot_id = graph_snapshot_id(&graph)?;
                let tensions = score_tensions(&graph, &HealthThresholds::default(), &[]);
                let subsystem = request
                    .params
                    .get("subsystem")
                    .and_then(Value::as_str)
                    .ok_or("missing subsystem")?;
                let instruction = request.params.get("instruction").and_then(Value::as_str);
                let tag_expression = request.params.get("tag_expression").and_then(Value::as_str);
                let explicit_nodes = request
                    .params
                    .get("node_ids")
                    .and_then(Value::as_array)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(Value::as_str)
                            .map(GraphNodeId::new)
                            .collect::<Vec<_>>()
                    });
                let mut document = if let Some(selected) = explicit_nodes {
                    generate_subsystem_doc_for_nodes(
                        &graph,
                        subsystem,
                        &snapshot_id,
                        &tensions,
                        instruction,
                        &selected,
                    )
                } else if let Some(expression) = tag_expression {
                    let selected = resolve_expression(
                        &TagIndex::new(derive_tags(&graph, "current")),
                        expression,
                    )
                    .map_err(std::io::Error::other)?;
                    let selected = selected
                        .into_iter()
                        .map(crate::graph::GraphNodeId::new)
                        .collect::<Vec<_>>();
                    generate_subsystem_doc_for_nodes(
                        &graph,
                        subsystem,
                        &snapshot_id,
                        &tensions,
                        instruction,
                        &selected,
                    )
                } else {
                    generate_subsystem_doc(&graph, subsystem, &snapshot_id, &tensions, instruction)
                };
                document.tag_expression = tag_expression.map(str::to_owned);
                SubsystemDocumentStore::new(self.repo_root.join(".lithograph/subsystem-docs"))
                    .save(&document)?;
                Ok(serde_json::to_value(document)?)
            }
            "get_subsystem_graph_context"
            | "refine_subsystem_document"
            | "validate_subsystem_document" => {
                let graph = self.load_graph()?;
                let snapshot_id = graph_snapshot_id(&graph)?;
                let tensions = score_tensions(&graph, &HealthThresholds::default(), &[]);
                let subsystem = request
                    .params
                    .get("subsystem")
                    .and_then(Value::as_str)
                    .ok_or("missing subsystem")?;
                let instruction = request.params.get("instruction").and_then(Value::as_str);
                let tag_expression = request.params.get("tag_expression").and_then(Value::as_str);
                let explicit_nodes = request
                    .params
                    .get("node_ids")
                    .and_then(Value::as_array)
                    .map(|ids| {
                        ids.iter()
                            .filter_map(Value::as_str)
                            .map(GraphNodeId::new)
                            .collect::<Vec<_>>()
                    });
                let mut document = if let Some(selected) = explicit_nodes {
                    generate_subsystem_doc_for_nodes(
                        &graph,
                        subsystem,
                        &snapshot_id,
                        &tensions,
                        instruction,
                        &selected,
                    )
                } else if let Some(expression) = tag_expression {
                    let selected = resolve_expression(
                        &TagIndex::new(derive_tags(&graph, "current")),
                        expression,
                    )
                    .map_err(std::io::Error::other)?;
                    let selected = selected
                        .into_iter()
                        .map(crate::graph::GraphNodeId::new)
                        .collect::<Vec<_>>();
                    generate_subsystem_doc_for_nodes(
                        &graph,
                        subsystem,
                        &snapshot_id,
                        &tensions,
                        instruction,
                        &selected,
                    )
                } else {
                    generate_subsystem_doc(&graph, subsystem, &snapshot_id, &tensions, instruction)
                };
                document.tag_expression = tag_expression.map(str::to_owned);
                if request.tool == "validate_subsystem_document" {
                    let valid = document
                        .cited_nodes
                        .iter()
                        .all(|id| graph.nodes.iter().any(|node| node.id() == id));
                    Ok(
                        json!({"valid": valid, "fresh": true, "graph_snapshot_id": document.graph_snapshot_id}),
                    )
                } else {
                    if request.tool == "refine_subsystem_document" {
                        SubsystemDocumentStore::new(
                            self.repo_root.join(".lithograph/subsystem-docs"),
                        )
                        .save(&document)?;
                    }
                    Ok(serde_json::to_value(document)?)
                }
            }
            "get_graph_layout" => {
                let graph = self.load_graph()?;
                let request = layout_params(&request.params)?;
                let store = LayoutSnapshotStore::new(self.repo_root.join(".lithograph/layout"));
                let result = compute_layout_cached(&graph, &request, &store)
                    .map_err(std::io::Error::other)?;
                Ok(serde_json::to_value(result)?)
            }
            "get_graph_analytics" => {
                let graph = self.load_graph()?;
                let degrees = degree_metrics(&graph);
                let pagerank = page_rank(&graph, 20)
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>();
                let between = betweenness(&graph, BetweennessPolicy::default())
                    .into_iter()
                    .collect::<std::collections::BTreeMap<_, _>>();
                let nodes = degrees
                    .into_iter()
                    .map(|(id, fan_in, fan_out)| {
                        json!({
                            "id": id, "fan_in": fan_in, "fan_out": fan_out,
                            "page_rank": pagerank.get(&id).copied().unwrap_or_default(),
                            "betweenness": between.get(&id).copied().unwrap_or_default(),
                        })
                    })
                    .collect::<Vec<_>>();
                Ok(
                    json!({"nodes": nodes, "findings": detect_health(&graph, &HealthThresholds::default())}),
                )
            }
            "get_repository_tensions" => {
                let graph = self.load_graph()?;
                let snapshot_id = graph_snapshot_id(&graph)?;
                let mut tensions = score_tensions(&graph, &HealthThresholds::default(), &[]);
                for tension in &mut tensions {
                    tension.tags = tension_display_tags(tension, &snapshot_id);
                }
                Ok(serde_json::to_value(tensions)?)
            }
            "get_node_detail" => {
                let graph = self.load_graph()?;
                let snapshot_id = graph_snapshot_id(&graph)?;
                let node_id = request
                    .params
                    .get("node_id")
                    .and_then(Value::as_str)
                    .ok_or("get_node_detail requires params.node_id")?;
                Ok(serde_json::to_value(node_detail(
                    &self.repo_root,
                    &graph,
                    node_id,
                    &snapshot_id,
                )?)?)
            }
            "get_graph_document" | "regenerate_graph_document" => {
                let graph = self.load_graph()?;
                let snapshot_id = graph_snapshot_id(&graph)?;
                let tensions = score_tensions(&graph, &HealthThresholds::default(), &[]);
                let current = generate_graph_docs(&graph, &tensions, &snapshot_id);
                let path = self.repo_root.join(".lithograph/graph-document.json");
                let stored: Option<StoredGraphDocument> = JsonStore.read(&path)?;
                let had_stored = stored.is_some();
                let baseline = stored.unwrap_or_else(|| {
                    StoredGraphDocument::new(current.0.clone(), current.1.clone())
                });
                let selected = request
                    .params
                    .get("section_ids")
                    .and_then(Value::as_array)
                    .map(|values| {
                        values
                            .iter()
                            .filter_map(Value::as_str)
                            .collect::<BTreeSet<_>>()
                    });
                let next = if request.tool == "regenerate_graph_document" {
                    regenerate_graph_document(&baseline, &current.0, &current.1, selected.as_ref())
                } else {
                    baseline.clone()
                };
                let regenerated = request.tool == "regenerate_graph_document" && next != baseline;
                if !had_stored || regenerated {
                    JsonStore.write_if_changed(&path, &next)?;
                }
                Ok(graph_document_response(
                    &next,
                    &baseline,
                    &current.0,
                    regenerated,
                ))
            }
            "get_schema" | "get_graph_schema" => {
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(KnowledgeIndex::new(&graph).schema())?)
            }
            "search_graph" => {
                let graph = self.load_graph()?;
                let params = search_params(&request.params);
                Ok(serde_json::to_value(
                    KnowledgeIndex::new(&graph).search(&params),
                )?)
            }
            "explain_env" => {
                let graph = self.load_graph()?;
                let variable = request.params.get("variable").and_then(Value::as_str);
                Ok(serde_json::to_value(explain_environment(&graph, variable))?)
            }
            "search_code" => {
                let walk_options = WalkOptions {
                    exclude_globs: crate::orchestrate::scan_exclude_globs(),
                    ..WalkOptions::default()
                };
                let artifacts = RepositoryWalker::new(walk_options).walk(&self.repo_root)?;
                let graph = self.load_graph()?;
                let modules = ModulePlanner.plan(&graph, &artifacts);
                let params = code_search_params(&request.params);
                Ok(serde_json::to_value(CodeSearch.search(
                    &self.repo_root,
                    &artifacts,
                    &graph,
                    &modules,
                    &params,
                ))?)
            }
            "search_fulltext" => {
                let index: crate::retrieval::fts::FtsIndex = JsonStore
                    .read(&self.repo_root.join(".lithograph/fts-index.json"))?
                    .ok_or("no FTS index found; run init or update first")?;
                let query = request
                    .params
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or("search_fulltext requires params.query")?;
                let limit = request
                    .params
                    .get("limit")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                Ok(serde_json::to_value(index.search(query, limit))?)
            }
            "search_semantic" => {
                let graph = self.load_graph()?;
                let query = request
                    .params
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or("search_semantic requires params.query")?;
                let limit = request
                    .params
                    .get("limit")
                    .and_then(Value::as_u64)
                    .and_then(|value| usize::try_from(value).ok())
                    .unwrap_or_default();
                // Deterministic and offline, matching this server's design
                // (LIT-22.4.4 AC3): no live model or network call.
                let results = crate::retrieval::semantic_search::SemanticSearch.search(
                    &crate::retrieval::semantic_search::MockEmbeddingProvider,
                    &graph,
                    query,
                    limit,
                    crate::retrieval::semantic_search::SemanticSearchWeights::default(),
                )?;
                Ok(serde_json::to_value(results)?)
            }
            "query_graph" => {
                let graph = self.load_graph()?;
                let text = request
                    .params
                    .get("query")
                    .and_then(Value::as_str)
                    .ok_or("query_graph requires params.query")?;
                let query = crate::retrieval::query::parse(text)?;
                Ok(serde_json::to_value(crate::retrieval::query::evaluate(
                    &query, &graph,
                ))?)
            }
            "trace_path" => {
                let graph = self.load_graph()?;
                let params = trace_params(&request.params)?;
                match KnowledgeIndex::new(&graph).trace(&params) {
                    Some(trace) => Ok(serde_json::to_value(trace)?),
                    None => Err(format!("no graph node matched `{}`", params.query).into()),
                }
            }
            "impact_analysis" => {
                let graph = self.load_graph()?;
                let params = trace_params(&request.params)?;
                match KnowledgeIndex::new(&graph).impact_analysis(&params) {
                    Some(trace) => Ok(serde_json::to_value(trace)?),
                    None => Err(format!("no graph node matched `{}`", params.query).into()),
                }
            }
            "find_dead_code" => {
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(
                    KnowledgeIndex::new(&graph).find_dead_code(),
                )?)
            }
            "detect_changes" => Ok(serde_json::to_value(self.detect_changes()?)?),
            "get_run_metrics" => Ok(serde_json::to_value(self.get_run_metrics()?)?),
            "detect_drift" => {
                let walk_options = WalkOptions {
                    exclude_globs: crate::orchestrate::cache_exclude_globs(),
                    ..WalkOptions::default()
                };
                let artifacts = RepositoryWalker::new(walk_options).walk(&self.repo_root)?;
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(
                    crate::knowledge::drift::DriftDetector.scan(
                        &artifacts,
                        &graph,
                        &self.repo_root,
                    ),
                )?)
            }
            "get_architecture" => {
                let graph = self.load_graph()?;
                let aspects = architecture_aspects(&request.params)?;
                let snapshot_id = graph_snapshot_id(&graph)?;
                let mut summary = KnowledgeIndex::new(&graph).architecture(aspects.as_ref());
                for cluster in &mut summary.clusters {
                    cluster.tags = cluster_display_tags(cluster, &snapshot_id);
                }
                Ok(serde_json::to_value(summary)?)
            }
            "create_adr" => {
                let params = &request.params;
                let title = params
                    .get("title")
                    .and_then(Value::as_str)
                    .ok_or("create_adr requires params.title")?;
                let context = params
                    .get("context")
                    .and_then(Value::as_str)
                    .ok_or("create_adr requires params.context")?;
                let decision = params
                    .get("decision")
                    .and_then(Value::as_str)
                    .ok_or("create_adr requires params.decision")?;
                let consequences = params.get("consequences").and_then(Value::as_str);
                Ok(serde_json::to_value(
                    crate::docs::adr::AdrStore::new(&self.repo_root).create(
                        title,
                        context,
                        decision,
                        consequences,
                    )?,
                )?)
            }
            "get_adr" => {
                let id = request
                    .params
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("get_adr requires params.id")?;
                Ok(serde_json::to_value(
                    crate::docs::adr::AdrStore::new(&self.repo_root).get(id)?,
                )?)
            }
            "update_adr" => {
                let params = &request.params;
                let id = params
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("update_adr requires params.id")?;
                let store = crate::docs::adr::AdrStore::new(&self.repo_root);
                let mut record = store.get(id)?;
                if let (Some(section), Some(value)) = (
                    params.get("section").and_then(Value::as_str),
                    params.get("value").and_then(Value::as_str),
                ) {
                    record = store.update_section(id, section, value)?;
                }
                if let Some(status) = params.get("status").and_then(Value::as_str) {
                    let status = adr_status_from_str(status)?;
                    record = store.update_status(id, status)?;
                }
                Ok(serde_json::to_value(record)?)
            }
            "delete_adr" => {
                let id = request
                    .params
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("delete_adr requires params.id")?;
                crate::docs::adr::AdrStore::new(&self.repo_root).delete(id)?;
                Ok(json!({ "deleted": id }))
            }
            "list_adrs" => Ok(serde_json::to_value(
                crate::docs::adr::AdrStore::new(&self.repo_root).list(),
            )?),
            "ask_question" => {
                let question = request
                    .params
                    .get("question")
                    .and_then(Value::as_str)
                    .ok_or("ask_question requires params.question")?;
                Ok(serde_json::to_value(
                    WikiSearch.ask(&self.repo_root, question)?,
                )?)
            }
            other => Ok(json!({
                "available_tools": MCP_TOOLS.iter().map(|tool| tool.name).collect::<Vec<_>>(),
                "message": format!("unknown tool `{other}`")
            })),
        }
    }

    fn load_graph(&self) -> Result<Graph, Box<dyn std::error::Error>> {
        Ok(GraphStore::new(&self.repo_root).load()?.graph)
    }

    /// Artifact paths added, removed, or content-changed since the last
    /// `init`/`update` run's `.lithograph/snapshot.json`. Reuses that
    /// snapshot's own pipeline metadata (rather than the current binary's)
    /// so a version bump alone never reports every file as "changed" --
    /// this only ever reports real content differences.
    fn detect_changes(&self) -> Result<Vec<String>, Box<dyn std::error::Error>> {
        let snapshot_path = self.repo_root.join(".lithograph/snapshot.json");
        let previous: Option<RepositorySnapshot> = JsonStore.read(&snapshot_path)?;
        let walk_options = WalkOptions {
            exclude_globs: crate::orchestrate::scan_exclude_globs(),
            ..WalkOptions::default()
        };
        let artifacts = RepositoryWalker::new(walk_options).walk(&self.repo_root)?;
        let pipeline = previous
            .as_ref()
            .map(|snapshot| snapshot.pipeline.clone())
            .unwrap_or_default();
        let current = RepositorySnapshot::from_artifacts(&artifacts, pipeline);
        Ok(current.changed_since(previous.as_ref()))
    }

    /// Reads the last recorded `.lithograph/run.json` (LIT-22.8.4 AC2:
    /// MCP-caller visibility into index/generation time, graph size, cache
    /// hit rate, and estimated prompt tokens).
    fn get_run_metrics(&self) -> Result<RunMetadata, Box<dyn std::error::Error>> {
        let run_metadata_path = self.repo_root.join(".lithograph/run.json");
        JsonStore.read(&run_metadata_path)?.ok_or_else(|| {
            format!(
                "no run metadata found at {}; run `init` or `update` first",
                run_metadata_path.display()
            )
            .into()
        })
    }

    /// Runs a JSON-line request loop until EOF.
    pub(crate) fn run<R, W>(
        &self,
        reader: R,
        writer: &mut W,
    ) -> Result<(), Box<dyn std::error::Error>>
    where
        R: BufRead,
        W: Write,
    {
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            let response = match serde_json::from_str::<McpRequest>(&line) {
                Ok(request) => self.handle(request),
                Err(error) => McpResponse {
                    id: Value::Null,
                    result: None,
                    error: Some(format!("invalid request JSON: {error}")),
                },
            };
            serde_json::to_writer(&mut *writer, &response)?;
            writer.write_all(b"\n")?;
        }
        Ok(())
    }
}

fn graph_snapshot_id(graph: &Graph) -> Result<String, Box<dyn std::error::Error>> {
    Ok(format!(
        "blake3:{}",
        blake3::hash(graph.to_json()?.as_bytes()).to_hex()
    ))
}

const GRAPH_DOC_CONTEXT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct StoredGraphDocument {
    document: GraphDocument,
    markdown: String,
    prompt_context_version: u32,
}

impl StoredGraphDocument {
    fn new(document: GraphDocument, markdown: String) -> Self {
        Self {
            document,
            markdown,
            prompt_context_version: GRAPH_DOC_CONTEXT_VERSION,
        }
    }
}

fn regenerate_graph_document(
    previous: &StoredGraphDocument,
    current: &GraphDocument,
    markdown: &str,
    selected: Option<&BTreeSet<&str>>,
) -> StoredGraphDocument {
    let Some(selected) = selected else {
        return StoredGraphDocument::new(current.clone(), markdown.to_owned());
    };
    let mut document = previous.document.clone();
    document.graph_snapshot_id = current.graph_snapshot_id.clone();
    for section in &mut document.sections {
        if selected.contains(section.id.as_str())
            && let Some(replacement) = current
                .sections
                .iter()
                .find(|candidate| candidate.id == section.id)
        {
            *section = replacement.clone();
        }
    }
    StoredGraphDocument::new(document, markdown.to_owned())
}

fn graph_document_response(
    value: &StoredGraphDocument,
    previous: &StoredGraphDocument,
    current: &GraphDocument,
    regenerated: bool,
) -> Value {
    let current_by_id = current.section_index();
    let section_freshness = value.document.sections.iter().map(|section| {
        let current_section = current_by_id.get(&section.id).copied();
        let source_query_hash = stable_hash(&section.source_query_ids);
        let evidence_hash = stable_hash(&section.evidence_references);
        let current_source_hash = current_section.map(|item| stable_hash(&item.source_query_ids));
        let current_evidence_hash = current_section.map(|item| stable_hash(&item.evidence_references));
        let mut reasons = Vec::new();
        if section.graph_snapshot_id != current.graph_snapshot_id { reasons.push("graph snapshot changed"); }
        if current_source_hash.as_ref() != Some(&source_query_hash) { reasons.push("source query inputs changed"); }
        if current_evidence_hash.as_ref() != Some(&evidence_hash) { reasons.push("evidence references changed"); }
        let status = if reasons.is_empty() { "current" } else if reasons.len() == 1 && reasons[0] == "graph snapshot changed" { "partially_stale" } else { "stale" };
        json!({ "section_id": section.id, "status": status, "source_query_hash": source_query_hash, "evidence_hash": evidence_hash, "prompt_context_version": value.prompt_context_version, "drift_findings": reasons })
    }).collect::<Vec<_>>();
    let previous_by_id = previous.document.section_index();
    let diff = value.document.sections.iter().filter_map(|section| {
        let before = previous_by_id.get(&section.id).copied();
        if before == Some(section) { return None; }
        Some(json!({ "section_id": section.id, "title": section.title, "before": before.map(section_summary), "after": section_summary(section) }))
    }).collect::<Vec<_>>();
    let freshness = if section_freshness
        .iter()
        .all(|item| item["status"] == "current")
    {
        "current"
    } else {
        "stale"
    };
    json!({ "document": value.document, "markdown": value.markdown, "freshness": freshness, "section_freshness": section_freshness, "diff": diff, "regenerated": regenerated })
}

fn section_summary(section: &GraphDocumentSection) -> String {
    format!(
        "{} · {} nodes · {} evidence · snapshot {}",
        section.title,
        section.affected_nodes.len(),
        section.evidence_references.len(),
        section.graph_snapshot_id
    )
}

fn stable_hash<T: Serialize>(value: &T) -> String {
    let bytes = match serde_json::to_vec(value) {
        Ok(bytes) => bytes,
        Err(_) => b"serialization-error".to_vec(),
    };
    format!("blake3:{}", blake3::hash(&bytes).to_hex())
}

fn node_detail(
    repo_root: &Path,
    graph: &Graph,
    node_id: &str,
    snapshot_id: &str,
) -> Result<NodeDetail, Box<dyn std::error::Error>> {
    let node = graph
        .nodes
        .iter()
        .find(|node| node.id().as_str() == node_id)
        .ok_or_else(|| format!("unknown graph node `{node_id}`"))?;
    let evidence = node_evidence(node);
    let source = source_excerpt(repo_root, evidence.first());
    let definitions = graph
        .relations
        .iter()
        .filter(|relation| {
            relation.source == *node.id()
                && matches!(
                    relation.kind,
                    RelationKind::Contains | RelationKind::HasMethod | RelationKind::MemberOf
                )
        })
        .filter_map(|relation| related_node(graph, &relation.target))
        .collect();
    let related_docs = graph
        .relations
        .iter()
        .filter(|relation| relation.kind == RelationKind::DocumentsSource)
        .filter_map(|relation| {
            if relation.source == *node.id() {
                related_node(graph, &relation.target)
            } else if relation.target == *node.id() {
                related_node(graph, &relation.source)
            } else {
                None
            }
        })
        .collect();
    let references = graph
        .relations
        .iter()
        .filter_map(|relation| relation_for_node(graph, relation, node.id(), snapshot_id))
        .collect();
    let tags = derive_tags(graph, snapshot_id)
        .into_iter()
        .filter(|tag| tag.entity_id == node.id().as_str())
        .collect();

    Ok(NodeDetail {
        id: node.id().as_str().to_owned(),
        label: node_label(node).to_owned(),
        name: node_name(node),
        evidence,
        source,
        definitions,
        references,
        related_docs,
        tags,
    })
}

fn node_evidence(node: &GraphNode) -> Vec<NodeEvidence> {
    let evidence = match node {
        GraphNode::Artifact(node) => Some(&node.evidence),
        GraphNode::Symbol(node) => Some(&node.evidence),
        GraphNode::Config(node) => Some(&node.evidence),
        GraphNode::Documentation(node) => Some(&node.evidence),
        GraphNode::Command(node) => Some(&node.evidence),
        GraphNode::Module(node) => Some(&node.evidence),
        GraphNode::Rationale(node) => Some(&node.evidence),
        GraphNode::Container(_)
        | GraphNode::EnvVar(_)
        | GraphNode::Package(_)
        | GraphNode::Unresolved(_) => None,
    };
    evidence.into_iter().map(evidence_detail).collect()
}

fn evidence_detail(evidence: &crate::domain::EvidenceRef) -> NodeEvidence {
    NodeEvidence {
        path: evidence.path.as_str().to_owned(),
        start_line: evidence.span.as_ref().map(|span| span.start_line),
        end_line: evidence.span.as_ref().map(|span| span.end_line),
    }
}

fn source_excerpt(repo_root: &Path, evidence: Option<&NodeEvidence>) -> SourceExcerpt {
    let Some(evidence) = evidence else {
        return SourceExcerpt {
            status: SourceStatus::Opaque,
            text: None,
            message: Some("This node has no repository source evidence.".to_owned()),
        };
    };
    let relative = Path::new(&evidence.path);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| matches!(part, Component::ParentDir))
    {
        return SourceExcerpt {
            status: SourceStatus::Opaque,
            text: None,
            message: Some("The evidence path is not safe to read.".to_owned()),
        };
    }
    let path = repo_root.join(relative);
    let source = match std::fs::read_to_string(&path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return SourceExcerpt {
                status: SourceStatus::Missing,
                text: None,
                message: Some("The source file is no longer present in this checkout.".to_owned()),
            };
        }
        Err(_) => {
            return SourceExcerpt {
                status: SourceStatus::Opaque,
                text: None,
                message: Some(
                    "The source is generated, binary, or cannot be displayed as text.".to_owned(),
                ),
            };
        }
    };
    let lines = source.lines().collect::<Vec<_>>();
    let start = evidence.start_line.unwrap_or(1).saturating_sub(1) as usize;
    let end = evidence.end_line.unwrap_or_else(|| (start + 40) as u32) as usize;
    let first = start.saturating_sub(3).min(lines.len());
    let last = end.min(first + 80).min(lines.len());
    let text = lines[first..last]
        .iter()
        .enumerate()
        .map(|(offset, line)| format!("{:>5} | {line}", first + offset + 1))
        .collect::<Vec<_>>()
        .join("\n");
    SourceExcerpt {
        status: SourceStatus::Available,
        text: Some(text),
        message: None,
    }
}

fn related_node(graph: &Graph, id: &GraphNodeId) -> Option<RelatedNode> {
    graph
        .nodes
        .iter()
        .find(|node| node.id() == id)
        .map(|node| RelatedNode {
            id: node.id().as_str().to_owned(),
            label: node_label(node).to_owned(),
            name: node_name(node),
        })
}

fn relation_for_node(
    graph: &Graph,
    relation: &Relation,
    node_id: &GraphNodeId,
    snapshot_id: &str,
) -> Option<RelatedRelation> {
    let (direction, counterpart) = if relation.source == *node_id {
        ("outbound", related_node(graph, &relation.target)?)
    } else if relation.target == *node_id {
        ("inbound", related_node(graph, &relation.source)?)
    } else {
        return None;
    };
    Some(RelatedRelation {
        id: relation.id.clone(),
        direction,
        kind: relation.kind,
        counterpart,
        evidence: relation.evidence.iter().map(evidence_detail).collect(),
        resolver_strategy: relation
            .provenance
            .as_ref()
            .map(|value| value.resolver_strategy.clone()),
        confidence: relation.confidence,
        tags: relation_display_tags(relation, snapshot_id),
    })
}

fn required_string_param(params: &Value, name: &str) -> Result<String, Box<dyn std::error::Error>> {
    params
        .get(name)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("missing or invalid string param `{name}`").into())
}

fn optional_u64_param(
    params: &Value,
    name: &str,
) -> Result<Option<u64>, Box<dyn std::error::Error>> {
    match params.get(name) {
        None | Some(Value::Null) => Ok(None),
        Some(value) => value
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("param `{name}` must be an unsigned integer").into()),
    }
}

fn search_params(params: &Value) -> SearchParams {
    SearchParams {
        label: params
            .get("label")
            .and_then(Value::as_str)
            .map(str::to_owned),
        query: params
            .get("query")
            .or_else(|| params.get("name_pattern"))
            .and_then(Value::as_str)
            .map(str::to_owned),
        limit: params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_default(),
    }
}

fn code_search_params(params: &Value) -> CodeSearchParams {
    CodeSearchParams {
        query: params
            .get("query")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned(),
        path_contains: params
            .get("path_contains")
            .and_then(Value::as_str)
            .map(str::to_owned),
        language: params
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_owned),
        module_id: params
            .get("module_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        package: params
            .get("package")
            .and_then(Value::as_str)
            .map(str::to_owned),
        graph_node_id: params
            .get("graph_node_id")
            .and_then(Value::as_str)
            .map(str::to_owned),
        limit: params
            .get("limit")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_default(),
    }
}

fn trace_params(params: &Value) -> Result<TraceParams, Box<dyn std::error::Error>> {
    let query = params
        .get("query")
        .or_else(|| params.get("node_id"))
        .or_else(|| params.get("name"))
        .or_else(|| params.get("function_name"))
        .and_then(Value::as_str)
        .ok_or("trace_path requires params.query, params.node_id, params.name, or params.function_name")?
        .to_owned();
    let direction = match params
        .get("direction")
        .and_then(Value::as_str)
        .unwrap_or("both")
        .to_ascii_lowercase()
        .as_str()
    {
        "inbound" | "in" => TraceDirection::Inbound,
        "outbound" | "out" => TraceDirection::Outbound,
        _ => TraceDirection::Both,
    };
    Ok(TraceParams {
        query,
        depth: params
            .get("depth")
            .and_then(Value::as_u64)
            .and_then(|value| usize::try_from(value).ok())
            .unwrap_or_default(),
        direction,
    })
}

/// Parses `get_graph_layout` params: `center_node?`, `radius?`, `max_nodes?`,
/// `max_edges?`, `node_labels?` (array of label strings), `edge_types?`
/// (array of `RelationKind` names). An invalid `edge_types` entry is a
/// validation error, not a silently-ignored one, matching `architecture_aspects`.
fn layout_params(params: &Value) -> Result<LayoutRequest, Box<dyn std::error::Error>> {
    let center_node = params
        .get("center_node")
        .and_then(Value::as_str)
        .map(str::to_owned);
    let radius = params
        .get("radius")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    let max_nodes = params
        .get("max_nodes")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    let max_edges = params
        .get("max_edges")
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
        .unwrap_or_default();
    let node_labels = match params.get("node_labels") {
        None | Some(Value::Null) => BTreeSet::new(),
        Some(value) => value
            .as_array()
            .ok_or("params.node_labels must be an array of label strings")?
            .iter()
            .map(|entry| {
                entry
                    .as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| "params.node_labels entries must be strings".into())
            })
            .collect::<Result<BTreeSet<String>, Box<dyn std::error::Error>>>()?,
    };
    let node_ids = match params.get("node_ids") {
        None | Some(Value::Null) => BTreeSet::new(),
        Some(value) => value
            .as_array()
            .ok_or("params.node_ids must be an array of node-id strings")?
            .iter()
            .map(|entry| {
                entry
                    .as_str()
                    .map(crate::graph::GraphNodeId::new)
                    .ok_or_else(|| "params.node_ids entries must be strings".into())
            })
            .collect::<Result<BTreeSet<_>, Box<dyn std::error::Error>>>()?,
    };
    let edge_types = match params.get("edge_types") {
        None | Some(Value::Null) => BTreeSet::new(),
        Some(value) => value
            .as_array()
            .ok_or("params.edge_types must be an array of relation kind names")?
            .iter()
            .map(|name| {
                serde_json::from_value::<RelationKind>(name.clone()).map_err(|error| {
                    format!("invalid params.edge_types entry {name}: {error}").into()
                })
            })
            .collect::<Result<BTreeSet<RelationKind>, Box<dyn std::error::Error>>>()?,
    };
    let hide_unresolved = match params.get("hide_unresolved") {
        None | Some(Value::Null) => false,
        Some(value) => value
            .as_bool()
            .ok_or("params.hide_unresolved must be a boolean")?,
    };
    Ok(LayoutRequest {
        center_node,
        radius,
        max_nodes,
        max_edges,
        node_labels,
        node_ids,
        edge_types,
        hide_unresolved,
    })
}

/// Parses `params.aspects` (an array of section names, e.g.
/// `["packages", "layers"]`) into the typed filter `get_architecture`
/// passes to [`KnowledgeIndex::architecture`] (LIT-22.4.6 AC2). Absent or
/// `null` means "every aspect"; an unrecognized name is a validation error,
/// not a silently-ignored one.
fn architecture_aspects(
    params: &Value,
) -> Result<Option<BTreeSet<ArchitectureAspect>>, Box<dyn std::error::Error>> {
    let Some(aspects) = params.get("aspects") else {
        return Ok(None);
    };
    if aspects.is_null() {
        return Ok(None);
    }
    let names = aspects
        .as_array()
        .ok_or("params.aspects must be an array of section names")?;
    let parsed = names
        .iter()
        .map(|name| {
            serde_json::from_value::<ArchitectureAspect>(name.clone())
                .map_err(|error| format!("invalid params.aspects entry {name}: {error}").into())
        })
        .collect::<Result<BTreeSet<ArchitectureAspect>, Box<dyn std::error::Error>>>()?;
    Ok(Some(parsed))
}

/// Parses one `params.status` string into [`crate::docs::adr::AdrStatus`] with a
/// validation error for an unrecognized value.
fn adr_status_from_str(
    status: &str,
) -> Result<crate::docs::adr::AdrStatus, Box<dyn std::error::Error>> {
    serde_json::from_value(Value::String(status.to_owned()))
        .map_err(|error| format!("invalid params.status `{status}`: {error}").into())
}

#[cfg(test)]
mod tests {
    use super::{
        MCP_TOOLS, McpRequest, StoredGraphDocument, WikiMcpServer, graph_document_response,
        regenerate_graph_document,
    };
    use crate::docs::graph_docs::generate_graph_docs;
    use crate::generation::MockModel;
    use crate::graph::{Graph, GraphStore};
    use crate::orchestrate::run_init;
    use serde_json::Value;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::io::Cursor;
    use std::path::Path;

    #[test]
    fn handles_structure_contents_and_question_requests() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let structure = server.handle(McpRequest {
            id: json!(1),
            tool: "read_wiki_structure".to_owned(),
            params: json!({}),
        });
        let research = server.handle(McpRequest {
            id: json!(2),
            tool: "read_research_memory".to_owned(),
            params: json!({}),
        });
        let schema = server.handle(McpRequest {
            id: json!(3),
            tool: "get_graph_schema".to_owned(),
            params: json!({}),
        });
        let search = server.handle(McpRequest {
            id: json!(4),
            tool: "search_graph".to_owned(),
            params: json!({ "label": "Artifact", "query": "python", "limit": 5 }),
        });
        let trace_query = search
            .result
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| item.get("id"))
            .and_then(Value::as_str)
            .unwrap_or("src/python_app/service.py")
            .to_owned();
        let trace = server.handle(McpRequest {
            id: json!(5),
            tool: "trace_path".to_owned(),
            params: json!({ "query": trace_query, "depth": 1 }),
        });
        let architecture = server.handle(McpRequest {
            id: json!(6),
            tool: "get_architecture".to_owned(),
            params: json!({}),
        });
        let answer = server.handle(McpRequest {
            id: json!(7),
            tool: "ask_question".to_owned(),
            params: json!({ "question": "source evidence" }),
        });

        assert!(structure.result.is_some());
        assert!(
            research
                .result
                .as_ref()
                .is_some_and(|value| value.get("architecture").is_some())
        );
        // LIT-22.6.5 AC1/AC4: MCP access exposes the versioned index fields,
        // not just the raw reports.
        assert!(
            research
                .result
                .as_ref()
                .is_some_and(|value| value.get("schema_version") == Some(&json!(1)))
        );
        assert!(
            research
                .result
                .as_ref()
                .is_some_and(|value| value.get("report_keys").is_some_and(Value::is_array))
        );
        assert!(
            research
                .result
                .as_ref()
                .is_some_and(|value| value.get("input_hash").is_some_and(Value::is_string))
        );
        assert!(
            schema
                .result
                .as_ref()
                .is_some_and(|value| value.get("node_labels").is_some())
        );
        assert!(
            search
                .result
                .as_ref()
                .and_then(Value::as_array)
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            trace
                .result
                .as_ref()
                .is_some_and(|value| value.get("visited").is_some())
        );
        assert!(
            architecture
                .result
                .as_ref()
                .is_some_and(|value| value.get("hotspots").is_some())
        );
        assert!(answer.result.is_some());
        assert!(answer.error.is_none());

        Ok(())
    }

    #[test]
    fn research_feedback_tools_save_reflect_and_read_lessons()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());
        let node_id = GraphStore::new(temp.path()).load()?.graph.nodes[0]
            .id()
            .as_str()
            .to_owned();

        for (id, question) in [(1, "where?"), (2, "which?")] {
            let response = server.handle(McpRequest {
                id: json!(id),
                tool: "save_research_result".to_owned(),
                params: json!({
                    "question": question,
                    "answer": "here",
                    "cited_node_ids": [node_id.clone()],
                    "outcome": "useful",
                    "recorded_at": 100
                }),
            });
            assert!(response.error.is_none(), "{:?}", response.error);
        }
        let reflected = server.handle(McpRequest {
            id: json!(3),
            tool: "reflect_research".to_owned(),
            params: json!({ "now": 100 }),
        });
        assert!(reflected.error.is_none(), "{:?}", reflected.error);
        assert_eq!(
            reflected.result.as_ref().and_then(|value| value
                .get("preferred_sources")
                .and_then(Value::as_array)
                .map(Vec::len)),
            Some(1)
        );
        let read = server.handle(McpRequest {
            id: json!(4),
            tool: "read_research_lessons".to_owned(),
            params: json!({}),
        });
        assert_eq!(read.result, reflected.result);
        Ok(())
    }

    #[test]
    fn layout_params_parses_hide_unresolved_flag() -> Result<(), Box<dyn std::error::Error>> {
        assert!(!super::layout_params(&json!({}))?.hide_unresolved);
        assert!(!super::layout_params(&json!({ "hide_unresolved": false }))?.hide_unresolved);
        assert!(super::layout_params(&json!({ "hide_unresolved": true }))?.hide_unresolved);
        assert!(super::layout_params(&json!({ "hide_unresolved": "yes" })).is_err());
        Ok(())
    }

    #[test]
    fn get_graph_layout_supports_overview_and_detail_and_caches_results()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let overview = server.handle(McpRequest {
            id: json!(1),
            tool: "get_graph_layout".to_owned(),
            params: json!({ "max_nodes": 5 }),
        });
        assert!(overview.error.is_none(), "{:?}", overview.error);
        let overview_result = overview.result.as_ref().ok_or("missing overview result")?;
        assert!(
            overview_result
                .get("center_node")
                .is_some_and(Value::is_null)
        );
        let nodes = overview_result
            .get("nodes")
            .and_then(Value::as_array)
            .ok_or("expected nodes array")?;
        assert_eq!(nodes.len(), 5);
        let edge = overview_result
            .get("edges")
            .and_then(Value::as_array)
            .and_then(|edges| edges.first())
            .ok_or("expected a layout edge")?;
        assert!(edge.get("id").is_some_and(Value::is_string));
        assert!(edge.get("resolution").is_some_and(Value::is_string));
        assert!(edge.get("confidence").is_some_and(Value::is_string));
        assert!(edge.get("resolver_strategy").is_some());
        assert!(
            overview_result
                .get("budget")
                .and_then(|budget| budget.get("nodes_truncated"))
                .is_some_and(|value| value == &json!(true))
        );

        let center_id = nodes
            .first()
            .and_then(|node| node.get("id"))
            .and_then(Value::as_str)
            .ok_or("expected a node id")?
            .to_owned();
        let detail = server.handle(McpRequest {
            id: json!(2),
            tool: "get_graph_layout".to_owned(),
            params: json!({ "center_node": center_id, "radius": 1 }),
        });
        assert!(detail.error.is_none(), "{:?}", detail.error);
        assert_eq!(
            detail
                .result
                .as_ref()
                .and_then(|value| value.get("center_node")),
            Some(&json!(center_id))
        );

        // A second identical overview request must be a cache hit served
        // from `.lithograph/layout` rather than a fresh computation.
        assert!(temp.path().join(".lithograph/layout").is_dir());
        let overview_again = server.handle(McpRequest {
            id: json!(3),
            tool: "get_graph_layout".to_owned(),
            params: json!({ "max_nodes": 5 }),
        });
        assert_eq!(overview_again.result, overview.result);

        let bad_center = server.handle(McpRequest {
            id: json!(4),
            tool: "get_graph_layout".to_owned(),
            params: json!({ "center_node": "does-not-exist" }),
        });
        assert!(bad_center.error.is_some());

        Ok(())
    }

    #[test]
    fn get_node_detail_returns_typed_evidence_and_gracefully_handles_missing_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());
        let search = server.handle(McpRequest {
            id: json!(1),
            tool: "search_graph".to_owned(),
            params: json!({ "label": "Artifact", "query": "python_app" }),
        });
        let id = search
            .result
            .as_ref()
            .and_then(Value::as_array)
            .and_then(|nodes| nodes.first())
            .and_then(|node| node.get("id"))
            .and_then(Value::as_str)
            .ok_or("fixture artifact missing")?
            .to_owned();
        let detail = server.handle(McpRequest {
            id: json!(2),
            tool: "get_node_detail".to_owned(),
            params: json!({ "node_id": id }),
        });
        let detail = detail.result.ok_or("missing detail response")?;
        assert_eq!(detail["source"]["status"], json!("available"));
        assert!(
            detail["source"]["text"]
                .as_str()
                .is_some_and(|text| !text.is_empty())
        );
        assert!(
            detail["evidence"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(detail["references"].is_array());
        assert!(
            detail["tags"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
        );
        assert!(
            detail["tags"]
                .as_array()
                .is_some_and(|items| items.iter().all(|tag| tag["graph_snapshot_id"]
                    .as_str()
                    .is_some_and(|snapshot| snapshot.starts_with("blake3:"))))
        );
        assert!(
            detail["references"]
                .as_array()
                .is_some_and(|items| items.iter().all(|relation| relation["tags"]
                    .as_array()
                    .is_some_and(|tags| tags.iter().all(|tag| tag["graph_snapshot_id"]
                        .as_str()
                        .is_some_and(|snapshot| snapshot.starts_with("blake3:"))))))
        );

        let subsystem_doc = server.handle(McpRequest {
            id: json!(3),
            tool: "generate_subsystem_document".to_owned(),
            params: json!({ "subsystem": "focused-fixture", "node_ids": [id] }),
        });
        let subsystem_doc = subsystem_doc.result.ok_or("missing subsystem document")?;
        assert_eq!(
            subsystem_doc["cited_nodes"].as_array().map(Vec::len),
            Some(1)
        );
        assert!(
            subsystem_doc["graph_snapshot_id"]
                .as_str()
                .is_some_and(|snapshot| snapshot.starts_with("blake3:"))
        );
        let refined = server.handle(McpRequest {
            id: json!(4),
            tool: "refine_subsystem_document".to_owned(),
            params: json!({ "subsystem": "focused-fixture", "node_ids": [id], "instruction": "add operations" }),
        });
        assert!(
            refined
                .result
                .as_ref()
                .and_then(|value| value["markdown"].as_str())
                .is_some_and(|markdown| markdown.contains("add operations"))
        );

        let path = detail["evidence"][0]["path"]
            .as_str()
            .ok_or("detail evidence lacks a path")?;
        std::fs::remove_file(temp.path().join(path))?;
        let missing = server.handle(McpRequest {
            id: json!(5),
            tool: "get_node_detail".to_owned(),
            params: json!({ "node_id": id }),
        });
        assert_eq!(
            missing
                .result
                .as_ref()
                .map(|result| &result["source"]["status"]),
            Some(&json!("missing"))
        );
        assert_eq!(
            super::source_excerpt(temp.path(), None).status,
            super::SourceStatus::Opaque
        );
        Ok(())
    }

    #[test]
    fn explain_env_is_deterministic_and_handles_missing_variables()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        std::fs::write(
            temp.path().join("application.properties"),
            "RIDGELINE_WORKER=/usr/local/bin/worker\n",
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let request = || McpRequest {
            id: json!(1),
            tool: "explain_env".to_owned(),
            params: json!({ "variable": "RIDGELINE_WORKER" }),
        };
        let first = server.handle(request());
        let second = server.handle(request());
        assert_eq!(first, second);
        let variables = first
            .result
            .as_ref()
            .and_then(|value| value.get("variables"))
            .and_then(Value::as_array)
            .ok_or("explain_env should return variables")?;
        assert_eq!(variables.len(), 1);
        assert!(variables[0].get("code_users").is_some());
        assert!(variables[0].get("resolved").is_some());

        let missing = server.handle(McpRequest {
            id: json!(2),
            tool: "explain_env".to_owned(),
            params: json!({ "variable": "DOES_NOT_EXIST" }),
        });
        assert_eq!(
            missing
                .result
                .as_ref()
                .and_then(|value| value.get("variables"))
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
        Ok(())
    }

    /// LIT-22.4.1 AC1/AC2/AC3: `get_schema` (the `get_graph_schema` alias),
    /// `impact_analysis`, `find_dead_code`, and `detect_changes` each
    /// return a typed success response, and `impact_analysis` returns a
    /// validation error for a query that matches no graph node.
    #[test]
    fn handles_new_query_tools_and_validation_errors() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let schema = server.handle(McpRequest {
            id: json!(1),
            tool: "get_schema".to_owned(),
            params: json!({}),
        });
        assert!(
            schema
                .result
                .as_ref()
                .is_some_and(|value| value.get("node_labels").is_some())
        );
        assert!(schema.error.is_none());

        let dead_code = server.handle(McpRequest {
            id: json!(2),
            tool: "find_dead_code".to_owned(),
            params: json!({}),
        });
        assert!(dead_code.result.as_ref().is_some_and(Value::is_array));
        assert!(dead_code.error.is_none());

        let changes = server.handle(McpRequest {
            id: json!(3),
            tool: "detect_changes".to_owned(),
            params: json!({}),
        });
        // No new snapshot has been written since `run_init`'s own, so
        // nothing has changed relative to it.
        assert_eq!(changes.result, Some(json!([])));
        assert!(changes.error.is_none());

        let metrics = server.handle(McpRequest {
            id: json!(4),
            tool: "get_run_metrics".to_owned(),
            params: json!({}),
        });
        assert!(
            metrics
                .result
                .as_ref()
                .is_some_and(|value| value.get("graph_node_count").is_some_and(Value::is_u64))
        );
        assert!(
            metrics
                .result
                .as_ref()
                .is_some_and(|value| value.get("estimated_prompt_tokens").is_some())
        );
        assert!(metrics.error.is_none());

        let impact = server.handle(McpRequest {
            id: json!(5),
            tool: "impact_analysis".to_owned(),
            params: json!({ "query": "src/python_app/service.py" }),
        });
        assert!(
            impact
                .result
                .as_ref()
                .is_some_and(|value| value.get("visited").is_some())
        );
        assert!(impact.error.is_none());

        let impact_error = server.handle(McpRequest {
            id: json!(6),
            tool: "impact_analysis".to_owned(),
            params: json!({ "query": "no-such-node-anywhere" }),
        });
        assert!(impact_error.result.is_none());
        assert!(
            impact_error
                .error
                .as_ref()
                .is_some_and(|error| error.contains("no graph node matched"))
        );

        let trace_missing_query = server.handle(McpRequest {
            id: json!(7),
            tool: "trace_path".to_owned(),
            params: json!({}),
        });
        assert!(trace_missing_query.result.is_none());
        assert!(trace_missing_query.error.is_some());

        Ok(())
    }

    /// LIT-22.8.4 AC2: `get_run_metrics` reports an actionable error rather
    /// than a panic or an empty/fabricated result when no run metadata has
    /// been recorded yet.
    #[test]
    fn get_run_metrics_reports_an_actionable_error_when_run_json_is_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        std::fs::remove_file(temp.path().join(".lithograph/run.json"))?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "get_run_metrics".to_owned(),
            params: json!({}),
        });

        assert!(response.result.is_none());
        assert!(
            response
                .error
                .as_ref()
                .is_some_and(|error| error.contains("run `init` or `update` first"))
        );

        Ok(())
    }

    /// LIT-22.4.2 AC1/AC2: `search_code` is reachable through the MCP
    /// server, honors filters, and returns evidence-carrying results.
    #[test]
    fn search_code_returns_filtered_results_with_evidence() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "search_code".to_owned(),
            params: json!({ "query": "class RouteService", "language": "python" }),
        });

        let results = response
            .result
            .as_ref()
            .and_then(Value::as_array)
            .ok_or("expected an array of results")?;
        assert!(!results.is_empty());
        assert!(
            results[0]
                .get("artifact_path")
                .and_then(Value::as_str)
                .is_some_and(|path| path.ends_with(".py"))
        );
        assert!(results[0].get("evidence").is_some());

        let empty_query = server.handle(McpRequest {
            id: json!(2),
            tool: "search_code".to_owned(),
            params: json!({}),
        });
        assert_eq!(empty_query.result, Some(json!([])));

        Ok(())
    }

    /// Regression test: `search_code` and `detect_drift` used to walk with a
    /// bare `WalkOptions::default()`, so on an already-`init`ed repository
    /// they re-ingested `.lithograph/cache/analysis/*.json` as if it were
    /// repository source (see the matching `commands.rs` regression test for
    /// the full story, including the live LIT-22 comparison against
    /// codebase-memory-mcp on `ridgeline` that surfaced this).
    #[test]
    fn search_code_and_detect_drift_never_rescan_lithographs_own_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        assert!(
            std::fs::read_dir(temp.path().join(".lithograph/cache/analysis"))?
                .next()
                .is_some(),
            "expected init to have populated the analysis cache"
        );
        let server = WikiMcpServer::new(temp.path());

        let search = server.handle(McpRequest {
            id: json!(1),
            tool: "search_code".to_owned(),
            params: json!({ "query": "e" }),
        });
        let results = search
            .result
            .as_ref()
            .and_then(Value::as_array)
            .ok_or("expected an array of results")?;
        assert!(!results.is_empty());
        assert!(results.iter().all(|result| {
            result
                .get("artifact_path")
                .and_then(Value::as_str)
                .is_some_and(|path| !path.starts_with(".lithograph/"))
        }));

        let drift = server.handle(McpRequest {
            id: json!(2),
            tool: "detect_drift".to_owned(),
            params: json!({}),
        });
        assert!(
            !drift
                .result
                .map(|value| value.to_string())
                .unwrap_or_default()
                .contains(".lithograph/")
        );
        assert!(drift.error.is_none());

        Ok(())
    }

    /// LIT-22.4.3 AC1/AC2: `search_fulltext` reads the FTS index persisted
    /// by `init` and returns BM25-ranked results, and rejects a request
    /// with no `params.query`.
    #[test]
    fn search_fulltext_returns_ranked_results_and_requires_a_query()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "search_fulltext".to_owned(),
            params: json!({ "query": "RouteService" }),
        });
        let results = response
            .result
            .as_ref()
            .and_then(Value::as_array)
            .ok_or("expected an array of results")?;
        assert!(!results.is_empty());
        assert!(results[0].get("score").and_then(Value::as_f64).is_some());

        let missing_query = server.handle(McpRequest {
            id: json!(2),
            tool: "search_fulltext".to_owned(),
            params: json!({}),
        });
        assert!(missing_query.result.is_none());
        assert!(
            missing_query
                .error
                .as_ref()
                .is_some_and(|error| error.contains("requires params.query"))
        );

        Ok(())
    }

    /// LIT-22.4.4 AC2/AC4: `search_semantic` is reachable through the MCP
    /// server and returns evidence-carrying, blended-score results.
    #[test]
    fn search_semantic_returns_blended_results_with_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "search_semantic".to_owned(),
            params: json!({ "query": "RouteService route handling" }),
        });

        let results = response
            .result
            .as_ref()
            .and_then(Value::as_array)
            .ok_or("expected an array of results")?;
        assert!(!results.is_empty());
        assert!(
            results[0]
                .get("combined_score")
                .and_then(Value::as_f64)
                .is_some()
        );
        assert!(
            results
                .iter()
                .any(|result| result.get("evidence").is_some_and(|e| !e.is_null()))
        );

        Ok(())
    }

    /// LIT-22.4.5 AC1/AC2/AC3: `query_graph` evaluates a valid MATCH query
    /// through the MCP server, and rejects an invalid one with an
    /// actionable error rather than a bare failure.
    #[test]
    fn query_graph_evaluates_valid_queries_and_rejects_invalid_ones()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "query_graph".to_owned(),
            params: json!({ "query": "MATCH (a:Artifact)-[:Contains]->(b:Symbol) RETURN a, b" }),
        });
        let rows = response
            .result
            .as_ref()
            .and_then(Value::as_array)
            .ok_or("expected an array of rows")?;
        assert!(!rows.is_empty());
        assert!(rows[0].get("id").is_some());

        let invalid = server.handle(McpRequest {
            id: json!(2),
            tool: "query_graph".to_owned(),
            params: json!({ "query": "SELECT * FROM nodes" }),
        });
        assert!(invalid.result.is_none());
        assert!(invalid.error.is_some());

        Ok(())
    }

    /// LIT-22.4.6 AC2/AC4: `get_architecture` accepts `params.aspects` to
    /// filter its sections, and rejects an unrecognized aspect name.
    #[test]
    fn get_architecture_honors_aspect_filter_and_validates_names()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let full = server.handle(McpRequest {
            id: json!(1),
            tool: "get_architecture".to_owned(),
            params: json!({}),
        });
        assert!(full.result.as_ref().is_some_and(
            |value| value.get("clusters").is_some() && value.get("file_tree").is_some()
        ));

        let filtered = server.handle(McpRequest {
            id: json!(2),
            tool: "get_architecture".to_owned(),
            params: json!({ "aspects": ["packages"] }),
        });
        let filtered_value = filtered.result.ok_or("missing filtered result")?;
        assert!(
            filtered_value
                .get("packages")
                .and_then(Value::as_array)
                .is_some_and(|packages| !packages.is_empty())
        );
        assert!(
            filtered_value
                .get("clusters")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty)
        );

        let invalid = server.handle(McpRequest {
            id: json!(3),
            tool: "get_architecture".to_owned(),
            params: json!({ "aspects": ["not-a-real-aspect"] }),
        });
        assert!(invalid.result.is_none());
        assert!(
            invalid
                .error
                .as_ref()
                .is_some_and(|error| error.contains("invalid params.aspects"))
        );

        Ok(())
    }

    /// LIT-22.5.3 AC3: `detect_drift` is available through the MCP server.
    #[test]
    fn detect_drift_reports_findings() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        std::fs::write(
            temp.path().join("stray.md"),
            "TODO: document the `/health` endpoint.\n",
        )?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "detect_drift".to_owned(),
            params: json!({}),
        });
        assert!(response.error.is_none());
        assert!(
            response
                .result
                .as_ref()
                .and_then(|value| value.get("findings"))
                .and_then(Value::as_array)
                .is_some_and(|findings| !findings.is_empty())
        );

        Ok(())
    }

    /// LIT-22.5.4 AC3/AC4: create/get/update/list/delete ADR operations are
    /// available through the MCP server, including validation errors.
    #[test]
    fn adr_tools_create_get_update_list_delete_round_trip() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let created = server.handle(McpRequest {
            id: json!(1),
            tool: "create_adr".to_owned(),
            params: json!({
                "title": "Use Postgres",
                "context": "We need a database.",
                "decision": "Use Postgres.",
            }),
        });
        let created_value = created.result.ok_or("missing create_adr result")?;
        let id = created_value
            .get("id")
            .and_then(Value::as_str)
            .ok_or("missing id")?
            .to_owned();
        assert_eq!(created_value.get("status"), Some(&json!("proposed")));

        let missing_field = server.handle(McpRequest {
            id: json!(2),
            tool: "create_adr".to_owned(),
            params: json!({ "context": "x", "decision": "y" }),
        });
        assert!(missing_field.result.is_none());
        assert!(
            missing_field
                .error
                .as_ref()
                .is_some_and(|error| error.contains("params.title"))
        );

        let fetched = server.handle(McpRequest {
            id: json!(3),
            tool: "get_adr".to_owned(),
            params: json!({ "id": id }),
        });
        assert_eq!(fetched.result, Some(created_value));

        let updated = server.handle(McpRequest {
            id: json!(4),
            tool: "update_adr".to_owned(),
            params: json!({
                "id": id,
                "section": "consequences",
                "value": "Adds an ops dependency.",
                "status": "accepted",
            }),
        });
        let updated_value = updated.result.ok_or("missing update_adr result")?;
        assert_eq!(updated_value.get("status"), Some(&json!("accepted")));
        assert_eq!(
            updated_value
                .get("sections")
                .and_then(|sections| sections.get("consequences")),
            Some(&json!("Adds an ops dependency."))
        );

        let invalid_section = server.handle(McpRequest {
            id: json!(5),
            tool: "update_adr".to_owned(),
            params: json!({ "id": id, "section": "not-a-real-section", "value": "x" }),
        });
        assert!(invalid_section.result.is_none());
        assert!(
            invalid_section
                .error
                .as_ref()
                .is_some_and(|error| error.contains("unknown ADR section"))
        );

        let listed = server.handle(McpRequest {
            id: json!(6),
            tool: "list_adrs".to_owned(),
            params: json!({}),
        });
        assert_eq!(
            listed
                .result
                .as_ref()
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(1)
        );

        let deleted = server.handle(McpRequest {
            id: json!(7),
            tool: "delete_adr".to_owned(),
            params: json!({ "id": id }),
        });
        assert!(deleted.error.is_none());

        let not_found = server.handle(McpRequest {
            id: json!(8),
            tool: "get_adr".to_owned(),
            params: json!({ "id": id }),
        });
        assert!(not_found.result.is_none());
        assert!(
            not_found
                .error
                .as_ref()
                .is_some_and(|error| error.contains("no ADR found"))
        );

        Ok(())
    }

    #[test]
    fn json_line_loop_reports_invalid_request_without_stopping()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let input = Cursor::new(
            b"not-json\n{\"id\":3,\"tool\":\"read_wiki_contents\",\"params\":{}}\n".to_vec(),
        );
        let mut output = Vec::new();

        WikiMcpServer::new(temp.path()).run(input, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("invalid request JSON"));
        assert!(output.contains("\"id\":3"));

        Ok(())
    }

    /// LIT-22.8.1 AC1/AC2: `list_tools` deterministically enumerates every
    /// tool this server implements, across every capability category
    /// (graph/search/architecture/impact/dead-code/ADR/research-memory/
    /// doc-content), and matches the same list the wiki export exposes
    /// (AC4) -- the two can never drift since both read `MCP_TOOLS`.
    #[test]
    fn list_tools_enumerates_every_capability_and_matches_wiki_export()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "list_tools".to_owned(),
            params: json!({}),
        });
        let names: Vec<String> = response
            .result
            .as_ref()
            .and_then(Value::as_array)
            .ok_or("expected an array of tools")?
            .iter()
            .map(|tool| {
                tool.get("name")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();

        for expected in [
            "search_graph",         // graph/search
            "get_architecture",     // architecture
            "impact_analysis",      // impact
            "find_dead_code",       // dead-code
            "create_adr",           // ADR
            "read_research_memory", // research-memory
            "read_wiki_contents",   // doc-content
        ] {
            assert!(names.contains(&expected.to_owned()), "missing {expected}");
        }

        let export = crate::agent::ask::WikiSearch.export(temp.path(), None)?;
        assert_eq!(names, export.tools);

        Ok(())
    }

    /// LIT-22.8.1 AC2/AC3: an unrecognized tool name gets a deterministic,
    /// actionable error result naming the tool that was requested plus
    /// every tool that IS available, rather than a bare failure.
    #[test]
    fn unknown_tool_name_lists_available_tools() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let server = WikiMcpServer::new(temp.path());

        let response = server.handle(McpRequest {
            id: json!(1),
            tool: "not_a_real_tool".to_owned(),
            params: json!({}),
        });

        assert!(response.error.is_none());
        let result = response.result.ok_or("expected a result")?;
        assert_eq!(
            result.get("message").and_then(Value::as_str),
            Some("unknown tool `not_a_real_tool`")
        );
        assert!(
            result
                .get("available_tools")
                .and_then(Value::as_array)
                .is_some_and(|tools| tools.len() == MCP_TOOLS.len())
        );

        Ok(())
    }

    #[test]
    fn graph_document_freshness_supports_no_op_and_section_level_regeneration() {
        let graph = Graph {
            nodes: vec![],
            relations: vec![],
        };
        let (g1, markdown) = generate_graph_docs(&graph, &[], "g1");
        let (g2, current_markdown) = generate_graph_docs(&graph, &[], "g2");
        let previous = StoredGraphDocument::new(g1.clone(), markdown.clone());

        assert_eq!(
            regenerate_graph_document(&previous, &g1, &markdown, None),
            previous
        );

        let selected_id = g1.sections[0].id.as_str();
        let selected = BTreeSet::from([selected_id]);
        let regenerated =
            regenerate_graph_document(&previous, &g2, &current_markdown, Some(&selected));
        assert_eq!(regenerated.document.sections[0].graph_snapshot_id, "g2");
        assert!(
            regenerated
                .document
                .sections
                .iter()
                .skip(1)
                .any(|section| section.graph_snapshot_id == "g1")
        );

        let response = graph_document_response(&regenerated, &previous, &g2, true);
        assert_eq!(response["freshness"], "stale");
        assert_eq!(response["diff"].as_array().map(Vec::len), Some(1));
        assert!(
            response["section_freshness"]
                .as_array()
                .is_some_and(|items| items.iter().any(|item| item["status"] == "partially_stale"))
        );
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for entry in walk_files(from)? {
            let relative = entry.strip_prefix(from)?;
            let destination = to.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry, &destination)?;
        }
        Ok(())
    }

    fn walk_files(root: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        Ok(files)
    }
}
