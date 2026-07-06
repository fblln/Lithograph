//! Minimal deterministic MCP-like JSON-line server over generated wiki data.

use crate::ask::WikiSearch;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

/// One request accepted by the local server.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct McpRequest {
    /// Client-provided request id.
    pub id: Value,
    /// Tool name: read_wiki_structure, read_wiki_contents, or ask_question.
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

#[cfg(test)]
mod tests {
    use super::{McpRequest, WikiMcpServer};
    use crate::generation::MockModel;
    use crate::orchestrate::run_init;
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
        let answer = server.handle(McpRequest {
            id: json!(2),
            tool: "ask_question".to_owned(),
            params: json!({ "question": "source evidence" }),
        });

        assert!(structure.result.is_some());
        assert!(answer.result.is_some());
        assert!(answer.error.is_none());

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
