//! OpenAI-compatible chat completions adapter for [`LanguageModel`].
//!
//! Targets any server implementing the OpenAI chat completions API shape
//! (OpenAI itself, local compatible servers such as Ollama/LM Studio,
//! OpenRouter, and most hosted gateways), so first-release model support
//! does not need one adapter per provider.

use crate::generation::llm::{LanguageModel, ModelError, ModelRequest, PageGeneration};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Configuration for an OpenAI-compatible chat completions endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiConfig {
    /// API base URL, e.g. `https://api.openai.com/v1` or a local server URL.
    pub base_url: String,
    /// Bearer API key. Never included verbatim in error messages.
    pub api_key: String,
    /// Model name to request.
    pub model: String,
    /// Per-request timeout.
    pub timeout: Duration,
    /// Number of retries after the first attempt for retryable failures
    /// (timeouts, connection errors, HTTP 429, HTTP 5xx).
    pub max_retries: u32,
    /// Delay between retry attempts.
    pub retry_delay: Duration,
    /// Reasoning effort for reasoning models (e.g. `"low"`/`"medium"`/`"high"`).
    /// Omitted from the request when unset, which is required for
    /// non-reasoning models that reject the field outright.
    pub reasoning_effort: Option<String>,
}

impl OpenAiConfig {
    /// Builds a configuration with first-release-sensible retry/timeout defaults.
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model: model.into(),
            timeout: Duration::from_secs(60),
            max_retries: 2,
            retry_delay: Duration::from_millis(200),
            reasoning_effort: None,
        }
    }

    /// Overrides the per-request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Overrides the retry count.
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = max_retries;
        self
    }

    /// Overrides the delay between retries.
    pub fn with_retry_delay(mut self, retry_delay: Duration) -> Self {
        self.retry_delay = retry_delay;
        self
    }

    /// Sets the reasoning effort sent with each request.
    pub fn with_reasoning_effort(mut self, reasoning_effort: impl Into<String>) -> Self {
        self.reasoning_effort = Some(reasoning_effort.into());
        self
    }
}

/// OpenAI-compatible chat completions [`LanguageModel`] adapter.
pub struct OpenAiModel {
    config: OpenAiConfig,
    agent: ureq::Agent,
}

impl OpenAiModel {
    /// Builds an adapter from `config`.
    pub fn new(config: OpenAiConfig) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(config.timeout))
            .build()
            .into();
        Self { config, agent }
    }
}

impl LanguageModel for OpenAiModel {
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

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: [ChatMessage<'a>; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_effort: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct ResponseFormat<'a> {
    #[serde(rename = "type")]
    kind: &'a str,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatResponseChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatResponseChoice {
    message: ChatResponseMessage,
}

#[derive(Debug, Deserialize)]
struct ChatResponseMessage {
    content: String,
}

impl OpenAiModel {
    fn chat(&self, request: &ModelRequest, json_mode: bool) -> Result<String, ModelError> {
        let url = format!(
            "{}/chat/completions",
            self.config.base_url.trim_end_matches('/')
        );
        let body = ChatRequest {
            model: &self.config.model,
            messages: [
                ChatMessage {
                    role: "system",
                    content: &request.system_prompt,
                },
                ChatMessage {
                    role: "user",
                    content: &request.user_prompt,
                },
            ],
            response_format: json_mode.then_some(ResponseFormat {
                kind: "json_object",
            }),
            reasoning_effort: self.config.reasoning_effort.as_deref(),
        };

        let mut last_error = None;
        for attempt in 0..=self.config.max_retries {
            match self.send(&url, &body) {
                Ok(content) => return Ok(content),
                Err(error) => {
                    let retryable = error.retryable;
                    last_error = Some(error);
                    if !retryable || attempt == self.config.max_retries {
                        break;
                    }
                    std::thread::sleep(self.config.retry_delay);
                }
            }
        }

        Err(last_error
            .unwrap_or_else(|| {
                self.redact(RawError::Message(
                    "model request failed with no error detail".to_owned(),
                ))
            })
            .error)
    }

    fn send(&self, url: &str, body: &ChatRequest<'_>) -> Result<String, RetryableError> {
        let outcome = self
            .agent
            .post(url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .header("Content-Type", "application/json")
            .send_json(body);

        match outcome {
            Ok(mut response) => {
                let parsed: ChatResponse = response
                    .body_mut()
                    .read_json()
                    .map_err(|error| self.redact(RawError::Ureq(error)))?;
                parsed
                    .choices
                    .into_iter()
                    .next()
                    .map(|choice| choice.message.content)
                    .ok_or_else(|| {
                        self.redact(RawError::Message(
                            "model response had no choices".to_owned(),
                        ))
                    })
            }
            Err(error) => Err(self.redact(RawError::Ureq(error))),
        }
    }

    fn redact(&self, error: RawError) -> RetryableError {
        let retryable = error.is_retryable();
        let raw_message = error.to_string();
        let message = if self.config.api_key.is_empty() {
            raw_message
        } else {
            raw_message.replace(&self.config.api_key, "***REDACTED***")
        };
        RetryableError {
            error: ModelError {
                message: format!(
                    "OpenAI-compatible request to {} failed: {message}",
                    redacted_host(&self.config.base_url)
                ),
            },
            retryable,
        }
    }
}

struct RetryableError {
    error: ModelError,
    retryable: bool,
}

enum RawError {
    Ureq(ureq::Error),
    Message(String),
}

impl RawError {
    fn is_retryable(&self) -> bool {
        match self {
            Self::Ureq(ureq::Error::StatusCode(code)) => *code == 429 || (500..600).contains(code),
            Self::Ureq(
                ureq::Error::Timeout(_)
                | ureq::Error::Io(_)
                | ureq::Error::HostNotFound
                | ureq::Error::ConnectionFailed,
            ) => true,
            Self::Ureq(_) | Self::Message(_) => false,
        }
    }
}

impl std::fmt::Display for RawError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Ureq(error) => write!(formatter, "{error}"),
            Self::Message(message) => formatter.write_str(message),
        }
    }
}

/// Never logs the request body or full response, and never includes the
/// configured API key: only the request's host, matching AC3's redaction
/// requirement.
fn redacted_host(base_url: &str) -> String {
    base_url
        .split_once("://")
        .map(|(scheme, rest)| {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}")
        })
        .unwrap_or_else(|| "<configured endpoint>".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{OpenAiConfig, OpenAiModel};
    use crate::generation::llm::LanguageModel;
    use crate::manifest::TaskKind;
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::Duration;

    fn request() -> super::ModelRequest {
        super::ModelRequest {
            model: "gpt-test".to_owned(),
            prompt_version: "v1".to_owned(),
            task_kind: TaskKind::ModulePage,
            input_hash: "hash".to_owned(),
            system_prompt: "system".to_owned(),
            user_prompt: "user".to_owned(),
        }
    }

    /// Reads one HTTP/1.1 request off `stream` (headers + body, using
    /// Content-Length) and returns the body, so tests can assert on the
    /// exact JSON sent, while staying a minimal loopback stub rather than a
    /// full HTTP server.
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

    fn respond(
        stream: &mut TcpStream,
        status_line: &str,
        body: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let response = format!(
            "{status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(response.as_bytes())?;
        Ok(())
    }

    fn spawn_server(
        responses: Vec<(&'static str, String)>,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let (base_url, _requests) = spawn_server_capturing_requests(responses)?;
        Ok(base_url)
    }

    type CapturedRequests = std::sync::Arc<std::sync::Mutex<Vec<String>>>;

    fn spawn_server_capturing_requests(
        responses: Vec<(&'static str, String)>,
    ) -> Result<(String, CapturedRequests), Box<dyn std::error::Error>> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let addr = listener.local_addr()?;
        let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let requests_handle = requests.clone();
        std::thread::spawn(move || {
            for (status_line, body) in responses {
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
                if respond(&mut stream, status_line, &body).is_err() {
                    break;
                }
            }
        });
        Ok((format!("http://{addr}"), requests))
    }

    const OK_CONTENT: &str = "{\"title\":\"T\",\"summary\":\"S\",\"evidence_refs\":[],\"unresolved_questions\":[],\"body\":\"# T\\n\"}";

    fn chat_response_body(content: &str) -> Result<String, Box<dyn std::error::Error>> {
        Ok(format!(
            "{{\"choices\":[{{\"message\":{{\"content\":{}}}}}]}}",
            serde_json::to_string(content)?
        ))
    }

    #[test]
    fn generate_json_parses_a_successful_response() -> Result<(), Box<dyn std::error::Error>> {
        let body = chat_response_body(OK_CONTENT)?;
        let base_url = spawn_server(vec![("HTTP/1.1 200 OK", body)])?;
        let model = OpenAiModel::new(
            OpenAiConfig::new(base_url, "sk-secret-key", "gpt-test")
                .with_timeout(Duration::from_secs(5)),
        );

        let page = model.generate_json(&request())?;

        assert_eq!(page.title, "T");
        assert_eq!(page.summary, "S");

        Ok(())
    }

    #[test]
    fn reasoning_effort_is_omitted_by_default_and_included_when_set()
    -> Result<(), Box<dyn std::error::Error>> {
        let bodies = vec![
            chat_response_body(OK_CONTENT)?,
            chat_response_body(OK_CONTENT)?,
        ];
        let (base_url, requests) = spawn_server_capturing_requests(
            bodies
                .into_iter()
                .map(|body| ("HTTP/1.1 200 OK", body))
                .collect(),
        )?;

        let without_effort = OpenAiModel::new(
            OpenAiConfig::new(base_url.clone(), "sk-secret-key", "gpt-test")
                .with_timeout(Duration::from_secs(5)),
        );
        without_effort.generate_json(&request())?;

        let with_effort = OpenAiModel::new(
            OpenAiConfig::new(base_url, "sk-secret-key", "gpt-test")
                .with_timeout(Duration::from_secs(5))
                .with_reasoning_effort("medium"),
        );
        with_effort.generate_json(&request())?;

        let requests = requests.lock().unwrap_or_else(|error| error.into_inner());
        assert!(
            !requests[0].contains("reasoning_effort"),
            "unexpected body: {}",
            requests[0]
        );
        assert!(
            requests[1].contains("reasoning_effort") && requests[1].contains("medium"),
            "unexpected body: {}",
            requests[1]
        );

        Ok(())
    }

    #[test]
    fn retries_on_server_error_then_succeeds() -> Result<(), Box<dyn std::error::Error>> {
        let body = chat_response_body(OK_CONTENT)?;
        let base_url = spawn_server(vec![
            ("HTTP/1.1 500 Internal Server Error", "{}".to_owned()),
            ("HTTP/1.1 200 OK", body),
        ])?;
        let model = OpenAiModel::new(
            OpenAiConfig::new(base_url, "sk-secret-key", "gpt-test")
                .with_timeout(Duration::from_secs(5))
                .with_max_retries(1)
                .with_retry_delay(Duration::from_millis(1)),
        );

        let page = model.generate_json(&request())?;

        assert_eq!(page.title, "T");

        Ok(())
    }

    #[test]
    fn does_not_retry_client_errors_and_redacts_the_api_key()
    -> Result<(), Box<dyn std::error::Error>> {
        let base_url = spawn_server(vec![("HTTP/1.1 401 Unauthorized", "{}".to_owned())])?;
        let model = OpenAiModel::new(
            OpenAiConfig::new(base_url, "sk-super-secret-key", "gpt-test")
                .with_timeout(Duration::from_secs(5))
                .with_max_retries(3)
                .with_retry_delay(Duration::from_millis(1)),
        );

        let error = model
            .generate_text(&request())
            .err()
            .ok_or("expected an error")?;

        assert!(!error.message.contains("sk-super-secret-key"));

        Ok(())
    }

    #[test]
    fn redacted_host_never_includes_query_or_path() {
        assert_eq!(
            super::redacted_host("https://api.openai.com/v1?key=secret"),
            "https://api.openai.com"
        );
        assert_eq!(super::redacted_host("not-a-url"), "<configured endpoint>");
    }
}
