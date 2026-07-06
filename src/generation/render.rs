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
    /// The rendered Markdown has invalid Mermaid fence structure.
    MermaidInvalid(Vec<String>),
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
            Self::MermaidInvalid(issues) => {
                writeln!(formatter, "generated page failed Mermaid validation:")?;
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
        let repaired_generation;
        let generation = if EvidenceValidator.validate(generation, context).is_empty() {
            generation
        } else {
            repaired_generation = {
                let mut repaired = generation.clone();
                repaired.evidence_refs = EvidenceValidator.valid_references(&repaired, context);
                repaired
            };
            let remaining_issues = EvidenceValidator.validate(&repaired_generation, context);
            if !remaining_issues.is_empty() {
                return Err(RenderError::EvidenceInvalid(remaining_issues));
            }
            &repaired_generation
        };

        let rendered_body = body_with_source_evidence(generation, repo_root);
        let mermaid_issues = validate_mermaid_fences(&rendered_body);
        if !mermaid_issues.is_empty() {
            return Err(RenderError::MermaidInvalid(mermaid_issues));
        }
        let output_hash = blake3::hash(rendered_body.as_bytes()).to_hex().to_string();
        let full_path = repo_root.join(&page.path);

        let unchanged =
            std::fs::read_to_string(&full_path).is_ok_and(|existing| existing == rendered_body);
        if !unchanged {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&full_path, &rendered_body)?;
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

fn body_with_source_evidence(generation: &PageGeneration, repo_root: &Path) -> String {
    let mut body = body_without_source_evidence(&generation.body);
    if generation.evidence_refs.is_empty() {
        return body;
    }

    body = body.trim_end().to_owned();
    body.push_str("\n\n## Source Evidence\n");
    let source_base = source_base_url(repo_root);
    for reference in &generation.evidence_refs {
        if let Some(url) = source_url(source_base.as_deref(), reference) {
            body.push_str(&format!("- [`{reference}`]({url})\n"));
        } else {
            body.push_str(&format!("- `{reference}`\n"));
        }
    }
    body
}

fn body_without_source_evidence(body: &str) -> String {
    let Some((prefix, _)) = body.split_once("\n## Source Evidence") else {
        return body.to_owned();
    };
    format!("{}\n", prefix.trim_end())
}

fn validate_mermaid_fences(body: &str) -> Vec<String> {
    let mut issues = Vec::new();
    let mut in_mermaid = false;
    let mut block_start = 0usize;
    let mut saw_body_line = false;

    for (index, line) in body.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if !in_mermaid && trimmed.eq_ignore_ascii_case("```mermaid") {
            in_mermaid = true;
            block_start = line_number;
            saw_body_line = false;
            continue;
        }
        if in_mermaid && trimmed == "```" {
            if !saw_body_line {
                issues.push(format!(
                    "Mermaid block starting at line {block_start} is empty"
                ));
            }
            in_mermaid = false;
            continue;
        }
        if in_mermaid && !trimmed.is_empty() {
            saw_body_line = true;
        }
    }

    if in_mermaid {
        issues.push(format!(
            "Mermaid block starting at line {block_start} is not closed"
        ));
    }
    issues
}

fn source_base_url(repo_root: &Path) -> Option<String> {
    let output = std::process::Command::new("git")
        .args(["config", "--get", "remote.origin.url"])
        .current_dir(repo_root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let remote = String::from_utf8(output.stdout).ok()?;
    let remote = remote.trim();
    let normalized = normalize_git_remote(remote)?;
    let head = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(repo_root)
        .output()
        .ok()
        .and_then(|output| {
            output
                .status
                .success()
                .then(|| String::from_utf8(output.stdout).ok())
                .flatten()
        })
        .map(|head| head.trim().to_owned())
        .filter(|head| !head.is_empty())?;
    Some(format!("{normalized}/blob/{head}"))
}

fn normalize_git_remote(remote: &str) -> Option<String> {
    if let Some(rest) = remote.strip_prefix("git@github.com:") {
        return Some(format!(
            "https://github.com/{}",
            rest.trim_end_matches(".git")
        ));
    }
    if let Some(rest) = remote.strip_prefix("git@gitlab.com:") {
        return Some(format!(
            "https://gitlab.com/{}",
            rest.trim_end_matches(".git")
        ));
    }
    if (remote.starts_with("https://github.com/") || remote.starts_with("https://gitlab.com/"))
        && !remote.contains(' ')
    {
        return Some(remote.trim_end_matches(".git").to_owned());
    }
    None
}

fn source_url(base: Option<&str>, reference: &str) -> Option<String> {
    let base = base?;
    let (path, fragment) = reference
        .split_once('#')
        .map_or((reference, None), |(path, fragment)| (path, Some(fragment)));
    if path.is_empty()
        || path.starts_with('/')
        || path.contains("..")
        || path.contains('\\')
        || path.contains(' ')
    {
        return None;
    }
    let mut url = format!("{base}/{path}");
    if let Some(fragment) = fragment
        && fragment.starts_with('L')
    {
        url.push('#');
        url.push_str(fragment);
    }
    Some(url)
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
    use super::{PageRenderer, normalize_git_remote, source_url};
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
            prompt_version: "v1".to_owned(),
            context_schema_version: TaskKind::ModulePage.context_schema_version().to_owned(),
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
            "# Test\nbody\n\n## Source Evidence\n- `src/lib.rs#L1-L5`\n"
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
    fn drops_invalid_evidence_without_recording_page_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();

        let outcome = PageRenderer.render_and_write(
            &mut page,
            &generation("# Test\nbody\n", vec!["not/in/context.rs"]),
            &context(),
            temp.path(),
        )?;

        assert!(outcome.written);
        assert_eq!(
            std::fs::read_to_string(temp.path().join(&page.path))?,
            "# Test\nbody\n"
        );
        assert!(page.output_hash.is_some());
        assert!(page.evidence.is_empty());

        Ok(())
    }

    #[test]
    fn rewrites_source_evidence_from_validated_refs() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();

        PageRenderer.render_and_write(
            &mut page,
            &generation(
                "# Test\n\n## Source Evidence\n- `not/in/context.rs`\n",
                vec!["src/lib.rs#L1-L5", "not/in/context.rs"],
            ),
            &context(),
            temp.path(),
        )?;

        assert_eq!(
            std::fs::read_to_string(temp.path().join(&page.path))?,
            "# Test\n\n## Source Evidence\n- `src/lib.rs#L1-L5`\n"
        );
        assert_eq!(page.evidence.len(), 1);
        assert_eq!(page.evidence[0].path.as_str(), "src/lib.rs");

        Ok(())
    }

    #[test]
    fn does_not_duplicate_existing_source_evidence_section()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();

        PageRenderer.render_and_write(
            &mut page,
            &generation(
                "# Test\n\n## Source Evidence\n- `src/lib.rs`\n",
                vec!["src/lib.rs"],
            ),
            &context(),
            temp.path(),
        )?;

        let body = std::fs::read_to_string(temp.path().join(&page.path))?;
        assert_eq!(body.matches("## Source Evidence").count(), 1);

        Ok(())
    }

    #[test]
    fn rejects_unclosed_mermaid_block_without_writing() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut page = page();

        let result = PageRenderer.render_and_write(
            &mut page,
            &generation("# Test\n\n```mermaid\nflowchart TD\n", vec![]),
            &context(),
            temp.path(),
        );

        assert!(matches!(result, Err(super::RenderError::MermaidInvalid(_))));
        assert!(!temp.path().join(&page.path).exists());

        Ok(())
    }

    #[test]
    fn source_url_supports_common_git_remotes_and_line_fragments() {
        let base = Some("https://github.com/example/repo/blob/abc123");

        assert_eq!(
            source_url(base, "src/lib.rs#L1-L5").as_deref(),
            Some("https://github.com/example/repo/blob/abc123/src/lib.rs#L1-L5")
        );
        assert_eq!(
            normalize_git_remote("git@github.com:owner/repo.git").as_deref(),
            Some("https://github.com/owner/repo")
        );
        assert!(source_url(base, "../secret").is_none());
    }
}
