//! Architecture drift (LIT-22.5.3): likely-stale Markdown facts checked
//! against the current repository and graph -- broken links, missing
//! referenced paths, documented commands that no longer match a known
//! script/manifest entry, image/service mentions absent from the current
//! graph, graph facts (routes) with no documentation coverage at all, and
//! documented intent (TODO/planned/not-yet-implemented) that hasn't
//! resolved into a matching graph fact.

use crate::analysis::{DriftKind as MarkdownDriftKind, MarkdownAnalyzer, MarkdownDrift};
use crate::docs::documentation_claims::{
    DocumentSectionClaims, extract_section_claims, is_human_authored_markdown,
};
use crate::domain::{Artifact, EvidenceRef, SourceSpan};
use crate::graph::{ConfigNodeKind, Graph, GraphNode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Category of a detected drift finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DriftKind {
    /// A local Markdown link does not resolve to a repository path.
    BrokenLink,
    /// A path-like reference in Markdown prose/code does not exist.
    MissingPath,
    /// A documented command references a `just`/`make`/`npm` target or script
    /// this repository does not actually define.
    StaleCommand,
    /// An inline-code token shaped like a container image is not among the
    /// current graph's known images.
    StaleImageMention,
    /// An inline-code token used near the word "service" does not match
    /// any current graph service.
    StaleServiceMention,
    /// A documented fact contradicts the current graph, rather than merely
    /// being absent from it -- e.g. an inline-code image reference names a
    /// real, current image but with a tag that no longer matches.
    ContradictsCurrentFact,
    /// A real graph fact (an HTTP/RPC/GraphQL route) has no mention in any
    /// scanned Markdown document.
    MissingDocumentation,
    /// Documentation names a planned/TODO/not-yet-implemented intent that
    /// has no matching current graph fact.
    UnresolvedIntent,
}

/// One likely-stale documentation fact, or a graph fact missing coverage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftFinding {
    /// Finding category.
    pub kind: DriftKind,
    /// Primary artifact this finding cites: the Markdown doc for
    /// doc-anchored kinds, or the defining artifact for
    /// `MissingDocumentation` (LIT-22.5.3 AC2).
    pub artifact_path: String,
    /// The stale text itself (link target, path, command, image, service
    /// name, or documented-intent excerpt).
    pub detail: String,
    /// Evidence for this finding.
    pub evidence: EvidenceRef,
    /// Graph node id cited by this finding, when applicable (LIT-22.5.3
    /// AC2: `MissingDocumentation` always sets this; doc-anchored kinds
    /// leave it `None` since they cite a Markdown artifact, not a node).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph_node: Option<String>,
}

/// Full drift scan result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DriftReport {
    /// All findings, in scan order.
    pub findings: Vec<DriftFinding>,
    /// Canonical claims from human-authored Markdown sections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_claims: Vec<DocumentSectionClaims>,
}

/// Scans Markdown documentation for likely drift against the current
/// repository and graph.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DriftDetector;

impl DriftDetector {
    /// Scans every safe Markdown artifact for drift.
    pub(crate) fn scan(
        &self,
        artifacts: &[Artifact],
        graph: &Graph,
        repo_root: &Path,
    ) -> DriftReport {
        let known_make_targets = make_targets(artifacts, repo_root);
        let known_just_targets = just_targets(artifacts, repo_root);
        let known_npm_scripts = npm_scripts(artifacts, repo_root);
        let known_images: BTreeSet<&str> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Container(image) => Some(image.reference.as_str()),
                _ => None,
            })
            .collect();
        let known_services: BTreeSet<&str> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Config(config) if config.kind == ConfigNodeKind::Service => {
                    Some(config.name.as_str())
                }
                _ => None,
            })
            .collect();

        let mut findings = Vec::new();
        let mut section_claims = Vec::new();
        let mut markdown_corpus = String::new();
        for artifact in artifacts {
            if artifact.detected_format.as_deref() != Some("markdown") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
                continue;
            };
            if is_human_authored_markdown(artifact.path.as_str()) {
                section_claims.extend(extract_section_claims(artifact.path.as_str(), &text));
            }
            let analysis = MarkdownAnalyzer.analyze(artifact, &text, repo_root);

            for drift in &analysis.drift {
                findings.push(from_markdown_drift(artifact.path.as_str(), drift));
            }
            for (line_number, line) in text.lines().enumerate() {
                if let Some(excerpt) = unresolved_intent_excerpt(line) {
                    findings.push(DriftFinding {
                        kind: DriftKind::UnresolvedIntent,
                        artifact_path: artifact.path.as_str().to_owned(),
                        detail: excerpt,
                        evidence: line_evidence(artifact, line_number as u32 + 1),
                        graph_node: None,
                    });
                }
            }
            markdown_corpus.push_str(&text);
            markdown_corpus.push('\n');
            for command in &analysis.commands {
                if let Some(detail) = stale_command(
                    &command.command,
                    &known_just_targets,
                    &known_make_targets,
                    &known_npm_scripts,
                ) {
                    findings.push(DriftFinding {
                        kind: DriftKind::StaleCommand,
                        artifact_path: artifact.path.as_str().to_owned(),
                        detail,
                        evidence: command.evidence.clone(),
                        graph_node: None,
                    });
                }
            }
            for token in inline_code_tokens(&text) {
                if is_image_like(token.text) && !known_images.contains(token.text) {
                    // An unknown reference is either genuinely stale (no
                    // current image with this name at all) or a
                    // contradiction (the image is real and current, but the
                    // documented tag no longer matches it) -- these are
                    // different failure modes and get different kinds.
                    let (kind, detail) = match matching_image_by_name(token.text, &known_images) {
                        Some(actual) => (
                            DriftKind::ContradictsCurrentFact,
                            format!(
                                "documented image `{}` does not match the current tag `{actual}`",
                                token.text
                            ),
                        ),
                        None => (DriftKind::StaleImageMention, token.text.to_owned()),
                    };
                    findings.push(DriftFinding {
                        kind,
                        artifact_path: artifact.path.as_str().to_owned(),
                        detail,
                        evidence: EvidenceRef::file(
                            crate::domain::ArtifactId::from_path(&artifact.path),
                            artifact.path.clone(),
                        ),
                        graph_node: None,
                    });
                } else if token.near_service_keyword
                    && is_service_name_like(token.text)
                    && !known_services.contains(token.text)
                {
                    findings.push(DriftFinding {
                        kind: DriftKind::StaleServiceMention,
                        artifact_path: artifact.path.as_str().to_owned(),
                        detail: token.text.to_owned(),
                        evidence: EvidenceRef::file(
                            crate::domain::ArtifactId::from_path(&artifact.path),
                            artifact.path.clone(),
                        ),
                        graph_node: None,
                    });
                }
            }
        }

        let markdown_corpus_lower = markdown_corpus.to_lowercase();
        for node in &graph.nodes {
            let GraphNode::Config(config) = node else {
                continue;
            };
            if config.kind != ConfigNodeKind::Route {
                continue;
            }
            if !markdown_corpus_lower.contains(&config.name.to_lowercase()) {
                findings.push(DriftFinding {
                    kind: DriftKind::MissingDocumentation,
                    artifact_path: config.evidence.path.as_str().to_owned(),
                    detail: config.name.clone(),
                    evidence: config.evidence.clone(),
                    graph_node: Some(node.id().as_str().to_owned()),
                });
            }
        }

        section_claims.sort_by(|left, right| {
            left.artifact_path
                .cmp(&right.artifact_path)
                .then_with(|| left.section_fingerprint.cmp(&right.section_fingerprint))
        });
        DriftReport {
            findings,
            section_claims,
        }
    }
}

/// Matches a documentation line naming a planned/TODO/not-yet-implemented
/// intent, returning a trimmed excerpt when it does. Deliberately coarse
/// (a keyword match, not NLP): a false positive here is a line a human
/// reviews and dismisses, but a missed one is a drift finding nobody sees.
fn unresolved_intent_excerpt(line: &str) -> Option<String> {
    const INTENT_KEYWORDS: &[&str] = &[
        "todo",
        "fixme",
        "not yet implemented",
        "not implemented",
        "coming soon",
        "planned",
        "will support",
        "should support",
    ];
    let lower = line.to_lowercase();
    INTENT_KEYWORDS
        .iter()
        .any(|keyword| lower.contains(keyword))
        .then(|| {
            let trimmed = line.trim();
            if trimmed.len() > 160 {
                format!("{}...", char_boundary_slice(trimmed, 0, 160))
            } else {
                trimmed.to_owned()
            }
        })
}

fn line_evidence(artifact: &Artifact, line_number: u32) -> EvidenceRef {
    let base = EvidenceRef::file(
        crate::domain::ArtifactId::from_path(&artifact.path),
        artifact.path.clone(),
    );
    match SourceSpan::new(line_number, line_number) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

fn from_markdown_drift(artifact_path: &str, drift: &MarkdownDrift) -> DriftFinding {
    let kind = match drift.kind {
        MarkdownDriftKind::BrokenLocalLink => DriftKind::BrokenLink,
        MarkdownDriftKind::MissingReferencedPath => DriftKind::MissingPath,
    };
    DriftFinding {
        kind,
        artifact_path: artifact_path.to_owned(),
        detail: drift.target.clone(),
        evidence: drift.evidence.clone(),
        graph_node: None,
    }
}

// ponytail: only `just|make <target>` and `npm|pnpm|yarn run <script>` are
// checked exactly, matching AC2's "when exact matching is practical" --
// commands like `cargo test` or `python -m pytest` can't be verified
// without executing them, so they are intentionally left unchecked.
fn stale_command(
    command: &str,
    just_targets: &BTreeSet<String>,
    make_targets: &BTreeSet<String>,
    npm_scripts: &BTreeSet<String>,
) -> Option<String> {
    let mut tokens = command.split_whitespace();
    let program = tokens.next()?;
    if program == "just" {
        let target = tokens.next()?;
        if !just_targets.contains(target) {
            return Some(format!(
                "just recipe `{target}` not found in a known justfile"
            ));
        }
    } else if program == "make" {
        let target = tokens.next()?;
        if !make_targets.contains(target) {
            return Some(format!(
                "make target `{target}` not found in a known Makefile"
            ));
        }
    } else if matches!(program, "npm" | "pnpm" | "yarn") && tokens.next() == Some("run") {
        let script = tokens.next()?;
        if !npm_scripts.contains(script) {
            return Some(format!(
                "npm script `{script}` not found in a known package.json"
            ));
        }
    }
    None
}

fn make_targets(artifacts: &[Artifact], repo_root: &Path) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for artifact in artifacts {
        if artifact.detected_format.as_deref() != Some("makefile") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
            continue;
        };
        for line in text.lines() {
            if line.starts_with(char::is_whitespace) || line.starts_with('#') {
                continue;
            }
            if let Some((name, _)) = line.split_once(':') {
                let name = name.trim();
                if !name.is_empty() && !name.contains(char::is_whitespace) {
                    targets.insert(name.to_owned());
                }
            }
        }
    }
    targets
}

/// Extracts simple, non-parameterized recipe names from repository justfiles.
/// Recipe declarations are unindented `name:` lines; assignments and recipe
/// bodies are intentionally ignored so documentation checks remain local and
/// deterministic.
fn just_targets(artifacts: &[Artifact], repo_root: &Path) -> BTreeSet<String> {
    let mut targets = BTreeSet::new();
    for artifact in artifacts {
        if artifact.detected_format.as_deref() != Some("justfile") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
            continue;
        };
        for line in text.lines() {
            if line.starts_with(char::is_whitespace)
                || line.starts_with('#')
                || line.starts_with("set ")
            {
                continue;
            }
            if let Some((name, _)) = line.split_once(':') {
                let name = name.trim();
                if !name.is_empty() && !name.contains(char::is_whitespace) && !name.ends_with('=') {
                    targets.insert(name.to_owned());
                }
            }
        }
    }
    targets
}

fn npm_scripts(artifacts: &[Artifact], repo_root: &Path) -> BTreeSet<String> {
    let mut scripts = BTreeSet::new();
    for artifact in artifacts {
        if artifact.path.as_str().rsplit('/').next() != Some("package.json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(object) = value.get("scripts").and_then(serde_json::Value::as_object) {
            scripts.extend(object.keys().cloned());
        }
    }
    scripts
}

struct InlineToken<'a> {
    text: &'a str,
    near_service_keyword: bool,
}

/// Extracts single-backtick inline-code tokens along with whether the word
/// "service" appears within a small window before/after the token.
fn inline_code_tokens(text: &str) -> Vec<InlineToken<'_>> {
    let bytes = text.as_bytes();
    let mut tokens = Vec::new();
    let mut positions = text.match_indices('`').map(|(index, _)| index);
    while let (Some(start), Some(end)) = (positions.next(), positions.next()) {
        let token = &text[start + 1..end];
        if token.is_empty() || token.contains('\n') {
            continue;
        }
        let window_start = start.saturating_sub(15);
        let window_end = (end + 15).min(bytes.len());
        let window = char_boundary_slice(text, window_start, window_end);
        tokens.push(InlineToken {
            text: token,
            near_service_keyword: window.to_ascii_lowercase().contains("service"),
        });
    }
    tokens
}

fn char_boundary_slice(text: &str, start: usize, end: usize) -> &str {
    let mut start = start;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    let mut end = end;
    while end < text.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    &text[start..end.max(start)]
}

// ponytail: a "service name" candidate is a short bare identifier (letters,
// digits, `-`/`_`); anything with '/', '.', or whitespace is a path or
// sentence fragment, not a plausible service name, which is what filters
// out false positives like `src/python_app/service.py`.
fn is_service_name_like(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '-' || character == '_'
        })
}

fn is_image_like(text: &str) -> bool {
    text.contains('/') && text.contains(':') && !text.contains(' ') && !text.starts_with("http")
}

/// The image name portion of `reference` (everything before the last `:`),
/// e.g. `"ghcr.io/example/app"` from `"ghcr.io/example/app:v1"`.
fn image_name(reference: &str) -> &str {
    reference
        .rsplit_once(':')
        .map_or(reference, |(name, _)| name)
}

/// Finds a known image sharing `reference`'s name but a different tag --
/// i.e. the same real, current image the documentation is trying to name,
/// just with a stale tag -- distinguishing a tag contradiction from a
/// genuinely unknown image (no match at all).
fn matching_image_by_name<'a>(
    reference: &str,
    known_images: &BTreeSet<&'a str>,
) -> Option<&'a str> {
    let name = image_name(reference);
    known_images
        .iter()
        .find(|image| image_name(image) == name)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::{DriftDetector, DriftKind};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    #[test]
    fn fixture_docs_have_no_drift() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let report = DriftDetector.scan(&artifacts, &graph, &root);

        assert!(
            report.findings.is_empty(),
            "unexpected drift: {:#?}",
            report.findings
        );

        Ok(())
    }

    #[test]
    fn detects_broken_link_missing_path_and_stale_command() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;
        std::fs::write(temp.path().join("justfile"), "test:\n    @true\n")?;
        std::fs::write(
            temp.path().join("docs/broken.md"),
            "\
# Broken docs

See [missing](./does-not-exist.md) for details.

Referenced path: `src/does_not_exist.py`.

```sh
make totally-not-a-real-target
make test
just totally-not-a-real-target
just test
```

Image: `ghcr.io/example/does-not-exist:latest`.

The `ghost` service handles routing.
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let report = DriftDetector.scan(&artifacts, &graph, temp.path());

        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::BrokenLink)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::MissingPath)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::StaleCommand
                    && finding.detail.contains("totally-not-a-real-target"))
        );
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::StaleCommand
                    && finding.detail.contains("`test`"))
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::StaleImageMention)
        );
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::StaleServiceMention)
        );

        Ok(())
    }

    #[test]
    fn documented_image_tag_mismatch_is_a_contradiction_not_a_stale_mention()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("docker-compose.yml"),
            "\
services:
  app:
    image: ghcr.io/example/app:v2
",
        )?;
        std::fs::write(
            temp.path().join("README.md"),
            "The app image is `ghcr.io/example/app:v1`.\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let report = DriftDetector.scan(&artifacts, &graph, temp.path());

        let contradiction = report
            .findings
            .iter()
            .find(|finding| finding.kind == DriftKind::ContradictsCurrentFact)
            .ok_or_else(|| {
                format!(
                    "expected a contradiction finding, got {:#?}",
                    report.findings
                )
            })?;
        assert_eq!(
            contradiction.detail,
            "documented image `ghcr.io/example/app:v1` does not match the current tag `ghcr.io/example/app:v2`"
        );
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::StaleImageMention),
            "a name match with a different tag is a contradiction, not a stale mention"
        );

        Ok(())
    }

    /// LIT-22.5.3 AC1/AC2/AC4: a graph fact (an HTTP route) with no
    /// mention in any Markdown doc is `MissingDocumentation`, citing the
    /// route's own graph node id and defining artifact.
    #[test]
    fn undocumented_route_is_missing_documentation() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;
        std::fs::write(temp.path().join("README.md"), "# Nothing here\n")?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let report = DriftDetector.scan(&artifacts, &graph, temp.path());

        let finding = report
            .findings
            .iter()
            .find(|finding| finding.kind == DriftKind::MissingDocumentation)
            .ok_or("missing MissingDocumentation finding")?;
        assert_eq!(finding.detail, "GET /users/{id}");
        assert_eq!(finding.artifact_path, "service.py");
        assert!(finding.graph_node.is_some());

        Ok(())
    }

    /// LIT-22.5.3 AC1/AC4: a documented route stays undetected as missing.
    #[test]
    fn documented_route_is_not_missing_documentation() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;
        std::fs::write(
            temp.path().join("README.md"),
            "The `GET /users/{id}` route returns a user.\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let report = DriftDetector.scan(&artifacts, &graph, temp.path());

        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.kind == DriftKind::MissingDocumentation),
            "unexpected drift: {:#?}",
            report.findings
        );

        Ok(())
    }

    /// LIT-22.5.3 AC1/AC2/AC4: a TODO/planned mention in a doc is
    /// `UnresolvedIntent`, citing the doc artifact and line.
    #[test]
    fn documented_todo_is_unresolved_intent() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("README.md"),
            "# Roadmap\n\nTODO: add a `/health` endpoint once the service is stable.\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let report = DriftDetector.scan(&artifacts, &graph, temp.path());

        let finding = report
            .findings
            .iter()
            .find(|finding| finding.kind == DriftKind::UnresolvedIntent)
            .ok_or("missing UnresolvedIntent finding")?;
        assert_eq!(finding.artifact_path, "README.md");
        assert!(finding.detail.contains("TODO"));
        assert!(finding.evidence.span.is_some());

        Ok(())
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut stack = vec![from.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                let destination = to.join(path.strip_prefix(from)?);
                if path.is_dir() {
                    std::fs::create_dir_all(&destination)?;
                    stack.push(path);
                } else {
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(&path, &destination)?;
                }
            }
        }
        Ok(())
    }
}
