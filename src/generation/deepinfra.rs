//! DeepInfra model adapter, built on `rig-core`'s OpenAI-compatible client
//! pointed at DeepInfra's endpoint.
//!
//! `rig-core`'s completion/agent API is async (tokio + reqwest), while
//! [`LanguageModel`] is deliberately synchronous (Lithograph is a
//! sequential single-process batch CLI, not a concurrency-serving
//! service). Rather than making the trait async for one backend, this
//! adapter owns a private current-thread tokio runtime used only to
//! bridge that gap internally.

use crate::generation::llm::{LanguageModel, ModelError, ModelRequest, PageGeneration};
use rig_core::client::CompletionClient;
use rig_core::completion::Prompt;
use rig_core::providers::openai;
use serde_json::{Map, Value};

const DEFAULT_BASE_URL: &str = "https://api.deepinfra.com/v1/openai";

/// Configuration for the DeepInfra adapter.
#[derive(Debug, Clone)]
pub struct DeepInfraConfig {
    /// DeepInfra's OpenAI-compatible base URL.
    pub base_url: String,
    /// Bearer API key. Never included verbatim in error messages.
    pub api_key: String,
    /// Model name to request, e.g. a DeepSeek model path hosted on DeepInfra.
    pub model: String,
    /// Reasoning effort for reasoning models (e.g. `"low"`/`"medium"`/`"high"`).
    /// Omitted from the request when unset.
    pub reasoning_effort: Option<String>,
}

impl DeepInfraConfig {
    /// Builds a configuration pointed at DeepInfra's default base URL.
    pub fn new(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.to_owned(),
            api_key: api_key.into(),
            model: model.into(),
            reasoning_effort: None,
        }
    }

    /// Overrides the base URL (for tests, or DeepInfra-compatible proxies).
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = base_url.into();
        self
    }

    /// Sets the reasoning effort sent with each request.
    pub fn with_reasoning_effort(mut self, reasoning_effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(reasoning_effort.into());
        self
    }
}

/// DeepInfra [`LanguageModel`] adapter.
///
/// Uses `openai::CompletionsClient` rather than the crate's default
/// `openai::Client` (which targets OpenAI's newer Responses API):
/// DeepInfra, like most third-party OpenAI-compatible providers, only
/// implements the classic `/chat/completions` shape.
pub struct DeepInfraModel {
    client: openai::CompletionsClient,
    config: DeepInfraConfig,
    runtime: tokio::runtime::Runtime,
}

impl DeepInfraModel {
    /// Builds an adapter from `config`.
    pub fn new(config: DeepInfraConfig) -> Result<Self, ModelError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| ModelError {
                message: format!("failed to start async runtime: {error}"),
            })?;
        let client = openai::CompletionsClient::builder()
            .api_key(config.api_key.clone())
            .base_url(&config.base_url)
            .build()
            .map_err(|error| ModelError {
                message: redact(&config.api_key, &error.to_string()),
            })?;
        Ok(Self {
            client,
            config,
            runtime,
        })
    }

    fn chat(&self, request: &ModelRequest, json_mode: bool) -> Result<String, ModelError> {
        let mut params = Map::new();
        if json_mode {
            params.insert(
                "response_format".to_owned(),
                Value::Object(Map::from_iter([(
                    "type".to_owned(),
                    Value::String("json_object".to_owned()),
                )])),
            );
        }
        if let Some(reasoning_effort) = &self.config.reasoning_effort {
            params.insert(
                "reasoning_effort".to_owned(),
                Value::String(reasoning_effort.clone()),
            );
        }

        let mut builder = self
            .client
            .agent(self.config.model.clone())
            .preamble(&request.system_prompt);
        if !params.is_empty() {
            builder = builder.additional_params(Value::Object(params));
        }
        let agent = builder.build();

        let user_prompt = request.user_prompt.clone();
        self.runtime
            .block_on(async move { agent.prompt(user_prompt).await })
            .map_err(|error| ModelError {
                message: redact(&self.config.api_key, &error.to_string()),
            })
    }
}

impl LanguageModel for DeepInfraModel {
    fn generate_text(&self, request: &ModelRequest) -> Result<String, ModelError> {
        self.chat(request, false)
    }

    fn generate_json(&self, request: &ModelRequest) -> Result<PageGeneration, ModelError> {
        let content = self.chat(request, true)?;
        serde_json::from_str(&content).map_err(|error| ModelError {
            message: format!("model response was not valid PageGeneration JSON: {error}"),
        })
    }
}

fn redact(api_key: &str, message: &str) -> String {
    if api_key.is_empty() {
        message.to_owned()
    } else {
        message.replace(api_key, "***REDACTED***")
    }
}

#[cfg(test)]
mod tests {
    use super::{DeepInfraConfig, DeepInfraModel};
    use crate::generation::llm::LanguageModel;
    use crate::manifest::TaskKind;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::{Arc, Mutex};

    fn request() -> super::ModelRequest {
        super::ModelRequest {
            model: "deepseek-test".to_owned(),
            prompt_version: "v1".to_owned(),
            task_kind: TaskKind::ModulePage,
            input_hash: "hash".to_owned(),
            system_prompt: "system".to_owned(),
            user_prompt: "user".to_owned(),
        }
    }

    /// Reads one HTTP/1.1 request off `stream` (headers + body, using
    /// Content-Length) and returns the body, so tests can assert on the
    /// exact JSON sent, keeping this a minimal loopback stub rather than a
    /// full HTTP server (mirrors `generation::openai`'s test harness).
    fn drain_request(stream: &mut TcpStream) -> Result<String, Box<dyn std::error::Error>> {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte)?;
            buffer.push(byte[0]);
            if buffer.len() >= 4 && buffer[buffer.len() - 4..] == *b"\r\n\r\n" {
                break;
            }
        }
        let headers = String::from_utf8_lossy(&buffer).into_owned();
        let content_length: usize = headers
            .lines()
            .find_map(|line| {
                line.to_ascii_lowercase()
                    .strip_prefix("content-length:")
                    .map(|value| value.trim().to_owned())
            })
            .and_then(|value| value.parse().ok())
            .unwrap_or(0);
        let mut body = vec![0u8; content_length];
        stream.read_exact(&mut body)?;
        Ok(String::from_utf8(body)?)
    }

    fn respond(stream: &mut TcpStream, body: &str) -> Result<(), Box<dyn std::error::Error>> {
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes())?;
        Ok(())
    }

    type CapturedRequests = Arc<Mutex<Vec<String>>>;

    fn spawn_server(
        responses: Vec<String>,
    ) -> Result<(String, CapturedRequests), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let requests = Arc::new(Mutex::new(Vec::new()));
        let requests_handle = requests.clone();
        std::thread::spawn(move || {
            for body in responses {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let Ok(request_body) = drain_request(&mut stream) else {
                    break;
                };
                requests_handle
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .push(request_body);
                if respond(&mut stream, &body).is_err() {
                    break;
                }
            }
        });
        Ok((format!("http://{addr}"), requests))
    }

    const OK_CONTENT: &str = "{\"title\":\"T\",\"summary\":\"S\",\"evidence_refs\":[],\"unresolved_questions\":[],\"body\":\"# T\\n\"}";

    /// Builds a full `openai::completion::CompletionResponse`-shaped body
    /// (rig-core requires every non-defaulted field to be present, unlike
    /// the minimal shape the raw `generation::openai` test harness posts).
    fn chat_response_body(content: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok(format!(
            "{{\"id\":\"chatcmpl-test\",\"object\":\"chat.completion\",\"created\":1700000000,\
             \"model\":\"deepseek-test\",\"system_fingerprint\":null,\"usage\":null,\
             \"choices\":[{{\"index\":0,\"message\":{{\"role\":\"assistant\",\"content\":{}}},\
             \"logprobs\":null,\"finish_reason\":\"stop\"}}]}}",
            serde_json::to_string(content)?
        ))
    }

    #[test]
    fn generate_json_parses_a_successful_response() -> Result<(), Box<dyn std::error::Error>> {
        let body = chat_response_body(OK_CONTENT)?;
        let (base_url, _requests) = spawn_server(vec![body])?;
        let model = DeepInfraModel::new(
            DeepInfraConfig::new("sk-secret-key", "deepseek-test").with_base_url(base_url),
        )?;

        let page = model.generate_json(&request())?;

        assert_eq!(page.title, "T");
        assert_eq!(page.summary, "S");

        Ok(())
    }

    #[test]
    fn reasoning_effort_and_response_format_are_included_only_when_set()
    -> Result<(), Box<dyn std::error::Error>> {
        let bodies = vec![
            chat_response_body(OK_CONTENT)?,
            chat_response_body(OK_CONTENT)?,
        ];
        let (base_url, requests) = spawn_server(bodies)?;

        let without_extras = DeepInfraModel::new(
            DeepInfraConfig::new("sk-secret-key", "deepseek-test").with_base_url(base_url.clone()),
        )?;
        without_extras.generate_text(&request())?;

        let with_extras = DeepInfraModel::new(
            DeepInfraConfig::new("sk-secret-key", "deepseek-test")
                .with_base_url(base_url)
                .with_reasoning_effort("medium"),
        )?;
        with_extras.generate_json(&request())?;

        let requests = requests.lock().unwrap_or_else(|error| error.into_inner());
        assert!(!requests[0].contains("response_format"));
        assert!(!requests[0].contains("reasoning_effort"));
        assert!(requests[1].contains("response_format"));
        assert!(requests[1].contains("json_object"));
        assert!(requests[1].contains("reasoning_effort"));
        assert!(requests[1].contains("medium"));

        Ok(())
    }

    #[test]
    fn errors_never_include_the_raw_api_key() -> Result<(), Box<dyn std::error::Error>> {
        let (base_url, _requests) = spawn_server(vec!["not json".to_owned()])?;
        let model = DeepInfraModel::new(
            DeepInfraConfig::new("sk-super-secret", "deepseek-test").with_base_url(base_url),
        )?;

        let Err(error) = model.generate_text(&request()) else {
            return Err("malformed response should have errored".into());
        };

        assert!(!error.message.contains("sk-super-secret"));

        Ok(())
    }
}
