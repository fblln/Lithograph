use super::*;

pub(super) fn artifact_cache_key(artifact: &Artifact) -> String {
    blake3::hash(format!("{}\0{}", artifact.content_hash, artifact.path.as_str()).as_bytes())
        .to_hex()
        .to_string()
}

/// Returns the analyzer that would handle `artifact`, or `None` when no
/// analyzer applies (binary/unsafe artifacts keep only their `Artifact` node).
/// Mirrors the routing table every `process_artifact` call site used to
/// dispatch on directly.
pub(super) fn analyzer_kind(artifact: &Artifact) -> Option<AnalyzerKind> {
    let name = file_name(artifact.path.as_str());
    if is_environment_config_file(name) {
        return Some(AnalyzerKind::Environment);
    }
    match (&artifact.analyzer, artifact.detected_format.as_deref()) {
        (AnalyzerSelection::Specialized(format), _) if format == "python" => {
            Some(AnalyzerKind::Python)
        }
        (AnalyzerSelection::Specialized(format), _) if format == "rust" => Some(AnalyzerKind::Rust),
        (AnalyzerSelection::Specialized(format), _) if format == "typescript" => {
            Some(AnalyzerKind::TypeScript(TypeScriptLanguage::TypeScript))
        }
        (AnalyzerSelection::Specialized(format), _) if format == "tsx" => {
            Some(AnalyzerKind::TypeScript(TypeScriptLanguage::Tsx))
        }
        (AnalyzerSelection::Specialized(format), _) if format == "requirements-txt" => {
            Some(AnalyzerKind::Requirements)
        }
        (AnalyzerSelection::Specialized(format), _) => {
            PackageManifestFormat::from_format_id(format)
                .map(AnalyzerKind::PackageManifest)
                .or_else(|| ProtocolFormat::from_format_id(format).map(AnalyzerKind::Protocol))
        }
        (AnalyzerSelection::Structured(format), _) if format == "dockerfile" => {
            Some(AnalyzerKind::Dockerfile)
        }
        (AnalyzerSelection::Structured(format), _) if format == "markdown" => {
            Some(AnalyzerKind::Markdown)
        }
        (AnalyzerSelection::Structured(format), _) if format == "docker-compose" => {
            Some(AnalyzerKind::Compose)
        }
        (AnalyzerSelection::Structured(format), _) if format == "github-actions" => {
            Some(AnalyzerKind::Actions)
        }
        (AnalyzerSelection::Structured(format), _) if format == "toml" && name == "Cargo.toml" => {
            Some(AnalyzerKind::Cargo)
        }
        (AnalyzerSelection::Structured(format), _)
            if format == "toml" && name == "pyproject.toml" =>
        {
            Some(AnalyzerKind::PyProject)
        }
        (AnalyzerSelection::Structured(format), _)
            if matches!(format.as_str(), "yaml" | "json" | "toml") =>
        {
            Some(AnalyzerKind::Structured(structured_format(format)))
        }
        (AnalyzerSelection::SyntaxIndexed(id), _) => {
            // Registry entries can outlive a parser binding. Keep such files
            // indexable through the generic extractor rather than silently
            // dropping extraction or aborting the repository build.
            SyntaxIndexedLanguage::from_registry_id(id)
                .map(AnalyzerKind::SyntaxIndexed)
                .or(Some(AnalyzerKind::GenericText))
        }
        (AnalyzerSelection::GenericText, _) => Some(AnalyzerKind::GenericText),
        _ => None,
    }
}

fn structured_format(format: &str) -> StructuredFormat {
    match format {
        "yaml" => StructuredFormat::Yaml,
        "json" => StructuredFormat::Json,
        _ => StructuredFormat::Toml,
    }
}

fn is_environment_config_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == ".env" || lower.starts_with(".env.") || lower.ends_with(".properties")
}

/// Runs the analyzer selected by `kind` against `text`, producing the same
/// output a cache hit for this artifact's content hash would have returned.
pub(super) fn compute_fresh(
    artifact: &Artifact,
    text: &str,
    repo_root: &Path,
    kind: AnalyzerKind,
) -> AnalyzerOutput {
    match kind {
        AnalyzerKind::Python => AnalyzerOutput::Python(PythonAnalyzer.analyze(artifact, text)),
        AnalyzerKind::Rust => AnalyzerOutput::Rust(RustAnalyzer.analyze(artifact, text)),
        AnalyzerKind::TypeScript(language) => {
            let analyzer = match language {
                TypeScriptLanguage::TypeScript => TypeScriptAnalyzer::typescript(),
                TypeScriptLanguage::Tsx => TypeScriptAnalyzer::tsx(),
            };
            AnalyzerOutput::TypeScript(analyzer.analyze(artifact, text))
        }
        AnalyzerKind::Requirements => {
            AnalyzerOutput::Requirements(RequirementsAnalyzer.analyze(artifact, text))
        }
        AnalyzerKind::Dockerfile => {
            AnalyzerOutput::Dockerfile(DockerfileAnalyzer.analyze(artifact, text))
        }
        AnalyzerKind::Markdown => {
            AnalyzerOutput::Markdown(MarkdownAnalyzer.analyze(artifact, text, repo_root))
        }
        AnalyzerKind::Compose => {
            AnalyzerOutput::Compose(ComposeProfileAnalyzer.analyze(artifact, text))
        }
        AnalyzerKind::Actions => {
            AnalyzerOutput::Actions(ActionsProfileAnalyzer.analyze(artifact, text))
        }
        AnalyzerKind::Cargo => AnalyzerOutput::Cargo(CargoProfileAnalyzer.analyze(artifact, text)),
        AnalyzerKind::PyProject => {
            AnalyzerOutput::PyProject(PyProjectAnalyzer.analyze(artifact, text))
        }
        AnalyzerKind::Structured(format) => {
            AnalyzerOutput::Structured(format, StructuredAnalyzer.analyze(artifact, text, format))
        }
        AnalyzerKind::SyntaxIndexed(language) => {
            AnalyzerOutput::SyntaxIndexed(language, language.adapter().parse(text))
        }
        AnalyzerKind::PackageManifest(format) => {
            AnalyzerOutput::PackageManifest(format, format.analyze(artifact, text))
        }
        AnalyzerKind::Protocol(format) => {
            AnalyzerOutput::Protocol(format, format.analyze(artifact, text))
        }
        AnalyzerKind::GenericText => {
            AnalyzerOutput::GenericText(GenericTextExtractor.extract(artifact, text))
        }
        AnalyzerKind::Environment => {
            AnalyzerOutput::Environment(EnvironmentFacts::parse_assignments(artifact, text))
        }
        // Not reachable via `analyzer_kind()` -- `Cargo.toml` artifacts
        // already dispatch to `AnalyzerKind::Cargo` through this path, and
        // `RustWorkspaceAnalyzer` is instead run from a dedicated pre-pass in
        // `build_with_cache` (a `Cargo.toml` needs both outputs, but this
        // per-artifact dispatch only ever selects one `AnalyzerKind`). This
        // arm exists only so the shared enum match stays exhaustive and
        // behaves consistently if that ever changes.
        AnalyzerKind::RustWorkspace => {
            AnalyzerOutput::RustWorkspace(RustWorkspaceAnalyzer.analyze(artifact, repo_root))
        }
    }
}
