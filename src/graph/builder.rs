//! Graph construction: merges artifact inventory and per-artifact analyzer
//! output into one typed semantic graph.

use crate::analysis::{
    ActionsProfile, ActionsProfileAnalyzer, ActionsStepHint, AnalysisCache, AnalyzerKind,
    AnalyzerOutput, CargoProfile, CargoProfileAnalyzer, ComposeProfile, ComposeProfileAnalyzer,
    ConfigReferenceKind, DockerfileAnalysis, DockerfileAnalyzer, GenericTextExtractor,
    MarkdownAnalysis, MarkdownAnalyzer, PyProjectAnalyzer, PyProjectProfile, PythonAnalysis,
    PythonAnalyzer, PythonImportKind, PythonReferenceKind, RequirementsAnalyzer,
    RequirementsProfile, RustAnalysis, RustAnalyzer, RustReferenceKind, RustWorkspaceAnalysis,
    RustWorkspaceAnalyzer, StructuredAnalysis, StructuredAnalyzer, StructuredFormat, TextFinding,
    TextFindingKind, is_python_stdlib_module, is_rust_prelude_type, python, rust_source,
    rust_std_crate,
};
use crate::domain::{
    AnalyzerSelection, Artifact, ArtifactId, Confidence, EvidenceRef, ModelExposurePolicy,
    TextStatus,
};
use crate::graph::model::{
    ArtifactNode, CommandNode, ConfigNode, ConfigNodeKind, ContainerImageNode, DocumentationNode,
    EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, PackageNode, Relation,
    RelationKind, SymbolKind, SymbolNode, UnresolvedNode,
};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

/// Builds a typed semantic graph from repository artifacts.
#[derive(Debug, Clone, Copy, Default)]
pub struct GraphBuilder;

impl GraphBuilder {
    /// Builds the graph for `artifacts` rooted at `repo_root`, reading each
    /// safe text artifact's content as needed to run its selected analyzer.
    ///
    /// Every artifact gets an `Artifact` node regardless of analyzer support,
    /// so unsupported artifacts remain visible in the graph. Equivalent to
    /// [`Self::build_with_cache`] with no cache, i.e. always parses fresh.
    pub fn build(&self, repo_root: &Path, artifacts: &[Artifact]) -> Graph {
        self.build_with_cache(repo_root, artifacts, None)
    }

    /// Builds the graph exactly like [`Self::build`], except each artifact's
    /// analyzer output is first looked up in `cache` (keyed by content hash)
    /// before falling back to reading and parsing the file. A fresh parse is
    /// written back to `cache` so a later run with the same content hash can
    /// reuse it. The resulting graph is identical either way: only whether an
    /// artifact's file is actually read and reparsed changes.
    pub fn build_with_cache(
        &self,
        repo_root: &Path,
        artifacts: &[Artifact],
        cache: Option<&AnalysisCache>,
    ) -> Graph {
        let mut state = BuilderState::new(artifacts);

        // Resolve every Cargo manifest's real crate/target layout before
        // indexing any Rust file's module, so `rust_module_path` has crate
        // roots available for every file regardless of artifact walk order.
        // `Cargo.toml` artifacts already dispatch to `AnalyzerKind::Cargo`
        // for their raw TOML profile below; this is a second, independent
        // analysis of the same artifact, cached under its own `AnalyzerKind`
        // (the cache key is `(content_hash, kind)`, not content hash alone).
        for artifact in artifacts {
            if analyzer_kind(artifact) != Some(AnalyzerKind::Cargo) {
                continue;
            }
            if artifact.text_status != TextStatus::Text
                || artifact.model_policy == ModelExposurePolicy::Never
            {
                continue;
            }
            let workspace = match cache.and_then(|cache| {
                cache.get(artifact.content_hash.as_str(), AnalyzerKind::RustWorkspace)
            }) {
                Some(AnalyzerOutput::RustWorkspace(analysis)) => analysis,
                _ => {
                    let fresh = RustWorkspaceAnalyzer.analyze(artifact, repo_root);
                    if let Some(cache) = cache {
                        cache.put(
                            artifact.content_hash.as_str(),
                            &AnalyzerOutput::RustWorkspace(fresh.clone()),
                        );
                    }
                    fresh
                }
            };
            state.register_rust_crate_roots(&workspace);
        }

        for artifact in artifacts {
            state.add_artifact_node(artifact);
            state.index_module(artifact);
        }

        for artifact in artifacts {
            if artifact.text_status != TextStatus::Text
                || artifact.model_policy == ModelExposurePolicy::Never
            {
                continue;
            }
            let Some(kind) = analyzer_kind(artifact) else {
                continue;
            };
            let mut output =
                match cache.and_then(|cache| cache.get(artifact.content_hash.as_str(), kind)) {
                    Some(cached) => cached,
                    None => {
                        let Ok(text) = fs::read_to_string(repo_root.join(artifact.path.as_str()))
                        else {
                            continue;
                        };
                        let fresh = compute_fresh(artifact, &text, repo_root, kind);
                        if let Some(cache) = cache {
                            cache.put(artifact.content_hash.as_str(), &fresh);
                        }
                        fresh
                    }
                };
            // Existence depends on the whole repo's current file listing, not
            // this artifact's own bytes, so it must be refreshed whether
            // `output` came from cache or a fresh parse.
            if let AnalyzerOutput::Markdown(analysis) = &mut output {
                MarkdownAnalyzer::refresh_path_existence(analysis, repo_root, &artifact.path);
            }
            state.apply_output(artifact, output);
        }

        state.finish()
    }
}

/// Returns the analyzer that would handle `artifact`, or `None` when no
/// analyzer applies (binary/unsafe artifacts keep only their `Artifact` node).
/// Mirrors the routing table every `process_artifact` call site used to
/// dispatch on directly.
fn analyzer_kind(artifact: &Artifact) -> Option<AnalyzerKind> {
    let name = file_name(artifact.path.as_str());
    match (&artifact.analyzer, artifact.detected_format.as_deref()) {
        (AnalyzerSelection::Specialized(format), _) if format == "python" => {
            Some(AnalyzerKind::Python)
        }
        (AnalyzerSelection::Specialized(format), _) if format == "rust" => Some(AnalyzerKind::Rust),
        (AnalyzerSelection::Specialized(format), _) if format == "requirements-txt" => {
            Some(AnalyzerKind::Requirements)
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

/// Runs the analyzer selected by `kind` against `text`, producing the same
/// output a cache hit for this artifact's content hash would have returned.
fn compute_fresh(
    artifact: &Artifact,
    text: &str,
    repo_root: &Path,
    kind: AnalyzerKind,
) -> AnalyzerOutput {
    match kind {
        AnalyzerKind::Python => AnalyzerOutput::Python(PythonAnalyzer.analyze(artifact, text)),
        AnalyzerKind::Rust => AnalyzerOutput::Rust(RustAnalyzer.analyze(artifact, text)),
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
        AnalyzerKind::GenericText => {
            AnalyzerOutput::GenericText(GenericTextExtractor.extract(artifact, text))
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

struct BuilderState {
    nodes: BTreeMap<GraphNodeId, GraphNode>,
    relations: Vec<Relation>,
    relation_count: usize,
    artifact_paths: BTreeSet<String>,
    python_modules: BTreeMap<String, GraphNodeId>,
    rust_modules: BTreeMap<String, GraphNodeId>,
    /// Repository-relative source root directory of every known Cargo
    /// build target (e.g. `"rust/src"` for a `rust/src/lib.rs` target),
    /// resolved from `cargo metadata` via [`RustWorkspaceAnalyzer`]. Used to
    /// compute a file's true crate-relative module path instead of
    /// `rust_source::module_path`'s naive whole-repo-relative guess.
    rust_crate_roots: BTreeSet<String>,
}

impl BuilderState {
    fn new(artifacts: &[Artifact]) -> Self {
        Self {
            nodes: BTreeMap::new(),
            relations: Vec::new(),
            relation_count: 0,
            artifact_paths: artifacts
                .iter()
                .map(|artifact| artifact.path.as_str().to_owned())
                .collect(),
            python_modules: BTreeMap::new(),
            rust_modules: BTreeMap::new(),
            rust_crate_roots: BTreeSet::new(),
        }
    }

    /// Records each resolved Cargo target's source root directory, so
    /// [`Self::rust_module_path`] can compute crate-relative module paths.
    /// Safe to call more than once for the same workspace (a `BTreeSet`).
    fn register_rust_crate_roots(&mut self, workspace: &RustWorkspaceAnalysis) {
        for package in &workspace.packages {
            for target in &package.targets {
                if let Some((root, _file_name)) = target.path.rsplit_once('/') {
                    self.rust_crate_roots.insert(root.to_owned());
                }
            }
        }
    }

    /// Computes a Rust file's module path relative to the deepest known
    /// Cargo target root directory that contains it, falling back to
    /// `rust_source::module_path`'s naive whole-repo-relative guess when no
    /// crate root is known (no `Cargo.toml` present, or `cargo metadata`
    /// failed to resolve it) -- see `docs/dev/parser-spike-decisions.md`.
    fn rust_module_path(&self, artifact_path: &str) -> String {
        let matched_root = self
            .rust_crate_roots
            .iter()
            .filter(|root| {
                artifact_path
                    .strip_prefix(root.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .max_by_key(|root| root.len());
        match matched_root {
            Some(root) => rust_source::module_path(&artifact_path[root.len() + 1..]),
            None => rust_source::module_path(artifact_path),
        }
    }

    fn insert(&mut self, node: GraphNode) -> GraphNodeId {
        let id = node.id().clone();
        self.nodes.entry(id.clone()).or_insert(node);
        id
    }

    fn relate(
        &mut self,
        source: GraphNodeId,
        target: GraphNodeId,
        kind: RelationKind,
        confidence: Confidence,
        evidence: Vec<EvidenceRef>,
    ) {
        self.relation_count += 1;
        self.relations.push(Relation {
            id: format!("relation:{}", self.relation_count),
            source,
            target,
            kind,
            confidence,
            evidence,
        });
    }

    fn add_artifact_node(&mut self, artifact: &Artifact) {
        self.insert(GraphNode::Artifact(ArtifactNode {
            id: artifact_node_id(artifact),
            path: artifact.path.as_str().to_owned(),
            category: artifact.category,
            evidence: file_evidence(artifact),
        }));
    }

    fn index_module(&mut self, artifact: &Artifact) {
        match artifact.detected_format.as_deref() {
            Some("python") => {
                let (module_path, _) = python::module_path(artifact.path.as_str());
                let id = self.module(
                    &module_path,
                    ModuleLanguage::Python,
                    file_evidence(artifact),
                );
                self.python_modules.insert(module_path, id);
            }
            Some("rust") => {
                let module_path = self.rust_module_path(artifact.path.as_str());
                let id = self.module(&module_path, ModuleLanguage::Rust, file_evidence(artifact));
                self.rust_modules.insert(module_path, id);
            }
            _ => {}
        }
    }

    fn module(
        &mut self,
        path: &str,
        language: ModuleLanguage,
        evidence: EvidenceRef,
    ) -> GraphNodeId {
        self.insert(GraphNode::Module(ModuleNode {
            id: GraphNodeId::new(format!("module:{path}")),
            path: path.to_owned(),
            language,
            evidence,
        }))
    }

    fn package(&mut self, name: &str, is_external: bool) -> GraphNodeId {
        self.insert(GraphNode::Package(PackageNode {
            id: GraphNodeId::new(format!("package:{name}")),
            name: name.to_owned(),
            is_external,
        }))
    }

    fn env_var(&mut self, name: &str) -> GraphNodeId {
        self.insert(GraphNode::EnvVar(EnvVarNode {
            id: GraphNodeId::new(format!("env:{name}")),
            name: name.to_owned(),
        }))
    }

    fn command(
        &mut self,
        artifact: &Artifact,
        key: &str,
        text: &str,
        evidence: EvidenceRef,
    ) -> GraphNodeId {
        self.insert(GraphNode::Command(CommandNode {
            id: GraphNodeId::new(format!("command:{}#{key}", artifact.path)),
            text: text.to_owned(),
            evidence,
        }))
    }

    fn image(&mut self, reference: &str) -> GraphNodeId {
        let is_dynamic = reference.contains("${");
        self.insert(GraphNode::Container(ContainerImageNode {
            id: GraphNodeId::new(format!("image:{reference}")),
            reference: reference.to_owned(),
            is_dynamic,
        }))
    }

    fn unresolved(&mut self, value: &str) -> GraphNodeId {
        self.insert(GraphNode::Unresolved(UnresolvedNode {
            id: GraphNodeId::new(format!("unresolved:{value}")),
            value: value.to_owned(),
        }))
    }

    /// Resolves a Python import target that didn't match a project module:
    /// a known stdlib module becomes a shared external `Package` node (one
    /// per module name, deduplicated across the whole repo) instead of a
    /// per-file `Unresolved` node.
    fn python_external_target(&mut self, dotted_name: &str) -> GraphNodeId {
        if is_python_stdlib_module(dotted_name) {
            let top_level = dotted_name.split('.').next().unwrap_or(dotted_name);
            self.package(top_level, true)
        } else {
            self.unresolved(dotted_name)
        }
    }

    fn config(
        &mut self,
        artifact: &Artifact,
        key: &str,
        kind: ConfigNodeKind,
        name: &str,
        evidence: EvidenceRef,
    ) -> GraphNodeId {
        self.insert(GraphNode::Config(ConfigNode {
            id: GraphNodeId::new(format!("config:{}#{key}", artifact.path)),
            kind,
            name: name.to_owned(),
            evidence,
        }))
    }

    /// Resolves a path-like string to a known artifact node, when it matches
    /// one exactly after stripping a `./` prefix.
    fn resolve_path(&self, value: &str) -> Option<GraphNodeId> {
        let normalized = value.trim_start_matches("./");
        self.artifact_paths
            .contains(normalized)
            .then(|| GraphNodeId::new(format!("artifact:{normalized}")))
    }

    fn reference_target(&mut self, value: &str) -> (GraphNodeId, Confidence) {
        match self.resolve_path(value) {
            Some(id) => (id, Confidence::High),
            None => (self.unresolved(value), Confidence::Low),
        }
    }

    /// Dispatches an already-computed analyzer output (fresh or cache-loaded)
    /// to the matching graph-resolution method.
    fn apply_output(&mut self, artifact: &Artifact, output: AnalyzerOutput) {
        let node = artifact_node_id(artifact);
        match output {
            AnalyzerOutput::Python(analysis) => self.process_python(artifact, analysis, &node),
            AnalyzerOutput::Rust(analysis) => self.process_rust(artifact, analysis, &node),
            AnalyzerOutput::Requirements(profile) => {
                self.process_requirements(profile, &node);
            }
            AnalyzerOutput::Dockerfile(analysis) => {
                self.process_dockerfile(artifact, analysis, &node);
            }
            AnalyzerOutput::Markdown(analysis) => self.process_markdown(artifact, analysis, &node),
            AnalyzerOutput::Compose(profile) => self.process_compose(artifact, profile, &node),
            AnalyzerOutput::Actions(profile) => self.process_actions(artifact, profile, &node),
            AnalyzerOutput::Cargo(profile) => self.process_cargo(profile, &node),
            AnalyzerOutput::PyProject(profile) => self.process_pyproject(profile, &node),
            AnalyzerOutput::Structured(_, analysis) => {
                self.process_structured(artifact, analysis, &node);
            }
            AnalyzerOutput::GenericText(findings) => {
                self.process_generic_text(artifact, &findings, &node);
            }
            AnalyzerOutput::RustWorkspace(analysis) => self.register_rust_crate_roots(&analysis),
        }
    }

    fn process_python(
        &mut self,
        artifact: &Artifact,
        analysis: PythonAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        let module_id = self
            .python_modules
            .get(&analysis.module_path)
            .cloned()
            .unwrap_or_else(|| {
                self.module(
                    &analysis.module_path,
                    ModuleLanguage::Python,
                    file_evidence(artifact),
                )
            });
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        let mut symbol_ids: BTreeMap<String, GraphNodeId> = BTreeMap::new();

        for class in &analysis.classes {
            let qualified = format!("{}::{}", analysis.module_path, class.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Class,
                qualified_name: qualified,
                doc: class.docstring.clone(),
                evidence: class.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![class.evidence.clone()],
            );
            symbol_ids.insert(class.name.clone(), id.clone());

            for method in &class.methods {
                let method_qualified =
                    format!("{}::{}::{}", analysis.module_path, class.name, method.name);
                let method_id = self.insert(GraphNode::Symbol(SymbolNode {
                    id: GraphNodeId::new(format!("symbol:{}#{method_qualified}", artifact.path)),
                    kind: SymbolKind::Method,
                    qualified_name: method_qualified,
                    doc: method.docstring.clone(),
                    evidence: method.evidence.clone(),
                }));
                self.relate(
                    id.clone(),
                    method_id,
                    RelationKind::Contains,
                    Confidence::High,
                    vec![method.evidence.clone()],
                );
            }
        }

        for function in &analysis.functions {
            let qualified = format!("{}::{}", analysis.module_path, function.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Function,
                qualified_name: qualified,
                doc: function.docstring.clone(),
                evidence: function.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
            );
            symbol_ids.insert(function.name.clone(), id);
        }

        for import in &analysis.imports {
            self.process_python_import(
                artifact,
                artifact_node,
                &analysis.module_path,
                analysis.is_package_init,
                import,
            );
        }

        for reference in &analysis.references {
            self.process_python_reference(artifact, artifact_node, reference, &symbol_ids);
        }
    }

    fn process_python_import(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        module_path: &str,
        is_package_init: bool,
        import: &crate::analysis::PythonImport,
    ) {
        match import.kind {
            PythonImportKind::Import => {
                for name in &import.names {
                    let target = self
                        .python_modules
                        .get(&name.name)
                        .cloned()
                        .unwrap_or_else(|| self.python_external_target(&name.name));
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::Imports,
                        Confidence::High,
                        vec![import.evidence.clone()],
                    );
                }
            }
            PythonImportKind::ImportFrom => {
                let resolved = python_relative_target(
                    module_path,
                    is_package_init,
                    import.relative_level,
                    import.module.as_deref(),
                );
                let target = resolved
                    .as_ref()
                    .and_then(|resolved| self.python_modules.get(resolved).cloned())
                    .unwrap_or_else(|| match (import.relative_level, &resolved) {
                        // Only an absolute (non-relative) import can ever name
                        // a stdlib module, so relative imports always fall
                        // through to the marker/unresolved path below.
                        (0, Some(module)) => self.python_external_target(module),
                        _ => {
                            let marker = resolved.clone().unwrap_or_else(|| {
                                format!(
                                    "{}{}",
                                    ".".repeat(import.relative_level as usize),
                                    import.module.clone().unwrap_or_default()
                                )
                            });
                            self.unresolved(&marker)
                        }
                    });
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::Imports,
                    Confidence::High,
                    vec![import.evidence.clone()],
                );
            }
        }
        let _ = artifact;
    }

    fn process_python_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::PythonReference,
        symbol_ids: &BTreeMap<String, GraphNodeId>,
    ) {
        match reference.kind {
            PythonReferenceKind::Call => {
                if let Some(target) = symbol_ids.get(&reference.value).cloned() {
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::Calls,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                    );
                }
            }
            PythonReferenceKind::EnvRead => {
                let target = self.env_var(&reference.value);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::ReadsEnv,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            PythonReferenceKind::Subprocess => {
                let key = format!(
                    "{}",
                    reference
                        .evidence
                        .span
                        .as_ref()
                        .map(|span| span.start_line)
                        .unwrap_or(0)
                );
                let target =
                    self.command(artifact, &key, &reference.value, reference.evidence.clone());
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::RunsCommand,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            PythonReferenceKind::DynamicImport => {
                let target = self.unresolved(&reference.value);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::Imports,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            PythonReferenceKind::Ctypes => {
                let target = self.unresolved(&reference.value);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            PythonReferenceKind::ConfigPath => {
                let (target, path_confidence) = self.reference_target(&reference.value);
                let confidence = reference.confidence.min(path_confidence);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    confidence,
                    vec![reference.evidence.clone()],
                );
            }
        }
    }

    fn process_rust(
        &mut self,
        artifact: &Artifact,
        analysis: RustAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        // `RustAnalyzer` only sees this one file's own path, so its
        // `analysis.module_path` is always the naive whole-repo-relative
        // guess; the graph builder has the cross-artifact `cargo metadata`
        // knowledge needed to correct it, via `rust_module_path`.
        let module_path = self.rust_module_path(artifact.path.as_str());
        let module_id = self
            .rust_modules
            .get(&module_path)
            .cloned()
            .unwrap_or_else(|| {
                self.module(&module_path, ModuleLanguage::Rust, file_evidence(artifact))
            });
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        let mut symbol_ids: BTreeMap<String, GraphNodeId> = BTreeMap::new();

        for item in analysis
            .structs
            .iter()
            .map(|item| (item, SymbolKind::Struct))
            .chain(analysis.enums.iter().map(|item| (item, SymbolKind::Enum)))
        {
            let (item, kind) = item;
            let qualified = format!("{module_path}::{}", item.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind,
                qualified_name: qualified,
                doc: item.doc.clone(),
                evidence: item.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![item.evidence.clone()],
            );
            symbol_ids.insert(item.name.clone(), id);
        }

        for trait_item in &analysis.traits {
            let qualified = format!("{module_path}::{}", trait_item.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Trait,
                qualified_name: qualified,
                doc: trait_item.doc.clone(),
                evidence: trait_item.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![trait_item.evidence.clone()],
            );
            symbol_ids.insert(trait_item.name.clone(), id);
        }

        for function in &analysis.functions {
            let qualified = format!("{module_path}::{}", function.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Function,
                qualified_name: qualified,
                doc: function.doc.clone(),
                evidence: function.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id,
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
            );
        }

        for imp in &analysis.impls {
            let Some(trait_name) = &imp.trait_name else {
                continue;
            };
            let source = symbol_ids
                .get(&imp.target_type)
                .cloned()
                .unwrap_or_else(|| self.rust_external_type_target(&imp.target_type));
            let target = symbol_ids
                .get(trait_name)
                .cloned()
                .unwrap_or_else(|| self.rust_external_type_target(trait_name));
            self.relate(
                source,
                target,
                RelationKind::Implements,
                Confidence::High,
                vec![imp.evidence.clone()],
            );
        }

        for use_ in &analysis.uses {
            let candidate = use_.path.strip_prefix("crate::").unwrap_or(&use_.path);
            let target = self
                .rust_modules
                .get(candidate)
                .cloned()
                .unwrap_or_else(|| match rust_std_crate(candidate) {
                    Some(crate_name) => self.package(crate_name, true),
                    None => self.unresolved(&use_.path),
                });
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::Imports,
                Confidence::High,
                vec![use_.evidence.clone()],
            );
        }

        for reference in &analysis.references {
            self.process_rust_reference(artifact, artifact_node, reference);
        }
    }

    fn process_rust_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::RustReference,
    ) {
        match reference.kind {
            RustReferenceKind::EnvRead => {
                let target = self.env_var(&reference.value);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::ReadsEnv,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            RustReferenceKind::Subprocess => {
                let key = format!(
                    "{}",
                    reference
                        .evidence
                        .span
                        .as_ref()
                        .map(|span| span.start_line)
                        .unwrap_or(0)
                );
                let target =
                    self.command(artifact, &key, &reference.value, reference.evidence.clone());
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::RunsCommand,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
        }
    }

    /// Resolves a Rust type/trait name that didn't match a same-file symbol:
    /// a well-known prelude type/trait becomes a shared external `Package`
    /// node instead of a per-file `Unresolved` node.
    fn rust_external_type_target(&mut self, name: &str) -> GraphNodeId {
        if is_rust_prelude_type(name) {
            self.package(name, true)
        } else {
            self.unresolved(name)
        }
    }

    fn process_dockerfile(
        &mut self,
        artifact: &Artifact,
        analysis: DockerfileAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        for stage in &analysis.stages {
            let target = self.image(&stage.image);
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::UsesImage,
                Confidence::High,
                vec![stage.evidence.clone()],
            );
        }
        for env in analysis.env.iter().chain(analysis.args.iter()) {
            let target = self.env_var(&env.key);
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::ReadsEnv,
                Confidence::High,
                vec![env.evidence.clone()],
            );
        }
        for command in &analysis.commands {
            let key = command.line.to_string();
            let target = self.command(artifact, &key, &command.command, command.evidence.clone());
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::RunsCommand,
                Confidence::High,
                vec![command.evidence.clone()],
            );
        }
    }

    fn process_markdown(
        &mut self,
        artifact: &Artifact,
        analysis: MarkdownAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        for heading in &analysis.headings {
            let key = heading
                .evidence
                .span
                .as_ref()
                .map(|span| span.start_line)
                .unwrap_or(0);
            let id = self.insert(GraphNode::Documentation(DocumentationNode {
                id: GraphNodeId::new(format!("doc:{}#{key}", artifact.path)),
                title: heading.text.clone(),
                evidence: heading.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id,
                RelationKind::Contains,
                Confidence::High,
                vec![heading.evidence.clone()],
            );
        }
        for link in analysis
            .links
            .iter()
            .filter(|link| matches!(link.kind, crate::analysis::LinkKind::Local))
        {
            let (target, confidence) = self.reference_target(&link.target);
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::References,
                confidence,
                vec![link.evidence.clone()],
            );
        }
        for path_ref in &analysis.source_paths {
            let target = if path_ref.exists {
                self.resolve_path(&path_ref.path)
                    .unwrap_or_else(|| self.unresolved(&path_ref.path))
            } else {
                self.unresolved(&path_ref.path)
            };
            let confidence = if path_ref.exists {
                Confidence::High
            } else {
                Confidence::Low
            };
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::References,
                confidence,
                vec![path_ref.evidence.clone()],
            );
        }
        for command in &analysis.commands {
            let key = command
                .evidence
                .span
                .as_ref()
                .map(|span| span.start_line)
                .unwrap_or(0);
            let target = self.command(
                artifact,
                &key.to_string(),
                &command.command,
                command.evidence.clone(),
            );
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::RunsCommand,
                Confidence::High,
                vec![command.evidence.clone()],
            );
        }
    }

    fn process_structured(
        &mut self,
        artifact: &Artifact,
        analysis: StructuredAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        for reference in &analysis.references {
            match reference.kind {
                ConfigReferenceKind::Path | ConfigReferenceKind::Url => {
                    let (target, confidence) = self.reference_target(&reference.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        confidence,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Port => {
                    let target = self.config(
                        artifact,
                        &reference.config_path,
                        ConfigNodeKind::Port,
                        &reference.value,
                        reference.evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::Contains,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Image => {
                    let target = self.image(&reference.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::UsesImage,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Service => {
                    let target = self.config(
                        artifact,
                        &reference.config_path,
                        ConfigNodeKind::Service,
                        &reference.value,
                        reference.evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::Contains,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Command => {
                    let target = self.command(
                        artifact,
                        &reference.config_path,
                        &reference.value,
                        reference.evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::EnvironmentVariable => {
                    let target = self.env_var(&reference.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
            }
        }
    }

    fn process_cargo(&mut self, profile: CargoProfile, artifact_node: &GraphNodeId) {
        let Some(package) = &profile.package else {
            return;
        };
        let Some(name) = &package.name else { return };
        let package_id = self.package(name, false);
        self.relate(
            artifact_node.clone(),
            package_id.clone(),
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![package.evidence.clone()],
        );
        for dependency in &profile.dependencies {
            let dependency_id = self.package(&dependency.name, true);
            self.relate(
                package_id.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence.clone()],
            );
        }
    }

    fn process_pyproject(&mut self, profile: PyProjectProfile, artifact_node: &GraphNodeId) {
        let Some(project) = &profile.project else {
            return;
        };
        let Some(name) = &project.name else { return };
        let package_id = self.package(name, false);
        self.relate(
            artifact_node.clone(),
            package_id.clone(),
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![project.evidence.clone()],
        );
        for dependency in &project.dependencies {
            let dependency_name = python_dependency_name(&dependency.requirement);
            let dependency_id = self.package(dependency_name, true);
            self.relate(
                package_id.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence.clone()],
            );
        }
    }

    fn process_requirements(&mut self, profile: RequirementsProfile, artifact_node: &GraphNodeId) {
        for requirement in &profile.requirements {
            let dependency_id = self.package(&requirement.name, true);
            self.relate(
                artifact_node.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![requirement.evidence.clone()],
            );
        }
    }

    fn process_compose(
        &mut self,
        artifact: &Artifact,
        profile: ComposeProfile,
        artifact_node: &GraphNodeId,
    ) {
        for service in &profile.services {
            let key = format!("services.{}", service.name);
            let service_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Service,
                &service.name,
                service.evidence.clone(),
            );
            self.relate(
                artifact_node.clone(),
                service_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![service.evidence.clone()],
            );
            if let Some(image) = &service.image {
                let target = self.image(image);
                self.relate(
                    service_id.clone(),
                    target,
                    RelationKind::UsesImage,
                    Confidence::High,
                    vec![service.evidence.clone()],
                );
            }
            for env in &service.environment {
                let target = self.env_var(&env.key);
                self.relate(
                    service_id.clone(),
                    target,
                    RelationKind::ReadsEnv,
                    Confidence::High,
                    vec![env.evidence.clone()],
                );
            }
            for depends_on in &service.depends_on {
                let dependency_key = format!("services.{depends_on}");
                let target = if profile
                    .services
                    .iter()
                    .any(|other| &other.name == depends_on)
                {
                    GraphNodeId::new(format!("config:{}#{dependency_key}", artifact.path))
                } else {
                    self.unresolved(depends_on)
                };
                self.relate(
                    service_id.clone(),
                    target,
                    RelationKind::References,
                    Confidence::High,
                    vec![service.evidence.clone()],
                );
            }
        }
    }

    fn process_actions(
        &mut self,
        artifact: &Artifact,
        profile: ActionsProfile,
        artifact_node: &GraphNodeId,
    ) {
        for job in &profile.jobs {
            let key = format!("jobs.{}", job.id);
            let job_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Job,
                &job.id,
                job.evidence.clone(),
            );
            self.relate(
                artifact_node.clone(),
                job_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![job.evidence.clone()],
            );
            for (index, step) in job.steps.iter().enumerate() {
                for env in &step.env {
                    let target = self.env_var(&env.key);
                    self.relate(
                        job_id.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::High,
                        vec![env.evidence.clone()],
                    );
                }
                if let Some(run) = &step.run {
                    let step_key = format!("{key}.steps[{index}]");
                    let target = self.command(artifact, &step_key, run, step.evidence.clone());
                    self.relate(
                        job_id.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::High,
                        vec![step.evidence.clone()],
                    );
                }
                match &step.hint {
                    Some(ActionsStepHint::Build { image }) => {
                        let (target, confidence) = self.hint_image_target(image);
                        self.relate(
                            job_id.clone(),
                            target,
                            RelationKind::BuildsImage,
                            confidence,
                            vec![step.evidence.clone()],
                        );
                    }
                    Some(ActionsStepHint::Publish { image }) => {
                        let (target, confidence) = self.hint_image_target(image);
                        self.relate(
                            job_id.clone(),
                            target,
                            RelationKind::PublishesImage,
                            confidence,
                            vec![step.evidence.clone()],
                        );
                    }
                    None => {}
                }
            }
        }
    }

    fn hint_image_target(&mut self, image: &Option<String>) -> (GraphNodeId, Confidence) {
        match image {
            Some(image) => (self.image(image), Confidence::High),
            None => (self.unresolved("dynamic-image"), Confidence::Low),
        }
    }

    fn process_generic_text(
        &mut self,
        artifact: &Artifact,
        findings: &[TextFinding],
        artifact_node: &GraphNodeId,
    ) {
        for finding in findings {
            let evidence = generic_finding_evidence(artifact, finding.line);
            match finding.kind {
                TextFindingKind::EnvironmentVariable => {
                    let target = self.env_var(&finding.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::Low,
                        vec![evidence],
                    );
                }
                TextFindingKind::Command => {
                    let target = self.command(
                        artifact,
                        &finding.line.to_string(),
                        &finding.value,
                        evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::Low,
                        vec![evidence],
                    );
                }
                TextFindingKind::LocalPath => {
                    let target = self
                        .resolve_path(&finding.value)
                        .unwrap_or_else(|| self.unresolved(&finding.value));
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        Confidence::Low,
                        vec![evidence],
                    );
                }
                TextFindingKind::Url
                | TextFindingKind::PackageOrImage
                | TextFindingKind::ImportOrInclude => {
                    let target = self.unresolved(&finding.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        Confidence::Low,
                        vec![evidence],
                    );
                }
                TextFindingKind::Section => {}
            }
        }
    }

    fn finish(mut self) -> Graph {
        let mut nodes: Vec<GraphNode> = self.nodes.into_values().collect();
        nodes.sort_by(|a, b| a.id().cmp(b.id()));
        self.relations
            .sort_by(|a, b| (&a.source, a.kind, &a.target).cmp(&(&b.source, b.kind, &b.target)));
        Graph {
            nodes,
            relations: self.relations,
        }
    }
}

fn python_relative_target(
    module_path: &str,
    is_package_init: bool,
    level: u32,
    module: Option<&str>,
) -> Option<String> {
    if level == 0 {
        return module.map(str::to_owned);
    }
    let mut segments: Vec<&str> = module_path
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect();
    if !is_package_init && !segments.is_empty() {
        segments.pop();
    }
    for _ in 1..level {
        segments.pop();
    }
    let mut target = segments.join(".");
    if let Some(module) = module {
        if !target.is_empty() {
            target.push('.');
        }
        target.push_str(module);
    }
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

fn python_dependency_name(requirement: &str) -> &str {
    let end = requirement
        .find(|character: char| {
            !(character.is_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '.')
        })
        .unwrap_or(requirement.len());
    &requirement[..end]
}

fn artifact_node_id(artifact: &Artifact) -> GraphNodeId {
    GraphNodeId::new(format!("artifact:{}", artifact.path))
}

fn file_evidence(artifact: &Artifact) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone())
}

fn generic_finding_evidence(artifact: &Artifact, line: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    match crate::domain::SourceSpan::new(line, line) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::GraphBuilder;
    use crate::analysis::AnalysisCache;
    use crate::graph::{GraphNode, RelationKind};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for entry in walk_files(from)? {
            let relative = entry.strip_prefix(from)?;
            let destination = to.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry, &destination)?;
        }
        Ok(())
    }

    fn walk_files(root: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        Ok(files)
    }

    #[test]
    fn build_with_cache_none_matches_build() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;

        let via_build = GraphBuilder.build(&root, &artifacts);
        let via_build_with_cache = GraphBuilder.build_with_cache(&root, &artifacts, None);

        assert_eq!(via_build, via_build_with_cache);

        Ok(())
    }

    #[test]
    fn mixed_cache_hit_and_miss_matches_a_fully_fresh_rebuild()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), repo.path())?;
        let cache_dir = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(cache_dir.path());

        let before_artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        // Populates one cache entry per analyzable artifact.
        GraphBuilder.build_with_cache(repo.path(), &before_artifacts, Some(&cache));
        let misses_after_populating = cache.misses();

        let lib_rs = repo.path().join("rust/src/lib.rs");
        let mut source = std::fs::read_to_string(&lib_rs)?;
        source.push_str("\n// a deliberate one-file change\n");
        std::fs::write(&lib_rs, source)?;

        let after_artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let mixed = GraphBuilder.build_with_cache(repo.path(), &after_artifacts, Some(&cache));
        // Every artifact except the mutated one should have been served from
        // cache: exactly one miss since the cache was populated.
        assert_eq!(cache.misses() - misses_after_populating, 1);

        let fresh = GraphBuilder.build(repo.path(), &after_artifacts);

        assert_eq!(mixed, fresh);
        assert_eq!(mixed.to_json()?, fresh.to_json()?);

        Ok(())
    }

    #[test]
    fn cache_hit_still_refreshes_markdown_path_existence() -> Result<(), Box<dyn std::error::Error>>
    {
        let repo = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), repo.path())?;
        // Plain prose (no Markdown link syntax) so this only populates
        // `source_paths`, not `links` -- `links` resolves live against the
        // current artifact set regardless of caching, so a link here
        // wouldn't exercise the `source_paths[].exists` staleness this test
        // is checking for.
        std::fs::write(
            repo.path().join("README.md"),
            "# Fixture\n\nSee not-yet-created.md for details.\n",
        )?;
        let cache_dir = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(cache_dir.path());

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        // Populates the cache with README.md's analysis while the reference
        // is still dangling.
        GraphBuilder.build_with_cache(repo.path(), &artifacts, Some(&cache));

        std::fs::write(repo.path().join("not-yet-created.md"), "# Now it exists\n")?;
        // README.md's own bytes are unchanged, so this rebuild must hit the
        // cache for it -- proving existence is still refreshed on a hit.
        let rebuilt_artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build_with_cache(repo.path(), &rebuilt_artifacts, Some(&cache));

        let resolved = graph.relations.iter().any(|relation| {
            relation.source.as_str() == "artifact:README.md"
                && relation.target.as_str() == "artifact:not-yet-created.md"
        });
        assert!(
            resolved,
            "expected the newly-created target to resolve from a cache-hit analysis"
        );

        Ok(())
    }

    #[test]
    fn graph_covers_every_node_kind_and_relation_has_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let mut seen_artifact = false;
        let mut seen_symbol = false;
        let mut seen_config = false;
        let mut seen_documentation = false;
        let mut seen_container = false;
        let mut seen_command = false;
        let mut seen_env_var = false;
        let mut seen_module = false;
        let mut seen_package = false;
        let mut seen_unresolved = false;
        for node in &graph.nodes {
            match node {
                GraphNode::Artifact(_) => seen_artifact = true,
                GraphNode::Symbol(_) => seen_symbol = true,
                GraphNode::Config(_) => seen_config = true,
                GraphNode::Documentation(_) => seen_documentation = true,
                GraphNode::Container(_) => seen_container = true,
                GraphNode::Command(_) => seen_command = true,
                GraphNode::EnvVar(_) => seen_env_var = true,
                GraphNode::Module(_) => seen_module = true,
                GraphNode::Package(_) => seen_package = true,
                GraphNode::Unresolved(_) => seen_unresolved = true,
            }
        }
        assert!(seen_artifact && seen_symbol && seen_config && seen_documentation);
        assert!(seen_container && seen_command && seen_env_var && seen_module);
        assert!(seen_package && seen_unresolved);

        assert!(!graph.relations.is_empty());
        assert!(
            graph
                .relations
                .iter()
                .all(|relation| !relation.evidence.is_empty())
        );
        let ids: std::collections::BTreeSet<_> = graph.nodes.iter().map(|node| node.id()).collect();
        for relation in &graph.relations {
            assert!(
                ids.contains(&relation.source),
                "dangling source {relation:?}"
            );
            assert!(
                ids.contains(&relation.target),
                "dangling target {relation:?}"
            );
        }

        Ok(())
    }

    #[test]
    fn graph_export_is_deterministic_json() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let first = GraphBuilder.build(&root, &artifacts).to_json()?;
        let second = GraphBuilder.build(&root, &artifacts).to_json()?;

        assert_eq!(first, second);
        assert!(first.contains("\"node_type\": \"Artifact\""));
        let round_tripped: crate::graph::Graph = serde_json::from_str(&first)?;
        assert_eq!(GraphBuilder.build(&root, &artifacts), round_tripped);

        Ok(())
    }

    #[test]
    fn graph_keeps_every_artifact_node_including_unsupported()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        for artifact in &artifacts {
            let id = format!("artifact:{}", artifact.path);
            assert!(
                graph.nodes.iter().any(|node| node.id().as_str() == id),
                "missing artifact node for {}",
                artifact.path
            );
        }
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == "artifact:data/sample.bin")
        );

        Ok(())
    }

    #[test]
    fn graph_resolves_python_relative_import_and_same_file_call()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let service_module = "module:src.python_app.service";
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == service_module)
        );

        let resolved_relative_import = graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Imports
                && relation.source.as_str() == "artifact:src/python_app/__init__.py"
                && relation.target.as_str() == service_module
        });
        assert!(
            resolved_relative_import,
            "expected resolved relative import to service module"
        );

        let same_file_call = graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Calls
                && relation.source.as_str() == "artifact:src/python_app/service.py"
        });
        assert!(
            same_file_call,
            "expected a resolved same-file call relation"
        );

        Ok(())
    }

    #[test]
    fn graph_links_dockerfile_and_compose_to_image_nodes() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::UsesImage
                    && relation.source.as_str() == "artifact:Dockerfile")
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::UsesImage
                    && relation.target.as_str() == "image:node:24-alpine")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == "package:fixture-worker")
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::DependsOnPackage
                    && relation.target.as_str() == "package:anyhow")
        );

        Ok(())
    }

    #[test]
    fn rust_module_paths_are_corrected_by_cargo_metadata_crate_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        // Before RustWorkspaceAnalyzer was wired in, `rust_source::module_path`
        // treated the whole repo-relative path as the module path, so
        // `rust/src/lib.rs` (the crate root, per `cargo_metadata`) wrongly
        // became module `rust::src` and `rust/src/bin/worker.rs` (its own
        // binary target root) wrongly became `rust::src::bin::worker`.
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let has_node = |id: &str| graph.nodes.iter().any(|node| node.id().as_str() == id);

        assert!(
            has_node("module:"),
            "expected the lib target's crate root to map to the empty module path"
        );
        assert!(
            has_node("module:worker"),
            "expected the bin target's own root to map to just its target name"
        );
        assert!(
            !has_node("module:rust::src"),
            "the old naive whole-path module id must no longer appear"
        );
        assert!(
            !has_node("module:rust::src::bin::worker"),
            "the old naive whole-path module id must no longer appear"
        );

        let lib_belongs_to_root = graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::BelongsToModule
                && relation.source.as_str() == "artifact:rust/src/lib.rs"
                && relation.target.as_str() == "module:"
        });
        assert!(lib_belongs_to_root);

        Ok(())
    }

    #[test]
    fn stdlib_and_prelude_references_become_external_packages_not_unresolved()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("app.py"),
            "\
import os
import requests
from __future__ import annotations

def run():
    os.getenv(\"HOME\")
",
        )?;
        std::fs::write(
            repo.path().join("lib.rs"),
            "\
use std::collections::HashMap;
use core::fmt::Debug;
use serde::Serialize;

struct Foo;

impl Debug for Foo {}
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);

        let has_node = |id: &str| graph.nodes.iter().any(|node| node.id().as_str() == id);

        // Known stdlib/prelude references resolve to shared external Package
        // nodes rather than per-file Unresolved noise.
        assert!(has_node("package:os"), "expected package:os");
        assert!(
            has_node("package:__future__"),
            "expected package:__future__"
        );
        assert!(has_node("package:std"), "expected package:std");
        assert!(has_node("package:Debug"), "expected package:Debug");
        assert!(
            !has_node("unresolved:os"),
            "os should not be Unresolved once classified as stdlib"
        );
        assert!(
            !has_node("unresolved:__future__"),
            "__future__ should not be Unresolved once classified as stdlib"
        );
        assert!(
            !has_node("unresolved:std::collections::HashMap"),
            "std:: use path should not be Unresolved once classified as stdlib"
        );
        assert!(
            !has_node("unresolved:Debug"),
            "Debug should not be Unresolved once classified as a prelude type"
        );

        // Genuinely unknown third-party references still fall through to
        // Unresolved -- this is a classification split, not a blanket
        // silencer of every import.
        assert!(
            has_node("unresolved:requests"),
            "third-party requests import should remain Unresolved"
        );
        assert!(
            !has_node("package:requests"),
            "third-party requests import must not be misclassified as stdlib"
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str().starts_with("unresolved:serde")),
            "third-party serde use path should remain Unresolved"
        );

        Ok(())
    }
}
