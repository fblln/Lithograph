//! Minimal deterministic MCP-like JSON-line server over generated wiki data.

use crate::ask::WikiSearch;
use crate::graph::{
    ArchitectureAspect, Graph, GraphStore, KnowledgeIndex, SearchParams, TraceDirection,
    TraceParams,
};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::plan::ModulePlanner;
use crate::run::RepositorySnapshot;
use crate::search::{CodeSearch, CodeSearchParams};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// One MCP tool's stable name and human-readable purpose (LIT-22.8.1
/// AC1/AC2): deterministic and schema-like enough for a caller to
/// discover what's available without guessing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct McpToolInfo {
    /// Stable tool name, passed as `McpRequest.tool`.
    pub name: &'static str,
    /// What the tool does and, where not obvious, its required params.
    pub description: &'static str,
}

/// Every tool this server implements, in a stable order. The single
/// source of truth for both the `list_tools` response and the wiki
/// export's tool listing (AC4), so the two can never silently drift
/// apart as new tools are added.
pub const MCP_TOOLS: &[McpToolInfo] = &[
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
        name: "get_graph_schema",
        description: "Returns the knowledge graph's node/relation label schema.",
    },
    McpToolInfo {
        name: "search_graph",
        description: "Searches graph nodes. Params: label?, query?, limit?.",
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
pub struct McpRequest {
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
pub struct McpResponse {
    /// Echoed request id.
    pub id: Value,
    /// Response payload on success.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Error text on failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Deterministic wiki MCP handler.
#[derive(Debug, Clone)]
pub struct WikiMcpServer {
    repo_root: PathBuf,
}

impl WikiMcpServer {
    /// Creates a server bound to one generated repository.
    pub fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
        }
    }

    /// Handles one request without live model or network calls.
    pub fn handle(&self, request: McpRequest) -> McpResponse {
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
            "search_code" => {
                let artifacts =
                    RepositoryWalker::new(WalkOptions::default()).walk(&self.repo_root)?;
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
                let index: crate::fts::FtsIndex = JsonStore
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
                let results = crate::semantic_search::SemanticSearch.search(
                    &crate::semantic_search::MockEmbeddingProvider,
                    &graph,
                    query,
                    limit,
                    crate::semantic_search::SemanticSearchWeights::default(),
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
                let query = crate::query::parse(text)?;
                Ok(serde_json::to_value(crate::query::evaluate(
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
            "detect_drift" => {
                let artifacts =
                    RepositoryWalker::new(WalkOptions::default()).walk(&self.repo_root)?;
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(crate::drift::DriftDetector.scan(
                    &artifacts,
                    &graph,
                    &self.repo_root,
                ))?)
            }
            "get_architecture" => {
                let graph = self.load_graph()?;
                let aspects = architecture_aspects(&request.params)?;
                Ok(serde_json::to_value(
                    KnowledgeIndex::new(&graph).architecture(aspects.as_ref()),
                )?)
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
                    crate::adr::AdrStore::new(&self.repo_root).create(
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
                    crate::adr::AdrStore::new(&self.repo_root).get(id)?,
                )?)
            }
            "update_adr" => {
                let params = &request.params;
                let id = params
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("update_adr requires params.id")?;
                let store = crate::adr::AdrStore::new(&self.repo_root);
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
                crate::adr::AdrStore::new(&self.repo_root).delete(id)?;
                Ok(json!({ "deleted": id }))
            }
            "list_adrs" => Ok(serde_json::to_value(
                crate::adr::AdrStore::new(&self.repo_root).list(),
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

    /// Runs a JSON-line request loop until EOF.
    pub fn run<R, W>(&self, reader: R, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
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

/// Parses one `params.status` string into [`crate::adr::AdrStatus`] with a
/// validation error for an unrecognized value.
fn adr_status_from_str(status: &str) -> Result<crate::adr::AdrStatus, Box<dyn std::error::Error>> {
    serde_json::from_value(Value::String(status.to_owned()))
        .map_err(|error| format!("invalid params.status `{status}`: {error}").into())
}

#[cfg(test)]
mod tests {
    use super::{MCP_TOOLS, McpRequest, WikiMcpServer};
    use crate::generation::MockModel;
    use crate::orchestrate::run_init;
    use serde_json::Value;
    use serde_json::json;
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

        let impact = server.handle(McpRequest {
            id: json!(4),
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
            id: json!(5),
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
            id: json!(6),
            tool: "trace_path".to_owned(),
            params: json!({}),
        });
        assert!(trace_missing_query.result.is_none());
        assert!(trace_missing_query.error.is_some());

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

        let export = crate::ask::WikiSearch.export(temp.path(), None)?;
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
