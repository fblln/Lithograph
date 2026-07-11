//! Line-oriented Markdown analysis with source evidence.

use crate::domain::{
    Artifact, ArtifactId, EvidenceRef, ModelExposurePolicy, RepoPath, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Structured facts extracted from one Markdown artifact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct MarkdownAnalysis {
    /// First level-one heading, treated as the document title.
    pub title: Option<MarkdownHeading>,
    /// All ATX headings in source order.
    pub headings: Vec<MarkdownHeading>,
    /// Inline local and external Markdown links.
    pub links: Vec<MarkdownLink>,
    /// Fenced code blocks.
    pub code_fences: Vec<CodeFence>,
    /// Commands found in shell fences or command-like Markdown lines.
    pub commands: Vec<MarkdownCommand>,
    /// Repository source paths referenced by text, links, or code.
    pub source_paths: Vec<MarkdownPathReference>,
    /// Likely documentation drift found during extraction.
    pub drift: Vec<MarkdownDrift>,
}

/// Markdown heading with hierarchy level and evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownHeading {
    /// ATX heading level from 1 through 6.
    pub level: u8,
    /// Heading text without leading hashes.
    pub text: String,
    /// Source evidence for the heading line.
    pub evidence: EvidenceRef,
}

/// Markdown link target category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LinkKind {
    /// Link points outside the repository.
    External,
    /// Link points to a repository-local target.
    Local,
}

/// Markdown inline link or image reference.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownLink {
    /// Link label or image alt text.
    pub label: String,
    /// Raw link target from the Markdown source.
    pub target: String,
    /// Local or external target kind.
    pub kind: LinkKind,
    /// True when the Markdown syntax used image form.
    pub is_image: bool,
    /// Source evidence for the link syntax.
    pub evidence: EvidenceRef,
}

/// Fenced Markdown code block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeFence {
    /// Declared fence language, normalized to lowercase when present.
    pub language: Option<String>,
    /// Whether the fence declares Mermaid.
    pub is_mermaid: bool,
    /// Source evidence spanning the whole fence.
    pub evidence: EvidenceRef,
}

/// Command extracted from Markdown.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownCommand {
    /// Command text without shell prompt markers.
    pub command: String,
    /// Source evidence for the command line.
    pub evidence: EvidenceRef,
}

/// Repository path reference extracted from Markdown text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownPathReference {
    /// Repository-relative path-like value.
    pub path: String,
    /// Whether the path exists under the analyzed repository root.
    pub exists: bool,
    /// Source evidence for the reference.
    pub evidence: EvidenceRef,
}

/// Kind of likely Markdown documentation drift.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftKind {
    /// A local Markdown link target does not resolve.
    BrokenLocalLink,
    /// A path-like reference in prose or code does not resolve.
    MissingReferencedPath,
}

/// Likely Markdown documentation drift with evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkdownDrift {
    /// Drift category.
    pub kind: DriftKind,
    /// Missing or broken target.
    pub target: String,
    /// Source evidence for the stale reference.
    pub evidence: EvidenceRef,
}

/// Markdown analyzer for safe text documentation artifacts.
#[derive(Debug, Clone, Copy, Default)]
pub struct MarkdownAnalyzer;

#[derive(Debug, Clone)]
struct OpenFence {
    language: Option<String>,
    is_mermaid: bool,
    start_line: u32,
}

impl MarkdownAnalyzer {
    /// Recomputes `source_paths[].exists` against the current filesystem.
    /// Existence depends on the whole repository's file listing, not this
    /// Markdown artifact's own bytes, so a cached analysis (reused because
    /// this artifact's content is unchanged) must still have this refreshed
    /// after loading -- a referenced path elsewhere being added or removed
    /// must be reflected even when the reference itself didn't change.
    pub fn refresh_path_existence(
        analysis: &mut MarkdownAnalysis,
        repo_root: &Path,
        artifact_path: &RepoPath,
    ) {
        for path_ref in &mut analysis.source_paths {
            path_ref.exists = referenced_path_exists(repo_root, artifact_path, &path_ref.path);
        }
    }

    /// Extracts Markdown structure and drift signals.
    pub fn analyze(&self, artifact: &Artifact, text: &str, repo_root: &Path) -> MarkdownAnalysis {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return MarkdownAnalysis::default();
        }

        let mut analysis = MarkdownAnalysis::default();
        let mut fence = None;
        for (index, line) in text.lines().enumerate() {
            let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            if let Some(open) = fence.take() {
                if is_fence_boundary(line) {
                    add_code_fence(&mut analysis, artifact, open, line_number);
                } else {
                    extract_fenced_line(
                        &mut analysis,
                        artifact,
                        repo_root,
                        line,
                        line_number,
                        &open,
                    );
                    fence = Some(open);
                }
                continue;
            }

            if let Some(open) = open_fence(line, line_number) {
                fence = Some(open);
                continue;
            }

            extract_heading(&mut analysis, artifact, line, line_number);
            extract_links(&mut analysis, artifact, repo_root, line, line_number);
            extract_command(&mut analysis, artifact, line, line_number);
            extract_path_references(&mut analysis, artifact, repo_root, line, line_number);
        }

        if let Some(open) = fence {
            add_code_fence(&mut analysis, artifact, open.clone(), open.start_line);
        }

        analysis
    }
}

fn extract_heading(
    analysis: &mut MarkdownAnalysis,
    artifact: &Artifact,
    line: &str,
    line_number: u32,
) {
    let trimmed = line.trim_start();
    let hash_count = trimmed
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&hash_count) {
        return;
    }
    let text = trimmed[hash_count..].trim();
    if text.is_empty() {
        return;
    }
    let heading = MarkdownHeading {
        level: u8::try_from(hash_count).unwrap_or(6),
        text: text.trim_end_matches('#').trim().to_owned(),
        evidence: evidence(artifact, line_number),
    };
    if heading.level == 1 && analysis.title.is_none() {
        analysis.title = Some(heading.clone());
    }
    analysis.headings.push(heading);
}

fn extract_links(
    analysis: &mut MarkdownAnalysis,
    artifact: &Artifact,
    repo_root: &Path,
    line: &str,
    line_number: u32,
) {
    let mut remainder = line;
    while let Some(link) = next_link(remainder) {
        let evidence = evidence(artifact, line_number);
        let kind = if is_external_target(&link.target) {
            LinkKind::External
        } else {
            LinkKind::Local
        };
        if kind == LinkKind::Local {
            let exists = local_target_exists(repo_root, &artifact.path, &link.target);
            if !exists {
                analysis.drift.push(MarkdownDrift {
                    kind: DriftKind::BrokenLocalLink,
                    target: link.target.clone(),
                    evidence: evidence.clone(),
                });
            }
            add_path_reference(
                analysis,
                repo_root,
                artifact,
                &link.target,
                line_number,
                true,
            );
        }
        analysis.links.push(MarkdownLink {
            label: link.label,
            target: link.target,
            kind,
            is_image: link.is_image,
            evidence,
        });
        remainder = &remainder[link.end..];
    }
}

fn extract_fenced_line(
    analysis: &mut MarkdownAnalysis,
    artifact: &Artifact,
    repo_root: &Path,
    line: &str,
    line_number: u32,
    fence: &OpenFence,
) {
    if fence
        .language
        .as_deref()
        .is_some_and(|language| matches!(language, "sh" | "bash" | "zsh" | "shell"))
    {
        extract_command(analysis, artifact, line, line_number);
    }
    if !fence.is_mermaid {
        extract_path_references(analysis, artifact, repo_root, line, line_number);
    }
}

fn extract_command(
    analysis: &mut MarkdownAnalysis,
    artifact: &Artifact,
    line: &str,
    line_number: u32,
) {
    if let Some(command) = command_value(line) {
        analysis.commands.push(MarkdownCommand {
            command,
            evidence: evidence(artifact, line_number),
        });
    }
}

fn extract_path_references(
    analysis: &mut MarkdownAnalysis,
    artifact: &Artifact,
    repo_root: &Path,
    line: &str,
    line_number: u32,
) {
    for raw_token in line.split_whitespace() {
        // Free prose regularly joins two words with '/' to mean "or"
        // ("generated/vendor hints", a table's "entrypoint/template"), with
        // no path meaning at all. Only trust a bare "contains a slash"
        // token as a path when it is explicitly code-formatted (backticks)
        // or `./`/`../`-prefixed; otherwise require a recognized extension
        // or filename.
        let code_like = is_backtick_wrapped(raw_token);
        let token = clean_token(raw_token);
        add_path_reference(
            analysis,
            repo_root,
            artifact,
            &token,
            line_number,
            code_like,
        );
    }
}

fn is_backtick_wrapped(token: &str) -> bool {
    let trimmed = token.trim_end_matches(|character: char| {
        matches!(character, '.' | ',' | ';' | ':' | ')' | ']' | '}')
    });
    trimmed.len() > 1 && trimmed.starts_with('`') && trimmed.ends_with('`')
}

fn add_path_reference(
    analysis: &mut MarkdownAnalysis,
    repo_root: &Path,
    artifact: &Artifact,
    token: &str,
    line_number: u32,
    code_like: bool,
) {
    if !is_source_path(token, code_like) {
        return;
    }
    let path = strip_fragment(token);
    let exists = referenced_path_exists(repo_root, &artifact.path, path);
    let evidence = evidence(artifact, line_number);
    if !exists {
        analysis.drift.push(MarkdownDrift {
            kind: DriftKind::MissingReferencedPath,
            target: path.to_owned(),
            evidence: evidence.clone(),
        });
    }
    analysis.source_paths.push(MarkdownPathReference {
        path: path.to_owned(),
        exists,
        evidence,
    });
}

fn add_code_fence(
    analysis: &mut MarkdownAnalysis,
    artifact: &Artifact,
    fence: OpenFence,
    end_line: u32,
) {
    analysis.code_fences.push(CodeFence {
        language: fence.language,
        is_mermaid: fence.is_mermaid,
        evidence: evidence_span(artifact, fence.start_line, end_line),
    });
}

fn open_fence(line: &str, line_number: u32) -> Option<OpenFence> {
    let trimmed = line.trim_start();
    let language = trimmed
        .strip_prefix("```")
        .map(str::trim)
        .filter(|language| !language.is_empty())
        .map(|language| {
            language
                .split_whitespace()
                .next()
                .unwrap_or(language)
                .to_lowercase()
        })?;
    Some(OpenFence {
        is_mermaid: language == "mermaid",
        language: Some(language),
        start_line: line_number,
    })
}

fn is_fence_boundary(line: &str) -> bool {
    line.trim_start().starts_with("```")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedLink {
    label: String,
    target: String,
    is_image: bool,
    end: usize,
}

fn next_link(text: &str) -> Option<ParsedLink> {
    let open = text.find('[')?;
    let is_image = open > 0 && text.as_bytes().get(open - 1) == Some(&b'!');
    let close = text[open + 1..].find(']')? + open + 1;
    let after_label = text.get(close + 1..)?;
    if !after_label.starts_with('(') {
        return next_link(&text[close + 1..]).map(|mut link| {
            link.end += close + 1;
            link
        });
    }
    let target_end = after_label[1..].find(')')?;
    let target = after_label[1..=target_end].trim();
    Some(ParsedLink {
        label: text[open + 1..close].to_owned(),
        target: target.to_owned(),
        is_image,
        end: close + target_end + 3,
    })
}

fn command_value(line: &str) -> Option<String> {
    let trimmed = line.trim();
    if let Some(command) = trimmed.strip_prefix("$ ") {
        return Some(command.to_owned());
    }
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let known = matches!(
        first,
        "cargo" | "docker" | "just" | "make" | "npm" | "pnpm" | "python" | "pytest" | "yarn"
    );
    known.then(|| trimmed.to_owned())
}

fn clean_token(token: &str) -> String {
    // Trim a trailing sentence period first, then the rest of the
    // punctuation from both ends: trimming '.' from both ends in one pass
    // would also strip a leading dot from dotfile/dot-directory references
    // like `.github/workflows/ci.yml`.
    token
        .trim_end_matches('.')
        .trim_matches(|character: char| {
            matches!(
                character,
                '`' | '"' | '\'' | ',' | ';' | ':' | ')' | '(' | '[' | ']' | '{' | '}'
            )
        })
        .to_owned()
}

fn is_source_path(token: &str, code_like: bool) -> bool {
    if token.is_empty() || is_external_target(token) {
        return false;
    }
    known_path_extension(token)
        || token.starts_with("./")
        || token.starts_with("../")
        || (code_like && token.contains('/'))
}

fn known_path_extension(token: &str) -> bool {
    [
        ".dockerfile",
        ".html",
        ".json",
        ".md",
        ".py",
        ".rs",
        ".toml",
        ".tsx",
        ".yaml",
        ".yml",
    ]
    .iter()
    .any(|extension| strip_fragment(token).ends_with(extension))
        || strip_fragment(token) == "Dockerfile"
        || strip_fragment(token) == "Makefile"
}

fn is_external_target(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://") || target.starts_with("mailto:")
}

fn local_target_exists(repo_root: &Path, artifact_path: &RepoPath, target: &str) -> bool {
    let target = strip_fragment(target);
    if target.is_empty() || target.starts_with('#') {
        return true;
    }
    let path = if target.starts_with('/') {
        repo_root.join(target.trim_start_matches('/'))
    } else {
        repo_root.join(parent_path(artifact_path)).join(target)
    };
    path.exists()
}

fn referenced_path_exists(repo_root: &Path, artifact_path: &RepoPath, target: &str) -> bool {
    let target = strip_fragment(target);
    if target.is_empty() {
        return true;
    }
    repo_root.join(target).exists() || local_target_exists(repo_root, artifact_path, target)
}

fn parent_path(path: &RepoPath) -> PathBuf {
    Path::new(path.as_str())
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default()
}

fn strip_fragment(target: &str) -> &str {
    target
        .split('#')
        .next()
        .unwrap_or(target)
        .split('?')
        .next()
        .unwrap_or(target)
}

fn evidence(artifact: &Artifact, line_number: u32) -> EvidenceRef {
    evidence_span(artifact, line_number, line_number)
}

fn evidence_span(artifact: &Artifact, start_line: u32, end_line: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    match SourceSpan::new(start_line, end_line) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CodeFence, DriftKind, LinkKind, MarkdownAnalysis, MarkdownAnalyzer, MarkdownCommand,
        MarkdownHeading, MarkdownLink, MarkdownPathReference, evidence_span,
    };
    use crate::domain::{
        Artifact, ArtifactCategory, ArtifactId, ContentHash, EvidenceRef, ModelExposurePolicy,
        RepoPath, SupportTier, TextStatus,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;
    use tempfile::TempDir;

    #[test]
    fn markdown_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let (artifact, text) = fixture_artifact(&root, "docs/architecture.md")?;
        let analysis = MarkdownAnalyzer.analyze(&artifact, &text, &root);

        assert_eq!(
            snapshot(&analysis),
            "\
title=Architecture Notes
heading:1:Architecture Notes:1-1
link_count=0
fence:mermaid:true:10-16
fence:sh:false:20-24
command:make test:21-21
command:cargo test --manifest-path rust/Cargo.toml:22-22
command:python -m pytest:23-23
path:src/python_app/:true:5-5
path:rust/:true:6-6
path:web/:true:7-7
path:rust/Cargo.toml:true:22-22
drift_count=0"
        );

        Ok(())
    }

    #[test]
    fn markdown_extracts_links_spans_and_drift() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join("docs"))?;
        fs::create_dir_all(temp.path().join("src"))?;
        fs::write(temp.path().join("docs/guide.md"), "guide")?;
        fs::write(temp.path().join("src/lib.rs"), "pub fn ok() {}\n")?;
        let artifact = markdown_artifact("docs/readme.md")?;
        let text = "\
# Title
See [guide](guide.md), [missing](missing.md), [web](https://example.test), and ![logo](../assets/logo.svg).
Run `$ cargo test` near src/lib.rs and missing/path.rs.
```bash
$ cargo test
```
";
        let analysis = MarkdownAnalyzer.analyze(&artifact, text, temp.path());

        assert_eq!(
            analysis.title.as_ref().map(|heading| heading.text.as_str()),
            Some("Title")
        );
        assert!(has_link(
            &analysis.links,
            "guide",
            "guide.md",
            LinkKind::Local
        ));
        assert!(has_link(
            &analysis.links,
            "web",
            "https://example.test",
            LinkKind::External
        ));
        assert!(
            analysis
                .links
                .iter()
                .any(|link| link.is_image && link.target == "../assets/logo.svg")
        );
        assert!(has_path(&analysis.source_paths, "guide.md", true, 2));
        assert!(has_path(&analysis.source_paths, "missing.md", false, 2));
        assert!(has_path(&analysis.source_paths, "src/lib.rs", true, 3));
        assert!(has_path(
            &analysis.source_paths,
            "missing/path.rs",
            false,
            3
        ));
        assert!(has_command(&analysis.commands, "cargo test", 5));
        assert!(has_drift(
            &analysis,
            DriftKind::BrokenLocalLink,
            "missing.md",
            2
        ));
        assert!(has_drift(
            &analysis,
            DriftKind::MissingReferencedPath,
            "missing/path.rs",
            3
        ));

        Ok(())
    }

    #[test]
    fn markdown_respects_model_policy_and_unclosed_fences() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = TempDir::new()?;
        let allowed = markdown_artifact("README.md")?;
        let never = markdown_artifact("secret.md")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let binary = markdown_artifact("binary.md")?.with_text_status(TextStatus::Binary, None);
        let text = "\
# Title
```mermaid
graph TD
";

        let analysis = MarkdownAnalyzer.analyze(&allowed, text, temp.path());
        assert_eq!(analysis.code_fences.len(), 1);
        assert!(analysis.code_fences[0].is_mermaid);
        assert_eq!(
            analysis.code_fences[0]
                .evidence
                .span
                .as_ref()
                .map(ToString::to_string),
            Some("2-2".to_owned())
        );
        assert!(
            MarkdownAnalyzer
                .analyze(&never, text, temp.path())
                .headings
                .is_empty()
        );
        assert!(
            MarkdownAnalyzer
                .analyze(&binary, text, temp.path())
                .headings
                .is_empty()
        );
        assert!(evidence_span(&allowed, 0, 0).span.is_none());

        Ok(())
    }

    #[test]
    fn markdown_covers_parser_edge_cases() -> Result<(), Box<dyn std::error::Error>> {
        let temp = TempDir::new()?;
        fs::create_dir_all(temp.path().join("docs"))?;
        fs::write(temp.path().join("README.md"), "root")?;
        let artifact = markdown_artifact("docs/readme.md")?;
        let text = "\
#
## Secondary
Text [not a link] then [root](/README.md), [anchor](#local), and [mail](mailto:a@example.test).
```zsh
pnpm test
```
```shell
npm test
```
";
        let analysis = MarkdownAnalyzer.analyze(&artifact, text, temp.path());

        assert_eq!(analysis.title, None);
        assert!(
            analysis
                .headings
                .iter()
                .any(|heading| heading.level == 2 && heading.text == "Secondary")
        );
        assert!(has_link(
            &analysis.links,
            "root",
            "/README.md",
            LinkKind::Local
        ));
        assert!(has_link(
            &analysis.links,
            "anchor",
            "#local",
            LinkKind::Local
        ));
        assert!(has_link(
            &analysis.links,
            "mail",
            "mailto:a@example.test",
            LinkKind::External
        ));
        assert!(has_command(&analysis.commands, "pnpm test", 5));
        assert!(has_command(&analysis.commands, "npm test", 8));
        assert!(super::referenced_path_exists(
            temp.path(),
            &artifact.path,
            "#local"
        ));
        let no_span_path = RepoPath::new("docs/readme.md")?;
        let no_span = EvidenceRef::file(ArtifactId::from_path(&no_span_path), no_span_path);
        assert_eq!(span_text(&no_span), "-");

        Ok(())
    }

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn fixture_artifact(
        root: &Path,
        path: &str,
    ) -> Result<(Artifact, String), Box<dyn std::error::Error>> {
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let not_found = std::io::ErrorKind::NotFound;
        let artifact = artifacts
            .into_iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or(std::io::Error::new(not_found, path.to_owned()))?;
        let text = fs::read_to_string(root.join(path))?;
        Ok((artifact, text))
    }

    fn markdown_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::Documentation,
            SupportTier::StructuredFormat,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_detected_format("markdown")
        .with_text_status(TextStatus::Text, Some(1)))
    }

    fn snapshot(analysis: &MarkdownAnalysis) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "title={}",
            analysis
                .title
                .as_ref()
                .map(|heading| heading.text.as_str())
                .unwrap_or("-")
        ));
        lines.extend(analysis.headings.iter().map(heading_line));
        lines.push(format!("link_count={}", analysis.links.len()));
        lines.extend(analysis.code_fences.iter().map(fence_line));
        lines.extend(analysis.commands.iter().map(command_line));
        lines.extend(analysis.source_paths.iter().map(path_line));
        lines.push(format!("drift_count={}", analysis.drift.len()));
        lines.join("\n")
    }

    fn heading_line(heading: &MarkdownHeading) -> String {
        format!(
            "heading:{}:{}:{}",
            heading.level,
            heading.text,
            span_text(&heading.evidence)
        )
    }

    fn fence_line(fence: &CodeFence) -> String {
        format!(
            "fence:{}:{}:{}",
            fence.language.as_deref().unwrap_or("-"),
            fence.is_mermaid,
            span_text(&fence.evidence)
        )
    }

    fn command_line(command: &MarkdownCommand) -> String {
        format!(
            "command:{}:{}",
            command.command,
            span_text(&command.evidence)
        )
    }

    fn path_line(path: &MarkdownPathReference) -> String {
        format!(
            "path:{}:{}:{}",
            path.path,
            path.exists,
            span_text(&path.evidence)
        )
    }

    fn span_text(evidence: &crate::domain::EvidenceRef) -> String {
        evidence
            .span
            .as_ref()
            .map(ToString::to_string)
            .unwrap_or_else(|| "-".to_owned())
    }

    fn has_link(links: &[MarkdownLink], label: &str, target: &str, kind: LinkKind) -> bool {
        links
            .iter()
            .any(|link| link.label == label && link.target == target && link.kind == kind)
    }

    fn has_path(paths: &[MarkdownPathReference], path: &str, exists: bool, line: u32) -> bool {
        paths.iter().any(|reference| {
            reference.path == path
                && reference.exists == exists
                && reference
                    .evidence
                    .span
                    .as_ref()
                    .is_some_and(|span| span.start_line == line && span.end_line == line)
        })
    }

    fn has_command(commands: &[MarkdownCommand], command: &str, line: u32) -> bool {
        commands.iter().any(|found| {
            found.command == command
                && found
                    .evidence
                    .span
                    .as_ref()
                    .is_some_and(|span| span.start_line == line && span.end_line == line)
        })
    }

    fn has_drift(analysis: &MarkdownAnalysis, kind: DriftKind, target: &str, line: u32) -> bool {
        analysis.drift.iter().any(|drift| {
            drift.kind == kind
                && drift.target == target
                && drift
                    .evidence
                    .span
                    .as_ref()
                    .is_some_and(|span| span.start_line == line && span.end_line == line)
        })
    }
}
