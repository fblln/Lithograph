//! Documentation drift: likely-stale Markdown facts checked against the
//! current repository and graph -- broken links, missing referenced paths,
//! documented commands that no longer match a known script/manifest entry,
//! and image/service mentions absent from the current graph.

use crate::analysis::{DriftKind as MarkdownDriftKind, MarkdownAnalyzer, MarkdownDrift};
use crate::domain::{Artifact, EvidenceRef};
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
    /// A documented command references a `make`/`npm` target or script
    /// this repository does not actually define.
    StaleCommand,
    /// An inline-code token shaped like a container image is not among the
    /// current graph's known images.
    StaleImageMention,
    /// An inline-code token used near the word "service" does not match
    /// any current graph service.
    StaleServiceMention,
}

/// One likely-stale documentation fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriftFinding {
    /// Finding category.
    pub kind: DriftKind,
    /// Markdown artifact this finding was found in.
    pub artifact_path: String,
    /// The stale text itself (link target, path, command, image, or service name).
    pub detail: String,
    /// Evidence for this finding.
    pub evidence: EvidenceRef,
}

/// Full drift scan result.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct DriftReport {
    /// All findings, in scan order.
    pub findings: Vec<DriftFinding>,
}

/// Scans Markdown documentation for likely drift against the current
/// repository and graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct DriftDetector;

impl DriftDetector {
    /// Scans every safe Markdown artifact for drift.
    pub fn scan(&self, artifacts: &[Artifact], graph: &Graph, repo_root: &Path) -> DriftReport {
        let known_make_targets = make_targets(artifacts, repo_root);
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
        for artifact in artifacts {
            if artifact.detected_format.as_deref() != Some("markdown") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
                continue;
            };
            let analysis = MarkdownAnalyzer.analyze(artifact, &text, repo_root);

            for drift in &analysis.drift {
                findings.push(from_markdown_drift(artifact.path.as_str(), drift));
            }
            for command in &analysis.commands {
                if let Some(detail) =
                    stale_command(&command.command, &known_make_targets, &known_npm_scripts)
                {
                    findings.push(DriftFinding {
                        kind: DriftKind::StaleCommand,
                        artifact_path: artifact.path.as_str().to_owned(),
                        detail,
                        evidence: command.evidence.clone(),
                    });
                }
            }
            for token in inline_code_tokens(&text) {
                if is_image_like(token.text) && !known_images.contains(token.text) {
                    findings.push(DriftFinding {
                        kind: DriftKind::StaleImageMention,
                        artifact_path: artifact.path.as_str().to_owned(),
                        detail: token.text.to_owned(),
                        evidence: EvidenceRef::file(
                            crate::domain::ArtifactId::from_path(&artifact.path),
                            artifact.path.clone(),
                        ),
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
                    });
                }
            }
        }

        DriftReport { findings }
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
    }
}

// ponytail: only `make <target>` and `npm|pnpm|yarn run <script>` are
// checked exactly, matching AC2's "when exact matching is practical" --
// commands like `cargo test` or `python -m pytest` can't be verified
// without executing them, so they are intentionally left unchecked.
fn stale_command(
    command: &str,
    make_targets: &BTreeSet<String>,
    npm_scripts: &BTreeSet<String>,
) -> Option<String> {
    let mut tokens = command.split_whitespace();
    let program = tokens.next()?;
    if program == "make" {
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
        std::fs::write(
            temp.path().join("docs/broken.md"),
            "\
# Broken docs

See [missing](./does-not-exist.md) for details.

Referenced path: `src/does_not_exist.py`.

```sh
make totally-not-a-real-target
make test
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
