//! The model boundary: a small trait separating documentation generation
//! from any specific model provider, plus a deterministic mock so tests and
//! local development never need live model credentials.
//!
//! `LanguageModel` is deliberately synchronous. Lithograph generates pages
//! for one task DAG in one CLI run, not a concurrent request-serving
//! service, so there is no concurrency need that would justify an async
//! trait (and the extra `async-trait`/`tokio` dependency weight that comes
//! with making it object-safe). Revisit if concurrent page generation
//! becomes a real throughput bottleneck.

use crate::manifest::TaskKind;
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// One bounded request to a language model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRequest {
    /// Model identifier to use, e.g. `gpt-4o-mini` or `mock`.
    pub model: String,
    /// Prompt template version, so regenerating with a changed prompt is
    /// detectable even when the underlying content hash is unchanged.
    pub prompt_version: String,
    /// Page category this request generates content for.
    pub task_kind: TaskKind,
    /// Hash over the request's context inputs.
    pub input_hash: String,
    /// System/instruction prompt.
    pub system_prompt: String,
    /// User/context prompt (evidence, excerpts, summaries).
    pub user_prompt: String,
}

/// Error returned by a [`LanguageModel`] request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelError {
    /// Human-readable failure description.
    pub message: String,
}

impl Display for ModelError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ModelError {}

/// Structured page generation output: JSON metadata plus Markdown content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageGeneration {
    /// Page title.
    pub title: String,
    /// Short summary of the page content.
    pub summary: String,
    /// Evidence references the model claims to have used, as raw strings
    /// (`artifact_path` or `artifact_path#span`); validated against the
    /// actual prompt context by the evidence validator (LIT-1.23).
    pub evidence_refs: Vec<String>,
    /// Questions the model could not resolve from the given context.
    pub unresolved_questions: Vec<String>,
    /// Markdown page body.
    pub body: String,
}

/// Model provider boundary: generate free text, or generate a validated
/// [`PageGeneration`] JSON document.
pub trait LanguageModel {
    /// Generates free-form text (e.g. a single Markdown body) for `request`.
    fn generate_text(&self, request: &ModelRequest) -> Result<String, ModelError>;
    /// Generates a structured [`PageGeneration`] document for `request`.
    fn generate_json(&self, request: &ModelRequest) -> Result<PageGeneration, ModelError>;
}

/// Deterministic model that never calls out to a real provider.
///
/// Output is a pure function of the request's fields, so the same request
/// always produces the same page, and tests never need live model
/// credentials or network access.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockModel;

impl LanguageModel for MockModel {
    fn generate_text(&self, request: &ModelRequest) -> Result<String, ModelError> {
        Ok(self.generate_json(request)?.body)
    }

    fn generate_json(&self, request: &ModelRequest) -> Result<PageGeneration, ModelError> {
        Ok(mock_page(request))
    }
}

fn mock_page(request: &ModelRequest) -> PageGeneration {
    let title = mock_title(request.task_kind);
    let context_lines = request.user_prompt.lines().count();
    let evidence_refs = first_evidence_ref(&request.user_prompt)
        .into_iter()
        .collect::<Vec<_>>();
    let evidence_section = if evidence_refs.is_empty() {
        String::new()
    } else {
        format!(
            "\n\n## Source Evidence\n{}",
            evidence_refs
                .iter()
                .map(|reference| format!("- `{reference}`"))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let body = format!(
        "# {title}\n\n\
         <!-- lithograph mock model: task={:?} model={} prompt_version={} input_hash={} -->\n\n\
         Deterministic mock content generated from {context_lines} line(s) of context.{evidence_section}\n",
        request.task_kind, request.model, request.prompt_version, request.input_hash,
    );

    PageGeneration {
        title,
        summary: format!(
            "Mock summary for {:?} task with input hash {}.",
            request.task_kind, request.input_hash
        ),
        evidence_refs,
        unresolved_questions: Vec::new(),
        body,
    }
}

fn first_evidence_ref(user_prompt: &str) -> Option<String> {
    user_prompt.lines().find_map(|line| {
        let line = line.trim().trim_start_matches("- ");
        line.strip_prefix("EVIDENCE:")
            .map(str::trim)
            .filter(|reference| !reference.is_empty())
            .map(str::to_owned)
    })
}

fn mock_title(task_kind: TaskKind) -> String {
    match task_kind {
        TaskKind::Overview => "Overview".to_owned(),
        TaskKind::Quickstart => "Quickstart".to_owned(),
        TaskKind::Architecture => "Architecture".to_owned(),
        TaskKind::Workflows => "Workflows".to_owned(),
        TaskKind::Boundaries => "Boundaries".to_owned(),
        TaskKind::Configuration => "Configuration".to_owned(),
        TaskKind::Database => "Database".to_owned(),
        TaskKind::KeyModules => "Key Modules".to_owned(),
        TaskKind::AdrDrift => "Architecture Decisions and Drift".to_owned(),
        TaskKind::ModulePage => "Module".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{LanguageModel, MockModel, ModelRequest};
    use crate::manifest::TaskKind;

    fn request(task_kind: TaskKind, input_hash: &str, user_prompt: &str) -> ModelRequest {
        ModelRequest {
            model: "mock".to_owned(),
            prompt_version: "v1".to_owned(),
            task_kind,
            input_hash: input_hash.to_owned(),
            system_prompt: "You are Lithograph.".to_owned(),
            user_prompt: user_prompt.to_owned(),
        }
    }

    #[test]
    fn mock_model_is_deterministic_for_the_same_request() -> Result<(), Box<dyn std::error::Error>>
    {
        let request = request(TaskKind::ModulePage, "abc123", "line one\nline two");

        let first = MockModel.generate_json(&request)?;
        let second = MockModel.generate_json(&request)?;

        assert_eq!(first, second);
        assert!(first.body.contains("abc123"));
        assert!(first.body.contains("v1"));
        assert_eq!(first.title, "Module");

        Ok(())
    }

    #[test]
    fn mock_model_cites_first_context_evidence() -> Result<(), Box<dyn std::error::Error>> {
        let page = MockModel.generate_json(&request(
            TaskKind::Overview,
            "hash",
            "summary\n- EVIDENCE: README.md\n- EVIDENCE: src/lib.rs",
        ))?;

        assert_eq!(page.evidence_refs, vec!["README.md".to_owned()]);
        assert!(page.body.contains("## Source Evidence"));
        assert!(page.body.contains("README.md"));

        Ok(())
    }

    #[test]
    fn mock_model_output_varies_with_request_fields() -> Result<(), Box<dyn std::error::Error>> {
        let module_request = request(TaskKind::ModulePage, "hash-a", "content");
        let quickstart_request = request(TaskKind::Quickstart, "hash-b", "content");

        let module_page = MockModel.generate_json(&module_request)?;
        let quickstart_page = MockModel.generate_json(&quickstart_request)?;

        assert_ne!(module_page.title, quickstart_page.title);
        assert!(module_page.body.contains("hash-a"));
        assert!(quickstart_page.body.contains("hash-b"));

        Ok(())
    }

    #[test]
    fn generate_text_returns_the_json_bodys_markdown() -> Result<(), Box<dyn std::error::Error>> {
        let request = request(TaskKind::Architecture, "hash-c", "context");

        let text = MockModel.generate_text(&request)?;
        let json = MockModel.generate_json(&request)?;

        assert_eq!(text, json.body);

        Ok(())
    }

    #[test]
    fn model_request_records_model_prompt_version_task_kind_and_input_hash() {
        let request = request(TaskKind::ModulePage, "hash-d", "context");

        assert_eq!(request.model, "mock");
        assert_eq!(request.prompt_version, "v1");
        assert_eq!(request.task_kind, TaskKind::ModulePage);
        assert_eq!(request.input_hash, "hash-d");
    }
}
