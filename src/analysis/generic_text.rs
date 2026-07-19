//! Generic safe-text extraction for unsupported artifacts.

use crate::domain::{Artifact, ModelExposurePolicy, TextStatus};
use serde::{Deserialize, Serialize};

/// Confidence assigned to an extracted text finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FindingConfidence {
    /// Finding was produced by deterministic heuristics.
    Heuristic,
}

/// Kind of generic text finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub(crate) enum TextFindingKind {
    /// Heading or top-level section-like line.
    Section,
    /// HTTP or HTTPS URL.
    Url,
    /// Repository-local path reference.
    LocalPath,
    /// Environment variable reference.
    EnvironmentVariable,
    /// Command-like line.
    Command,
    /// Package, crate, module, or image-like reference.
    PackageOrImage,
    /// Import, include, require, use, or module declaration.
    ImportOrInclude,
}

/// Generic text finding with source line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct TextFinding {
    /// Finding kind.
    pub kind: TextFindingKind,
    /// Extracted value.
    pub value: String,
    /// One-based source line.
    pub line: u32,
    /// Confidence assigned to the finding.
    pub confidence: FindingConfidence,
}

impl TextFinding {
    fn heuristic(kind: TextFindingKind, value: String, line: u32) -> Self {
        Self {
            kind,
            value,
            line,
            confidence: FindingConfidence::Heuristic,
        }
    }
}

/// Generic extractor for safe text artifacts.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GenericTextExtractor;

impl GenericTextExtractor {
    /// Extracts deterministic heuristic findings from safe model-exposable text.
    pub(crate) fn extract(&self, artifact: &Artifact, text: &str) -> Vec<TextFinding> {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return Vec::new();
        }

        // LIT-78 (same category error as LIT-73's HTML fix, LIT-23.2's CSS
        // fix): a markup document's tags, namespace declarations, and
        // presentation attributes are what the file *is*, not code
        // references to something else. The token/URL/package heuristics
        // below treat `xmlns:cc="http://..."` or `<dc:format>image/svg+xml`
        // as a URL/package reference and mint one Unresolved node apiece --
        // most visibly on `.svg` logos and `.mjml` email templates, which
        // are wholly markup. Markup contributes no generic-text code facts.
        if is_markup(text) {
            return Vec::new();
        }

        let mut findings = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            extract_line(line, line_number, &mut findings);
        }
        findings
    }
}

/// Whether `text` is a markup document (XML/SVG/HTML/MJML). The first
/// non-blank line beginning with `<` is a reliable, deterministic tell: a
/// code or config file whose analyzer fell through to generic text never
/// opens with an angle bracket, while every markup dialect does (`<?xml`,
/// `<svg`, `<!DOCTYPE`, `<mjml>`). Content-based rather than
/// extension-based so an unregistered markup extension is covered too.
fn is_markup(text: &str) -> bool {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .is_some_and(|line| line.starts_with('<'))
}

fn extract_line(line: &str, line_number: u32, findings: &mut Vec<TextFinding>) {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return;
    }

    if let Some(section) = section_value(trimmed) {
        findings.push(TextFinding::heuristic(
            TextFindingKind::Section,
            section,
            line_number,
        ));
    }
    if let Some(command) = command_value(line, trimmed) {
        findings.push(TextFinding::heuristic(
            TextFindingKind::Command,
            command,
            line_number,
        ));
    }
    if let Some(reference) = import_or_include_value(trimmed) {
        findings.push(TextFinding::heuristic(
            TextFindingKind::ImportOrInclude,
            reference.clone(),
            line_number,
        ));
        findings.push(TextFinding::heuristic(
            TextFindingKind::PackageOrImage,
            reference,
            line_number,
        ));
    }

    for token in tokens(trimmed) {
        if is_url(&token) {
            findings.push(TextFinding::heuristic(
                TextFindingKind::Url,
                token.clone(),
                line_number,
            ));
        }
        if is_local_path(&token) {
            findings.push(TextFinding::heuristic(
                TextFindingKind::LocalPath,
                token.clone(),
                line_number,
            ));
        }
        if is_env_var(&token) {
            findings.push(TextFinding::heuristic(
                TextFindingKind::EnvironmentVariable,
                normalize_env_var(token.clone()),
                line_number,
            ));
        }
        if is_package_or_image(&token) {
            findings.push(TextFinding::heuristic(
                TextFindingKind::PackageOrImage,
                token,
                line_number,
            ));
        }
    }
}

fn section_value(trimmed: &str) -> Option<String> {
    if let Some(heading) = trimmed.strip_prefix('#') {
        let heading = heading.trim_start_matches('#').trim();
        return (!heading.is_empty()).then(|| heading.to_owned());
    }
    if is_make_target(trimmed) {
        return trimmed.split(':').next().map(str::to_owned);
    }
    if let Some(name) = trimmed.strip_prefix("type ").and_then(first_identifier) {
        return Some(name.to_owned());
    }
    if let Some(name) = trimmed
        .strip_prefix("export function ")
        .and_then(first_identifier)
    {
        return Some(name.to_owned());
    }
    if is_title_like_section(trimmed) {
        return Some(trimmed.to_owned());
    }
    None
}

fn command_value(line: &str, trimmed: &str) -> Option<String> {
    let is_recipe = line.starts_with('\t');
    let first = trimmed.split_whitespace().next().unwrap_or("");
    let known = matches!(
        first,
        "cargo" | "docker" | "npm" | "pnpm" | "yarn" | "python" | "pytest" | "make" | "just"
    );
    (is_recipe || known).then(|| trimmed.to_owned())
}

fn import_or_include_value(trimmed: &str) -> Option<String> {
    if let Some(rest) = trimmed.strip_prefix("import ") {
        return quoted_value(rest).or_else(|| rest.split_whitespace().last().map(clean_token));
    }
    if let Some(rest) = trimmed.strip_prefix("from ") {
        return rest.split_whitespace().next().map(clean_token);
    }
    if let Some(rest) = trimmed.strip_prefix("use ") {
        return rest.split("::").next().map(clean_token);
    }
    if let Some(rest) = trimmed.strip_prefix("mod ") {
        return rest
            .trim_end_matches(';')
            .split_whitespace()
            .next()
            .map(clean_token);
    }
    if let Some(rest) = trimmed.strip_prefix("include ") {
        return quoted_value(rest);
    }
    if let Some(rest) = trimmed.strip_prefix("require(") {
        return quoted_value(rest);
    }
    None
}

fn tokens(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split_whitespace()
        .map(clean_token)
        .filter(|token| !token.is_empty())
}

fn clean_token(token: &str) -> String {
    token
        .trim_matches(|character: char| {
            matches!(
                character,
                '"' | '\'' | '`' | ',' | ';' | ')' | '(' | '[' | ']' | '{' | '}' | '.'
            )
        })
        .to_owned()
}

fn quoted_value(text: &str) -> Option<String> {
    let start = text.find(['"', '\'']).map_or(usize::MAX, |start| start);
    if start == usize::MAX {
        return None;
    }
    let quote = text.as_bytes()[start] as char;
    let rest = &text[start + 1..];
    let end = rest.find(quote).map_or(usize::MAX, |end| end);
    if end == usize::MAX {
        return None;
    }
    Some(rest[..end].to_owned())
}

fn first_identifier(text: &str) -> Option<&str> {
    text.split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
        .find(|part| !part.is_empty())
}

fn is_make_target(trimmed: &str) -> bool {
    let Some((target, _)) = trimmed.split_once(':') else {
        return false;
    };
    !target.is_empty()
        && !target.starts_with('.')
        && target
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
}

fn is_title_like_section(trimmed: &str) -> bool {
    let word_count = trimmed.split_whitespace().count();
    (1..=6).contains(&word_count)
        && trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character.is_whitespace())
        && trimmed
            .split_whitespace()
            .next()
            .is_some_and(|word| word.chars().next().is_some_and(char::is_uppercase))
}

fn is_url(token: &str) -> bool {
    token.starts_with("http://") || token.starts_with("https://")
}

fn is_local_path(token: &str) -> bool {
    // Documentation fallbacks see code samples as plain whitespace-delimited
    // tokens. A decorator/call such as `@app.post("/add")` contains both a
    // dot and slash, but it is not a repository path. Reject call-shaped
    // tokens before path heuristics so quoted literals are never emitted as
    // truncated pseudo-paths such as `@app.post("/add`.
    if token.contains("(\"") || token.contains("('") {
        return false;
    }
    token.contains('/')
        && !is_url(token)
        && (token.contains(".")
            || token.starts_with("./")
            || token.starts_with("../")
            || token.starts_with("src/")
            || token.starts_with("tests/"))
}

fn is_env_var(token: &str) -> bool {
    token.contains("${")
        || token.starts_with('$') && token.len() > 1
        || token.contains("process.env.")
}

fn normalize_env_var(token: String) -> String {
    if let Some((_, name)) = token.split_once("process.env.") {
        return clean_token(name);
    }
    if let Some((_, rest)) = token.split_once("${") {
        return rest.split('}').next().unwrap_or(rest).to_owned();
    }
    token
        .trim_start_matches("${")
        .trim_start_matches('$')
        .trim_end_matches('}')
        .to_owned()
}

fn is_package_or_image(token: &str) -> bool {
    token.starts_with("@")
        || token.starts_with("ghcr.io/")
        || token.starts_with("docker.io/")
        || token.starts_with("quay.io/")
        || token.contains('/') && token.contains(':') && !is_url(token)
}

#[cfg(test)]
mod tests {
    use super::{FindingConfidence, GenericTextExtractor, TextFindingKind};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;

    fn fixture_artifact(path: &str) -> Result<(Artifact, String), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let not_found = std::io::ErrorKind::NotFound;
        let artifact = artifacts
            .into_iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or(std::io::Error::new(not_found, path.to_owned()))?;
        let text = fs::read_to_string(root.join(path))?;
        Ok((artifact, text))
    }

    #[test]
    fn tsx_fixture_produces_import_and_sections() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("web/src/App.tsx")?;
        let findings = GenericTextExtractor.extract(&artifact, &text);

        assert!(has(&findings, TextFindingKind::ImportOrInclude, "react"));
        assert!(has(&findings, TextFindingKind::PackageOrImage, "react"));
        assert!(has(&findings, TextFindingKind::Section, "RouteSummary"));
        assert!(has(&findings, TextFindingKind::Section, "App"));
        assert!(
            findings
                .iter()
                .all(|finding| finding.confidence == FindingConfidence::Heuristic)
        );

        Ok(())
    }

    #[test]
    fn makefile_fixture_produces_commands_paths_and_images()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("Makefile")?;
        let findings = GenericTextExtractor.extract(&artifact, &text);

        assert!(has(&findings, TextFindingKind::Section, "python-test"));
        assert!(has(&findings, TextFindingKind::Command, "python -m pytest"));
        assert!(has(
            &findings,
            TextFindingKind::LocalPath,
            "rust/Cargo.toml"
        ));
        assert!(has(
            &findings,
            TextFindingKind::PackageOrImage,
            "ghcr.io/example/route-api:dev"
        ));

        Ok(())
    }

    #[test]
    fn license_fixture_produces_top_level_section() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("LICENSE")?;
        let findings = GenericTextExtractor.extract(&artifact, &text);

        assert!(has(&findings, TextFindingKind::Section, "MIT License"));

        Ok(())
    }

    #[test]
    fn generic_text_finds_urls_env_vars_and_includes() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = safe_text_artifact("notes.txt")?;
        let text = "\
# Setup
See https://example.test/docs and src/main.rs.
TOKEN=${API_TOKEN}
const mode = process.env.NODE_ENV;
include \"config/settings.yaml\"
";
        let findings = GenericTextExtractor.extract(&artifact, text);

        assert!(has(&findings, TextFindingKind::Section, "Setup"));
        assert!(has(
            &findings,
            TextFindingKind::Url,
            "https://example.test/docs"
        ));
        assert!(has(&findings, TextFindingKind::LocalPath, "src/main.rs"));
        assert!(has(
            &findings,
            TextFindingKind::EnvironmentVariable,
            "API_TOKEN"
        ));
        assert!(has(
            &findings,
            TextFindingKind::EnvironmentVariable,
            "NODE_ENV"
        ));
        assert!(has(
            &findings,
            TextFindingKind::ImportOrInclude,
            "config/settings.yaml"
        ));

        Ok(())
    }

    #[test]
    fn generic_text_does_not_truncate_decorator_literals_into_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = safe_text_artifact("guide.rst")?;
        let findings = GenericTextExtractor.extract(
            &artifact,
            "    @app.post(\"/add\")\n    @app.get(\"/result/<id>\")\n",
        );

        assert!(
            findings
                .iter()
                .all(|finding| finding.kind != TextFindingKind::LocalPath),
            "{findings:#?}"
        );
        Ok(())
    }

    /// LIT-78: a markup document (SVG logo, MJML email template, raw XML)
    /// contributes no generic-text code references. Its namespace URLs,
    /// `image/svg+xml`-style mime tokens, and tag fragments are markup, not
    /// references to a missing symbol, and each one otherwise minted a
    /// spurious Unresolved node.
    #[test]
    fn markup_documents_produce_no_reference_findings() -> Result<(), Box<dyn std::error::Error>> {
        let svg = safe_text_artifact("logo.svg")?;
        let svg_text = "<?xml version=\"1.0\"?>\n<svg xmlns:cc=\"http://creativecommons.org/ns#\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\">\n  <metadata><rdf:RDF><cc:Work><dc:format>image/svg+xml</dc:format></cc:Work></rdf:RDF></metadata>\n</svg>\n";
        assert!(
            GenericTextExtractor.extract(&svg, svg_text).is_empty(),
            "SVG markup must not produce generic-text findings"
        );

        let mjml = safe_text_artifact("welcome.mjml")?;
        let mjml_text = "<mjml>\n  <mj-body>\n    <mj-text>Verify your email by clicking below:</mj-text>\n  </mj-body>\n</mjml>\n";
        assert!(
            GenericTextExtractor.extract(&mjml, mjml_text).is_empty(),
            "MJML markup must not produce generic-text findings"
        );

        // A leading blank line before the markup is still recognized, and a
        // non-markup file that merely contains a `<` mid-line is unaffected.
        let leading_blank = safe_text_artifact("indented.svg")?;
        assert!(
            GenericTextExtractor
                .extract(&leading_blank, "\n\n  <svg><metadata/></svg>\n")
                .is_empty()
        );
        let code = safe_text_artifact("source.txt")?;
        assert!(
            !GenericTextExtractor
                .extract(&code, "value = a < b && c > d\nimport os\n")
                .is_empty(),
            "a code file with mid-line angle brackets is not markup"
        );
        Ok(())
    }

    #[test]
    fn generic_text_finds_import_variants_and_bare_env_vars()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = safe_text_artifact("source.txt")?;
        let text = "\
from package import thing
import lodash
use crate::module;
mod service;
require('left-pad');
$HOME
include config/settings.yaml
include \"unterminated
";
        let findings = GenericTextExtractor.extract(&artifact, text);

        assert!(has(&findings, TextFindingKind::ImportOrInclude, "package"));
        assert!(has(&findings, TextFindingKind::ImportOrInclude, "lodash"));
        assert!(has(&findings, TextFindingKind::ImportOrInclude, "crate"));
        assert!(has(&findings, TextFindingKind::ImportOrInclude, "service"));
        assert!(has(&findings, TextFindingKind::ImportOrInclude, "left-pad"));
        assert!(has(&findings, TextFindingKind::EnvironmentVariable, "HOME"));

        Ok(())
    }

    #[test]
    fn extractor_respects_model_exposure_policy() -> Result<(), Box<dyn std::error::Error>> {
        let allowed = safe_text_artifact("safe.txt")?;
        let never = safe_text_artifact("secret.txt")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let binary = safe_text_artifact("binary.bin")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::Binary, None);

        assert!(
            !GenericTextExtractor
                .extract(&allowed, "# Safe\n")
                .is_empty()
        );
        assert!(
            GenericTextExtractor
                .extract(&never, "# Secret\n")
                .is_empty()
        );
        assert!(
            GenericTextExtractor
                .extract(&binary, "# Binary\n")
                .is_empty()
        );

        Ok(())
    }

    fn safe_text_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::UnknownText,
            SupportTier::GenericText,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    fn has(findings: &[super::TextFinding], kind: TextFindingKind, value: &str) -> bool {
        findings
            .iter()
            .any(|finding| finding.kind == kind && finding.value == value)
    }
}
