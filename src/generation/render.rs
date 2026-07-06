//! Writes a validated generated page to disk, only when its content
//! actually changed, and never before evidence validation passes.

use crate::domain::{ArtifactId, EvidenceRef, RepoPath, SourceSpan};
use crate::generation::context::ModelContext;
use crate::generation::llm::PageGeneration;
use crate::generation::validate::{EvidenceIssue, EvidenceValidator};
use crate::manifest::DocumentationPage;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

/// Error returned when a page cannot be rendered or written.
#[derive(Debug)]
pub enum RenderError {
    /// The generated page cited evidence not present in its context.
    EvidenceInvalid(Vec<EvidenceIssue>),
    /// Writing the rendered file failed.
    Io(std::io::Error),
}

impl Display for RenderError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EvidenceInvalid(issues) => {
                writeln!(formatter, "generated page failed evidence validation:")?;
                for issue in issues {
                    writeln!(formatter, "  - {issue}")?;
                }
                Ok(())
            }
            Self::Io(error) => write!(formatter, "failed to write page: {error}"),
        }
    }
}

impl std::error::Error for RenderError {}

impl From<std::io::Error> for RenderError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Outcome of one render attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageWriteOutcome {
    /// True when the file was actually written (content changed).
    pub written: bool,
    /// Hash of the rendered body.
    pub output_hash: String,
    /// Absolute path written (or that would have been written).
    pub path: PathBuf,
}

/// Validates a generated page's evidence, then writes it to disk if its
/// content differs from what is already there.
#[derive(Debug, Clone, Copy, Default)]
pub struct PageRenderer;

impl PageRenderer {
    /// Validates `generation` against `context`, then updates `page`'s
    /// evidence/output hash and writes `repo_root.join(&page.path)`.
    ///
    /// Returns [`RenderError::EvidenceInvalid`] without touching `page` or
    /// the filesystem if any evidence reference fails validation.
    pub fn render_and_write(
        &self,
        page: &mut DocumentationPage,
        generation: &PageGeneration,
        context: &ModelContext,
        repo_root: &Path,
    ) -> Result<PageWriteOutcome, RenderError> {
        let issues = EvidenceValidator.validate(generation, context);
        if !issues.is_empty() {
            return Err(RenderError::EvidenceInvalid(issues));
        }

        let output_hash = blake3::hash(generation.body.as_bytes())
            .to_hex()
            .to_string();
        let full_path = repo_root.join(&page.path);

        let unchanged =
            std::fs::read_to_string(&full_path).is_ok_and(|existing| existing == generation.body);
        if !unchanged {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full_path, &generation.body)?;
        }

        page.evidence = evidence_refs(generation);
        page.output_hash = Some(output_hash.clone());

        Ok(PageWriteOutcome {
            written: !unchanged,
            output_hash,
            path: full_path,
        })
    }
}

fn evidence_refs(generation: &PageGeneration) -> Vec<EvidenceRef> {
    generation
        .evidence_refs
        .iter()
        .filter_map(|reference| evidence_ref(reference))
        .collect()
}

fn evidence_ref(reference: &str) -> Option<EvidenceRef> {
    let (path, fragment) = reference
        .split_once('#')
        .map_or((reference, None), |(path, fragment)| (path, Some(fragment)));
    let repo_path = RepoPath::new(path).ok()?;
    let artifact_id = ArtifactId::from_path(&repo_path);
    let base = EvidenceRef::file(artifact_id, repo_path);
    match fragment.and_then(parse_line_span) {
        Some((start, end)) => Some(base.with_span(SourceSpan::new(start, end).ok()?)),
        None => Some(base),
    }
}

fn parse_line_span(fragment: &str) -> Option<(u32, u32)> {
    let rest = fragment.strip_prefix('L')?;
    let (start, end) = rest.split_once("-L")?;
    Some((start.parse().ok()?, end.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::PageRenderer;
    use crate::domain::ModelExposurePolicy;
    use crate::generation::context::{ContextExcerpt, ModelContext};
    use crate::generation::llm::PageGeneration;
    use crate::manifest::{DocumentationPage, TaskKind};

    fn context() -> ModelContext {
        ModelContext {
            system_prompt: "system".to_owned(),
            user_prompt: "user".to_owned(),
            excerpts: vec![ContextExcerpt {
                artifact_path: "src/lib.rs".to_owned(),
                policy: ModelExposurePolicy::Allowed,
                included_lines: 10,
                truncated: false,
            }],
            input_hash: "hash".to_owned(),
            task_kind: TaskKind::ModulePage,
        }
    }

    fn page() -> DocumentationPage {
        DocumentationPage {
            id: "page:module:test".to_owned(),
            path: "docs/lithograph/modules/test.md".to_owned(),
            module_id: Some("module-plan:directory:test".to_owned()),
            dependencies: Vec::new(),
            evidence: Vec::new(),
            input_hash: "hash".to_owned(),
            output_hash: None,
        }
    }

    fn generation(body: &str, evidence_refs: Vec<&str>) -> PageGeneration {
        PageGeneration {
            title: "Test".to_owned(),
            summary: "Summary".to_owned(),
            evidence_refs: evidence_refs.into_iter().map(str::to_owned).collect(),
            unresolved_questions: Vec::new(),
            body: body.to_owned(),
        }
    }

    #[test]
    fn writes_new_page_and_records_evidence_and_output_hash()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();

        let outcome = PageRenderer.render_and_write(
            &mut page,
            &generation("# Test\nbody\n", vec!["src/lib.rs#L1-L5"]),
            &context(),
            temp.path(),
        )?;

        assert!(outcome.written);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(&page.path))?,
            "# Test\nbody\n"
        );
        assert_eq!(
            page.output_hash.as_deref(),
            Some(outcome.output_hash.as_str())
        );
        assert_eq!(page.evidence.len(), 1);
        assert_eq!(page.evidence[0].path.as_str(), "src/lib.rs");

        Ok(())
    }

    #[test]
    fn does_not_write_when_content_is_unchanged() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();
        let body = "# Test\nbody\n";

        let first = PageRenderer.render_and_write(
            &mut page,
            &generation(body, vec![]),
            &context(),
            temp.path(),
        )?;
        assert!(first.written);

        let written_at = std::fs::metadata(temp.path().join(&page.path))?.modified()?;
        std::thread::sleep(std::time::Duration::from_millis(10));

        let second = PageRenderer.render_and_write(
            &mut page,
            &generation(body, vec![]),
            &context(),
            temp.path(),
        )?;
        assert!(!second.written);
        assert_eq!(
            std::fs::metadata(temp.path().join(&page.path))?.modified()?,
            written_at
        );

        Ok(())
    }

    #[test]
    fn rejects_invalid_evidence_without_writing_or_updating_page_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();

        let result = PageRenderer.render_and_write(
            &mut page,
            &generation("# Test\nbody\n", vec!["not/in/context.rs"]),
            &context(),
            temp.path(),
        );

        assert!(result.is_err());
        assert!(!temp.path().join(&page.path).exists());
        assert!(page.output_hash.is_none());
        assert!(page.evidence.is_empty());

        Ok(())
    }
}
