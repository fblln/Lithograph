//! Deterministic section-local claims extracted from human-authored Markdown.

use serde::{Deserialize, Serialize};

/// Current persisted section-claim schema.
pub(crate) const SECTION_CLAIM_SCHEMA_VERSION: u32 = 1;
/// Current normalization and fingerprint semantics.
pub(crate) const SECTION_FINGERPRINT_VERSION: u32 = 1;

/// Observable repository fact a documentation claim can be checked against.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservableClaimKind {
    /// A shell command shown in a shell-language fence.
    Command,
    /// An HTTP route in inline code.
    Route,
    /// A container image reference with a tag.
    ContainerImage,
    /// A named service mentioned as inline code.
    Service,
    /// An environment variable mentioned as inline code.
    EnvironmentVariable,
    /// A repository path mentioned as inline code.
    RepositoryPath,
}

/// Why prose cannot be safely checked against current repository evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NonAssertableReason {
    /// Markdown syntax or another presentation-only line.
    FormattingOnly,
    /// A plan, TODO, or future-tense statement is intent rather than current fact.
    FutureIntent,
    /// Advice, preference, or qualitative judgment is not repository-observable.
    SubjectiveOrNormative,
    /// Prose contains no concrete reference supported by current analyzers.
    NoObservableReference,
    /// Inline code exists, but its meaning cannot be classified conservatively.
    AmbiguousReference,
}

/// Whether one extracted claim can be asserted against repository evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SectionClaimDisposition {
    /// The claim names a concrete observable fact.
    Assertable {
        /// Fact family used by downstream drift checks.
        kind: ObservableClaimKind,
    },
    /// The claim must not be represented as a current repository fact.
    NonAssertable {
        /// Explicit conservative classification reason.
        reason: NonAssertableReason,
    },
}

/// One normalized line-level claim within a Markdown section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SectionClaim {
    /// Stable fingerprint derived from its section and normalized text.
    pub fingerprint: String,
    /// One-based source line, retained as evidence but excluded from fingerprints.
    pub line: u32,
    /// Trimmed display text with list syntax removed.
    pub text: String,
    /// Conservative observable/non-assertable classification.
    pub disposition: SectionClaimDisposition,
}

/// Versioned claims for one human-authored Markdown section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentSectionClaims {
    /// Section-claim payload schema.
    pub schema_version: u32,
    /// Fingerprint algorithm and normalization version.
    pub fingerprint_version: u32,
    /// Repository-relative documentation path; never part of the fingerprint.
    pub artifact_path: String,
    /// Heading text, or `Preamble` for content before the first heading.
    pub heading: String,
    /// Fingerprint local to this section's normalized heading and content.
    pub section_fingerprint: String,
    /// Claims in source order.
    pub claims: Vec<SectionClaim>,
}

#[derive(Debug)]
struct PendingSection {
    heading: String,
    lines: Vec<ClaimLine>,
}

#[derive(Debug)]
struct ClaimLine {
    line: u32,
    text: String,
    fence_language: Option<String>,
    formatting_only: bool,
}

/// Extracts stable claims from one repository-relative Markdown document.
pub(crate) fn extract_section_claims(artifact_path: &str, markdown: &str) -> Vec<DocumentSectionClaims> {
    let mut sections = Vec::new();
    let mut current = PendingSection {
        heading: "Preamble".to_owned(),
        lines: Vec::new(),
    };
    let mut fence_language: Option<String> = None;
    for (index, raw_line) in markdown.lines().enumerate() {
        let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        let trimmed = raw_line.trim();
        if let Some(language) = fence_start(trimmed) {
            fence_language = Some(language);
            current.lines.push(ClaimLine {
                line: line_number,
                text: trimmed.to_owned(),
                fence_language: None,
                formatting_only: true,
            });
            continue;
        }
        if fence_language.is_some() && trimmed.starts_with("```") {
            fence_language = None;
            current.lines.push(ClaimLine {
                line: line_number,
                text: trimmed.to_owned(),
                fence_language: None,
                formatting_only: true,
            });
            continue;
        }
        if fence_language.is_none()
            && let Some(heading) = markdown_heading(trimmed)
        {
            finish_section(artifact_path, &mut sections, current);
            current = PendingSection {
                heading: heading.to_owned(),
                lines: Vec::new(),
            };
            continue;
        }
        if trimmed.is_empty() {
            continue;
        }
        current.lines.push(ClaimLine {
            line: line_number,
            text: claim_display_text(trimmed),
            fence_language: fence_language.clone(),
            formatting_only: trimmed == "---" || trimmed.starts_with("<!--"),
        });
    }
    finish_section(artifact_path, &mut sections, current);
    sections
}

/// Returns whether claims should be extracted for this repository-relative path.
pub(crate) fn is_human_authored_markdown(path: &str) -> bool {
    let normalized = path.replace('\\', "/");
    normalized.ends_with(".md") && !normalized.starts_with("docs/lithograph/")
}

fn finish_section(
    artifact_path: &str,
    sections: &mut Vec<DocumentSectionClaims>,
    section: PendingSection,
) {
    if section.lines.is_empty() {
        return;
    }
    let normalized_lines = section
        .lines
        .iter()
        .map(|line| normalize(&line.text))
        .collect::<Vec<_>>();
    let section_identity = format!(
        "v{}\n{}\n{}",
        SECTION_FINGERPRINT_VERSION,
        normalize(&section.heading),
        normalized_lines.join("\n")
    );
    let section_fingerprint = fingerprint("section", &section_identity);
    let claims = section
        .lines
        .into_iter()
        .enumerate()
        .map(|(ordinal, line)| {
            let text = line.text;
            SectionClaim {
                fingerprint: fingerprint(
                    "claim",
                    &format!("{section_fingerprint}\n{ordinal}\n{}", normalize(&text)),
                ),
                line: line.line,
                disposition: classify_claim(
                    &text,
                    line.fence_language.as_deref(),
                    line.formatting_only,
                ),
                text,
            }
        })
        .collect();
    sections.push(DocumentSectionClaims {
        schema_version: SECTION_CLAIM_SCHEMA_VERSION,
        fingerprint_version: SECTION_FINGERPRINT_VERSION,
        artifact_path: artifact_path.to_owned(),
        heading: section.heading,
        section_fingerprint,
        claims,
    });
}

fn classify_claim(
    text: &str,
    fence_language: Option<&str>,
    formatting_only: bool,
) -> SectionClaimDisposition {
    if formatting_only {
        return non_assertable(NonAssertableReason::FormattingOnly);
    }
    let lower = text.to_ascii_lowercase();
    if contains_future_intent(&lower) {
        return non_assertable(NonAssertableReason::FutureIntent);
    }
    if contains_subjective_language(&lower) {
        return non_assertable(NonAssertableReason::SubjectiveOrNormative);
    }
    if fence_language.is_some_and(is_shell_language) {
        return assertable(ObservableClaimKind::Command);
    }
    let tokens = inline_code_values(text);
    if tokens.is_empty() {
        return non_assertable(NonAssertableReason::NoObservableReference);
    }
    for token in &tokens {
        if is_route(token) {
            return assertable(ObservableClaimKind::Route);
        }
        if is_container_image(token) {
            return assertable(ObservableClaimKind::ContainerImage);
        }
        if lower.contains("service") && is_bare_identifier(token) {
            return assertable(ObservableClaimKind::Service);
        }
        if (lower.contains("environment") || lower.contains("variable")) && is_env_var(token) {
            return assertable(ObservableClaimKind::EnvironmentVariable);
        }
        if is_repository_path(token) {
            return assertable(ObservableClaimKind::RepositoryPath);
        }
    }
    non_assertable(NonAssertableReason::AmbiguousReference)
}

fn assertable(kind: ObservableClaimKind) -> SectionClaimDisposition {
    SectionClaimDisposition::Assertable { kind }
}

fn non_assertable(reason: NonAssertableReason) -> SectionClaimDisposition {
    SectionClaimDisposition::NonAssertable { reason }
}

fn fence_start(line: &str) -> Option<String> {
    let suffix = line.strip_prefix("```")?;
    if suffix.is_empty() {
        None
    } else {
        Some(suffix.split_whitespace().next()?.to_ascii_lowercase())
    }
}

fn markdown_heading(line: &str) -> Option<&str> {
    let hashes = line
        .chars()
        .take_while(|character| *character == '#')
        .count();
    if !(1..=6).contains(&hashes) || line.as_bytes().get(hashes) != Some(&b' ') {
        return None;
    }
    Some(line[hashes + 1..].trim())
}

fn claim_display_text(line: &str) -> String {
    line.trim_start_matches(['-', '*', '+'])
        .trim_start()
        .trim_start_matches(|character: char| character.is_ascii_digit())
        .trim_start_matches(['.', ')'])
        .trim_start()
        .to_owned()
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn fingerprint(prefix: &str, identity: &str) -> String {
    format!("{prefix}:{}", blake3::hash(identity.as_bytes()).to_hex())
}

fn inline_code_values(text: &str) -> Vec<&str> {
    let mut values = Vec::new();
    let mut parts = text.split('`');
    while let Some(_before) = parts.next() {
        let Some(value) = parts.next() else {
            break;
        };
        if !value.is_empty() && !value.contains('\n') {
            values.push(value);
        }
    }
    values
}

fn contains_future_intent(lower: &str) -> bool {
    ["todo", "planned", "not yet", "will eventually", "roadmap"]
        .iter()
        .any(|marker| lower.contains(marker))
}

fn contains_subjective_language(lower: &str) -> bool {
    [
        " should ",
        "best ",
        "prefer ",
        "recommended",
        "easy ",
        "simple ",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn is_shell_language(language: &str) -> bool {
    matches!(language, "sh" | "bash" | "zsh" | "shell")
}

fn is_route(value: &str) -> bool {
    let mut parts = value.split_whitespace();
    matches!(
        parts.next(),
        Some("GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "OPTIONS" | "HEAD")
    ) && parts.next().is_some_and(|path| path.starts_with('/'))
}

fn is_container_image(value: &str) -> bool {
    value.contains('/') && value.contains(':') && !value.contains(char::is_whitespace)
}

fn is_bare_identifier(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
}

fn is_env_var(value: &str) -> bool {
    value.contains('_')
        && value.chars().all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn is_repository_path(value: &str) -> bool {
    let value = value.split(['#', '?']).next().unwrap_or(value);
    value.starts_with("./")
        || value.starts_with("../")
        || value.contains('/')
        || [
            ".rs", ".py", ".ts", ".tsx", ".toml", ".yaml", ".yml", ".json", ".md",
        ]
        .iter()
        .any(|extension| value.ends_with(extension))
}

#[cfg(test)]
mod tests {
    use super::{
        NonAssertableReason, ObservableClaimKind, SectionClaimDisposition, extract_section_claims,
    };

    #[test]
    fn fingerprints_are_section_local_and_classification_is_conservative() {
        let original = "# Runtime\nThe `GET /health` route is public.\nUse `config/app.toml`.\n\n# Guidance\nPrefer simple deployments.\n";
        let changed = "# Runtime\nThe `GET /health` route is public.\nUse `config/app.toml`.\n\n# Guidance\nPrefer redundant deployments.\n";
        let first = extract_section_claims("docs/guide.md", original);
        let second = extract_section_claims("moved/guide.md", changed);

        assert_eq!(first[0].section_fingerprint, second[0].section_fingerprint);
        assert_ne!(first[1].section_fingerprint, second[1].section_fingerprint);
        assert!(matches!(
            first[0].claims[0].disposition,
            SectionClaimDisposition::Assertable {
                kind: ObservableClaimKind::Route
            }
        ));
        assert!(matches!(
            first[1].claims[0].disposition,
            SectionClaimDisposition::NonAssertable {
                reason: NonAssertableReason::SubjectiveOrNormative
            }
        ));
    }
}
