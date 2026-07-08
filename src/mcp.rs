//! Minimal deterministic MCP-like JSON-line server over generated wiki data.

use crate::ask::WikiSearch;
use crate::graph::{Graph, GraphStore, KnowledgeIndex, SearchParams, TraceDirection, TraceParams};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::run::RepositorySnapshot;
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

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
            "get_architecture" => {
                let graph = self.load_graph()?;
                Ok(serde_json::to_value(
                    KnowledgeIndex::new(&graph).architecture(),
                )?)
            }
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
                "available_tools": export.tools,
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

#[cfg(test)]
mod tests {
    use super::{McpRequest, WikiMcpServer};
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
