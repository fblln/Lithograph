//! Graph construction: merges artifact inventory and per-artifact analyzer
//! output into one typed semantic graph.

use crate::analysis::{
    ActionsProfileAnalyzer, ActionsStepHint, CargoProfileAnalyzer, ComposeProfileAnalyzer,
    ConfigReferenceKind, DockerfileAnalyzer, GenericTextExtractor, MarkdownAnalyzer,
    PyProjectAnalyzer, PythonAnalyzer, PythonImportKind, PythonReferenceKind, RequirementsAnalyzer,
    RustAnalyzer, StructuredAnalyzer, StructuredFormat, TextFindingKind, python, rust_source,
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
    /// so unsupported artifacts remain visible in the graph.
    pub fn build(&self, repo_root: &Path, artifacts: &[Artifact]) -> Graph {
        let mut state = BuilderState::new(artifacts);

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
            let Ok(text) = fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
                continue;
            };
            state.process_artifact(artifact, &text, repo_root);
        }

        state.finish()
    }
}

struct BuilderState {
    nodes: BTreeMap<GraphNodeId, GraphNode>,
    relations: Vec<Relation>,
    relation_count: usize,
    artifact_paths: BTreeSet<String>,
    python_modules: BTreeMap<String, GraphNodeId>,
    rust_modules: BTreeMap<String, GraphNodeId>,
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
                let module_path = rust_source::module_path(artifact.path.as_str());
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

    fn process_artifact(&mut self, artifact: &Artifact, text: &str, repo_root: &Path) {
        let node = artifact_node_id(artifact);
        let name = file_name(artifact.path.as_str());

        match (&artifact.analyzer, artifact.detected_format.as_deref()) {
            (AnalyzerSelection::Specialized(format), _) if format == "python" => {
                self.process_python(artifact, text, &node);
            }
            (AnalyzerSelection::Specialized(format), _) if format == "rust" => {
                self.process_rust(artifact, text, &node);
            }
            (AnalyzerSelection::Specialized(format), _) if format == "requirements-txt" => {
                self.process_requirements(artifact, text, &node);
            }
            (AnalyzerSelection::Structured(format), _) if format == "dockerfile" => {
                self.process_dockerfile(artifact, text, &node);
            }
            (AnalyzerSelection::Structured(format), _) if format == "markdown" => {
                self.process_markdown(artifact, text, repo_root, &node);
            }
            (AnalyzerSelection::Structured(format), _) if format == "docker-compose" => {
                self.process_compose(artifact, text, &node);
            }
            (AnalyzerSelection::Structured(format), _) if format == "github-actions" => {
                self.process_actions(artifact, text, &node);
            }
            (AnalyzerSelection::Structured(format), _)
                if format == "toml" && name == "Cargo.toml" =>
            {
                self.process_cargo(artifact, text, &node);
            }
            (AnalyzerSelection::Structured(format), _)
                if format == "toml" && name == "pyproject.toml" =>
            {
                self.process_pyproject(artifact, text, &node);
            }
            (AnalyzerSelection::Structured(format), _)
                if matches!(format.as_str(), "yaml" | "json" | "toml") =>
            {
                let format = match format.as_str() {
                    "yaml" => StructuredFormat::Yaml,
                    "json" => StructuredFormat::Json,
                    _ => StructuredFormat::Toml,
                };
                self.process_structured(artifact, text, format, &node);
            }
            (AnalyzerSelection::GenericText, _) => {
                self.process_generic_text(artifact, text, &node);
            }
            _ => {}
        }
    }

    fn process_python(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let analysis = PythonAnalyzer.analyze(artifact, text);
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
                        .unwrap_or_else(|| self.unresolved(&name.name));
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
                    .unwrap_or_else(|| {
                        let marker = resolved.unwrap_or_else(|| {
                            format!(
                                "{}{}",
                                ".".repeat(import.relative_level as usize),
                                import.module.clone().unwrap_or_default()
                            )
                        });
                        self.unresolved(&marker)
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

    fn process_rust(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let analysis = RustAnalyzer.analyze(artifact, text);
        let module_id = self
            .rust_modules
            .get(&analysis.module_path)
            .cloned()
            .unwrap_or_else(|| {
                self.module(
                    &analysis.module_path,
                    ModuleLanguage::Rust,
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

        for item in analysis
            .structs
            .iter()
            .map(|item| (item, SymbolKind::Struct))
            .chain(analysis.enums.iter().map(|item| (item, SymbolKind::Enum)))
        {
            let (item, kind) = item;
            let qualified = format!("{}::{}", analysis.module_path, item.name);
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
            let qualified = format!("{}::{}", analysis.module_path, trait_item.name);
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
            let qualified = format!("{}::{}", analysis.module_path, function.name);
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
                .unwrap_or_else(|| self.unresolved(&imp.target_type));
            let target = symbol_ids
                .get(trait_name)
                .cloned()
                .unwrap_or_else(|| self.unresolved(trait_name));
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
                .unwrap_or_else(|| self.unresolved(&use_.path));
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::Imports,
                Confidence::High,
                vec![use_.evidence.clone()],
            );
        }
    }

    fn process_dockerfile(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let analysis = DockerfileAnalyzer.analyze(artifact, text);

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
        text: &str,
        repo_root: &Path,
        artifact_node: &GraphNodeId,
    ) {
        let analysis = MarkdownAnalyzer.analyze(artifact, text, repo_root);

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
        text: &str,
        format: StructuredFormat,
        artifact_node: &GraphNodeId,
    ) {
        let analysis = StructuredAnalyzer.analyze(artifact, text, format);

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

    fn process_cargo(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let profile = CargoProfileAnalyzer.analyze(artifact, text);
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

    fn process_pyproject(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let profile = PyProjectAnalyzer.analyze(artifact, text);
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

    fn process_requirements(
        &mut self,
        artifact: &Artifact,
        text: &str,
        artifact_node: &GraphNodeId,
    ) {
        let profile = RequirementsAnalyzer.analyze(artifact, text);
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

    fn process_compose(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let profile = ComposeProfileAnalyzer.analyze(artifact, text);
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

    fn process_actions(&mut self, artifact: &Artifact, text: &str, artifact_node: &GraphNodeId) {
        let profile = ActionsProfileAnalyzer.analyze(artifact, text);
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
        text: &str,
        artifact_node: &GraphNodeId,
    ) {
        let findings = GenericTextExtractor.extract(artifact, text);
        for finding in &findings {
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
    use crate::graph::{GraphNode, RelationKind};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
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
}
