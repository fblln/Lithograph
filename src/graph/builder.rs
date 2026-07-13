//! Graph construction: merges artifact inventory and per-artifact analyzer
//! output into one typed semantic graph.

use crate::analysis::{
    ActionsProfile, ActionsProfileAnalyzer, ActionsStepHint, AnalysisCache, AnalyzerKind,
    AnalyzerOutput, CargoProfile, CargoProfileAnalyzer, ComposeProfile, ComposeProfileAnalyzer,
    ConfigReferenceKind, DockerfileAnalysis, DockerfileAnalyzer, EnvironmentFacts,
    GenericTextExtractor, MarkdownAnalysis, MarkdownAnalyzer, PackageManifestAnalysis,
    PackageManifestFormat, ProtocolFormat, ProtocolRoute, PyProjectAnalyzer, PyProjectProfile,
    PythonAnalysis, PythonAnalyzer, PythonImportKind, PythonReferenceKind, RequirementsAnalyzer,
    RequirementsProfile, RustAnalysis, RustAnalyzer, RustReferenceKind, RustWorkspaceAnalysis,
    RustWorkspaceAnalyzer, StructuredAnalysis, StructuredAnalyzer, StructuredFormat,
    SyntaxIndexedLanguage, TextFinding, TextFindingKind, TreeSitterAdapterOutput,
    TypeScriptAnalysis, TypeScriptAnalyzer, TypeScriptLanguage, is_python_stdlib_module, python,
    rust_source, rust_std_crate,
};
use crate::domain::{
    AnalyzerSelection, Artifact, ArtifactId, Confidence, EvidenceRef, ModelExposurePolicy,
    TextStatus,
};
use crate::graph::model::{
    ArtifactNode, CommandNode, ConfigNode, ConfigNodeKind, ContainerImageNode, DocumentationNode,
    EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, PackageNode, Relation,
    RelationKind, RelationProvenance, RelationResolution, SymbolKind, SymbolNode, UnresolvedNode,
};
use crate::graph::{
    GRAPH_BUILD_PIPELINE_VERSION, GraphBuildOutput, GraphBuildPass, GraphBuildTraceConfig,
    GraphBuildTraceDetail, GraphDecisionTrace,
};
use crate::inventory::language::by_name as registry_language;
use crate::resolve::{ConfigFact, EnvFact, FactRole, FactSourceKind};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use std::time::Instant;

#[derive(Debug, Default)]
struct CloneDiagnostics {
    candidate_count: u64,
    comparison_count: u64,
    emitted_count: u64,
    rejected_near_threshold_count: u64,
    pruned_count: u64,
    decisions: Vec<GraphDecisionTrace>,
}

#[derive(Debug)]
struct CloneCandidate {
    id: GraphNodeId,
    evidence: EvidenceRef,
    tokens: BTreeSet<String>,
    language: String,
    size_band: u32,
}

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
        self.build_with_report(repo_root, artifacts, None).graph
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
        self.build_with_report(repo_root, artifacts, cache).graph
    }

    /// Builds a graph and reports the deterministic pass outputs used to
    /// create it. This is the production entry point for callers needing
    /// invalidation and observability data.
    pub fn build_with_report(
        &self,
        repo_root: &Path,
        artifacts: &[Artifact],
        cache: Option<&AnalysisCache>,
    ) -> GraphBuildOutput {
        self.build_with_optional_trace(repo_root, artifacts, cache, None)
    }

    /// Builds the same graph as [`Self::build_with_report`] while retaining
    /// deterministic, inspectable state after every pipeline pass.
    pub fn build_with_trace(
        &self,
        repo_root: &Path,
        artifacts: &[Artifact],
        cache: Option<&AnalysisCache>,
        config: GraphBuildTraceConfig,
    ) -> GraphBuildOutput {
        self.build_with_optional_trace(repo_root, artifacts, cache, Some(config))
    }

    fn build_with_optional_trace(
        &self,
        repo_root: &Path,
        artifacts: &[Artifact],
        cache: Option<&AnalysisCache>,
        trace_config: Option<GraphBuildTraceConfig>,
    ) -> GraphBuildOutput {
        let structure_started = Instant::now();
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
            let cache_key = artifact_cache_key(artifact);
            let workspace =
                match cache.and_then(|cache| cache.get(&cache_key, AnalyzerKind::RustWorkspace)) {
                    Some(AnalyzerOutput::RustWorkspace(analysis)) => analysis,
                    _ => {
                        let fresh = RustWorkspaceAnalyzer.analyze(artifact, repo_root);
                        if let Some(cache) = cache {
                            cache.put(&cache_key, &AnalyzerOutput::RustWorkspace(fresh.clone()));
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

        let mut output = GraphBuildOutput {
            graph: Graph {
                nodes: Vec::new(),
                relations: Vec::new(),
            },
            pipeline_version: GRAPH_BUILD_PIPELINE_VERSION,
            passes: Vec::new(),
            trace: None,
        };
        let trace_detail = trace_config
            .as_ref()
            .map_or(GraphBuildTraceDetail::Summary, |config| {
                config.detail.clone()
            });
        if let Some(config) = &trace_config {
            output.enable_trace(config);
        }
        output.graph = state.snapshot();
        output.record(GraphBuildPass::Structure);
        output.record_trace(
            GraphBuildPass::Structure,
            &trace_detail,
            structure_started,
            BTreeMap::new(),
            Vec::new(),
        );

        let definitions_started = Instant::now();
        for artifact in artifacts {
            if artifact.text_status != TextStatus::Text
                || artifact.model_policy == ModelExposurePolicy::Never
            {
                continue;
            }
            let Some(kind) = analyzer_kind(artifact) else {
                continue;
            };
            let cache_key = artifact_cache_key(artifact);
            let mut output = match cache.and_then(|cache| cache.get(&cache_key, kind)) {
                Some(cached) => cached,
                None => {
                    let Ok(text) = fs::read_to_string(repo_root.join(artifact.path.as_str()))
                    else {
                        continue;
                    };
                    let fresh = compute_fresh(artifact, &text, repo_root, kind);
                    if let Some(cache) = cache {
                        cache.put(&cache_key, &fresh);
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

        output.graph = state.snapshot();
        output.record(GraphBuildPass::DefinitionsAndImports);
        output.record_trace(
            GraphBuildPass::DefinitionsAndImports,
            &trace_detail,
            definitions_started,
            BTreeMap::new(),
            Vec::new(),
        );

        let enrichment_started = Instant::now();
        let clone_started = Instant::now();
        let clone_diagnostics = state.detect_near_clones(
            repo_root,
            &trace_detail,
            trace_config
                .as_ref()
                .map_or(&[][..], |config| config.selectors.as_slice()),
        );
        let clone_duration_us = clone_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        state.materialize_environment_facts();
        let mut graph = state.finish();
        output.graph = graph.clone();
        output.record(GraphBuildPass::Enrichment);
        output.record_trace(
            GraphBuildPass::Enrichment,
            &trace_detail,
            enrichment_started,
            BTreeMap::from([
                (
                    "clone_candidates".to_owned(),
                    clone_diagnostics.candidate_count,
                ),
                (
                    "clone_comparisons".to_owned(),
                    clone_diagnostics.comparison_count,
                ),
                ("clone_emitted".to_owned(), clone_diagnostics.emitted_count),
                (
                    "clone_rejected_near_threshold".to_owned(),
                    clone_diagnostics.rejected_near_threshold_count,
                ),
                ("clone_pruned".to_owned(), clone_diagnostics.pruned_count),
            ]),
            clone_diagnostics.decisions,
        );
        output.record_component_duration(
            GraphBuildPass::Enrichment,
            "clone_detection",
            clone_duration_us,
        );
        // LIT-23.1: applies to every caller (init/update, inspect, MCP
        // tools, tests) uniformly, the same way detect_near_clones already
        // does -- a post-processing pass belongs here, not bolted onto one
        // caller, or callers would see inconsistently-resolved graphs.
        let resolution_started = Instant::now();
        let relations_before_resolution = graph.relations.clone();
        crate::resolve::HybridResolverPipeline::default_pipeline().resolve(&mut graph);
        crate::resolve::resolve_environment_links(&mut graph);
        prune_orphaned_unresolved_nodes(&mut graph);
        let resolution_decisions = trace_decisions_for_relations(
            &graph,
            &relations_before_resolution,
            &trace_detail,
            trace_config
                .as_ref()
                .map_or(&[][..], |config| config.selectors.as_slice()),
        );
        output.graph = graph;
        output.record(GraphBuildPass::Resolution);
        output.record_trace(
            GraphBuildPass::Resolution,
            &trace_detail,
            resolution_started,
            BTreeMap::from([(
                "resolved_relations".to_owned(),
                resolution_decisions.len() as u64,
            )]),
            resolution_decisions,
        );
        // Analytics and persistence have explicit contracts now. Their
        // topology-preserving implementations are added by their dedicated
        // graph-analytics/store tasks; recording them here keeps pass order
        // and invalidation stable for all callers in the interim.
        let analytics_started = Instant::now();
        output.record(GraphBuildPass::Analytics);
        output.record_trace(
            GraphBuildPass::Analytics,
            &trace_detail,
            analytics_started,
            BTreeMap::new(),
            Vec::new(),
        );
        let persistence_started = Instant::now();
        output.record(GraphBuildPass::Persistence);
        output.record_trace(
            GraphBuildPass::Persistence,
            &trace_detail,
            persistence_started,
            BTreeMap::new(),
            Vec::new(),
        );
        let finalize_started = Instant::now();
        output.record(GraphBuildPass::Finalize);
        output.record_trace(
            GraphBuildPass::Finalize,
            &trace_detail,
            finalize_started,
            BTreeMap::new(),
            Vec::new(),
        );
        output
    }
}

fn artifact_cache_key(artifact: &Artifact) -> String {
    blake3::hash(format!("{}\0{}", artifact.content_hash, artifact.path.as_str()).as_bytes())
        .to_hex()
        .to_string()
}

fn trace_decisions_for_relations(
    graph: &Graph,
    before: &[Relation],
    detail: &GraphBuildTraceDetail,
    selectors: &[String],
) -> Vec<GraphDecisionTrace> {
    graph
        .relations
        .iter()
        .filter(|relation| {
            relation.provenance.as_ref().is_some_and(|provenance| {
                provenance.resolution == RelationResolution::HybridResolved
            })
        })
        .filter(|relation| {
            (*detail == GraphBuildTraceDetail::Full && selectors.is_empty())
                || selectors.iter().any(|selector| {
                    relation.source.as_str().contains(selector)
                        || relation.target.as_str().contains(selector)
                        || relation
                            .evidence
                            .iter()
                            .any(|item| item.path.as_str().contains(selector))
                })
        })
        .flat_map(|relation| {
            let provenance = relation.provenance.as_ref();
            let selected = GraphDecisionTrace {
                kind: format!("{:?}", relation.kind).to_ascii_lowercase(),
                source: relation.source.as_str().to_owned(),
                target: relation.target.as_str().to_owned(),
                strategy: provenance
                    .map_or("unknown", |item| item.resolver_strategy.as_str())
                    .to_owned(),
                outcome: "selected".to_owned(),
                score_millionths: match relation.confidence {
                    Confidence::High => 1_000_000,
                    Confidence::Low => 500_000,
                },
                evidence_paths: relation
                    .evidence
                    .iter()
                    .map(|item| item.path.as_str().to_owned())
                    .collect(),
                reason: "candidate satisfied resolver kind, scope, and confidence filters"
                    .to_owned(),
            };
            let mut decisions = vec![selected];
            if let Some(original) = before.iter().find(|candidate| {
                candidate.source == relation.source
                    && candidate.kind == relation.kind
                    && candidate.target != relation.target
                    && candidate.evidence == relation.evidence
            }) {
                decisions.push(GraphDecisionTrace {
                    kind: format!("{:?}", relation.kind).to_ascii_lowercase(),
                    source: original.source.as_str().to_owned(),
                    target: original.target.as_str().to_owned(),
                    strategy: provenance
                        .map_or("unknown", |item| item.resolver_strategy.as_str())
                        .to_owned(),
                    outcome: "rejected".to_owned(),
                    score_millionths: 0,
                    evidence_paths: original
                        .evidence
                        .iter()
                        .map(|item| item.path.as_str().to_owned())
                        .collect(),
                    reason: "raw unresolved candidate was superseded by a typed resolver candidate"
                        .to_owned(),
                });
            }
            decisions
        })
        .collect()
}

fn clone_decision(
    left: &GraphNodeId,
    right: &GraphNodeId,
    left_evidence: &EvidenceRef,
    right_evidence: &EvidenceRef,
    similarity: f64,
    outcome: &str,
    reason: &str,
) -> GraphDecisionTrace {
    GraphDecisionTrace {
        kind: "near_clone".to_owned(),
        source: left.as_str().to_owned(),
        target: right.as_str().to_owned(),
        strategy: "lexical_jaccard_similarity".to_owned(),
        outcome: outcome.to_owned(),
        score_millionths: (similarity * 1_000_000.0).round() as u32,
        evidence_paths: vec![
            left_evidence.path.as_str().to_owned(),
            right_evidence.path.as_str().to_owned(),
        ],
        reason: reason.to_owned(),
    }
}

/// Removes `Unresolved` nodes no relation references anymore after hybrid
/// resolution (LIT-23.1): when every relation that targeted a raw
/// syntax-only fact gets upgraded to a real node, the placeholder is dead
/// weight -- leaving it in the graph would still surface the very raw-text
/// noise resolution was meant to eliminate, just disconnected from every
/// relation. A node created and immediately shared by several relations
/// (the common case, since `BuilderState::unresolved` deduplicates by id)
/// survives as long as at least one relation still targets it.
fn prune_orphaned_unresolved_nodes(graph: &mut Graph) {
    let referenced: BTreeSet<&GraphNodeId> = graph
        .relations
        .iter()
        .flat_map(|relation| [&relation.source, &relation.target])
        .collect();
    graph
        .nodes
        .retain(|node| !matches!(node, GraphNode::Unresolved(_)) || referenced.contains(node.id()));
}

/// Returns the analyzer that would handle `artifact`, or `None` when no
/// analyzer applies (binary/unsafe artifacts keep only their `Artifact` node).
/// Mirrors the routing table every `process_artifact` call site used to
/// dispatch on directly.
fn analyzer_kind(artifact: &Artifact) -> Option<AnalyzerKind> {
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
fn compute_fresh(
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

struct BuilderState {
    nodes: BTreeMap<GraphNodeId, GraphNode>,
    relations: Vec<Relation>,
    relation_count: usize,
    environment_facts: EnvironmentFacts,
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
            environment_facts: EnvironmentFacts::default(),
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
            // Workspace roots do not have a `[package]` table, so the raw
            // TOML pass cannot materialize their members. Cargo metadata is
            // the authoritative source for every in-repository member.
            self.package(&package.name, false);
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

    fn materialize_environment_facts(&mut self) {
        let mut facts = std::mem::take(&mut self.environment_facts);
        facts.sort_deterministically();
        for fact in &facts.env {
            self.materialize_env_fact(fact);
        }
        for fact in &facts.config {
            self.materialize_config_fact(fact);
        }
        self.environment_facts = facts;
    }

    fn materialize_env_fact(&mut self, fact: &EnvFact) {
        let target = self.env_var(fact.name.original());
        let owner = fact.owner.clone().or_else(|| {
            (fact.source == FactSourceKind::SourceCode)
                .then(|| self.smallest_symbol_owner(&fact.evidence))
                .flatten()
                .map(|id| id.to_string())
        });
        let source = self.fact_source_node(fact.source, owner.as_deref(), &fact.evidence);
        let kind = match fact.role {
            FactRole::Define => RelationKind::DefinesEnv,
            FactRole::Read | FactRole::Reference => RelationKind::ReadsEnv,
        };
        self.relate_if_absent(
            source,
            target,
            kind,
            fact.confidence,
            vec![fact.evidence.clone()],
            Some(environment_provenance(fact.source, fact.confidence)),
        );
    }

    fn materialize_config_fact(&mut self, fact: &ConfigFact) {
        let target = self.config_key(fact);
        let source = self.fact_source_node(fact.source, fact.owner.as_deref(), &fact.evidence);
        let kind = match fact.role {
            FactRole::Define => RelationKind::BindsConfig,
            FactRole::Read | FactRole::Reference => RelationKind::ReferencesConfig,
        };
        self.relate_if_absent(
            source,
            target,
            kind,
            fact.confidence,
            vec![fact.evidence.clone()],
            Some(environment_provenance(fact.source, fact.confidence)),
        );
    }

    fn config_key(&mut self, fact: &ConfigFact) -> GraphNodeId {
        let id = GraphNodeId::new(format!("config-key:{}", fact.key.canonical));
        self.insert(GraphNode::Config(ConfigNode {
            id: id.clone(),
            kind: ConfigNodeKind::Key,
            name: fact.key.canonical.clone(),
            evidence: fact.evidence.clone(),
        }))
    }

    fn fact_source_node(
        &self,
        source: FactSourceKind,
        owner: Option<&str>,
        evidence: &EvidenceRef,
    ) -> GraphNodeId {
        if let Some(owner) = owner {
            let path = evidence.path.as_str();
            return match source {
                FactSourceKind::Compose => {
                    GraphNodeId::new(format!("config:{path}#services.{owner}"))
                }
                FactSourceKind::CiWorkflow => {
                    GraphNodeId::new(format!("config:{path}#jobs.{owner}"))
                }
                _ => GraphNodeId::new(owner),
            };
        }
        GraphNodeId::new(format!("artifact:{}", evidence.path.as_str()))
    }

    fn smallest_symbol_owner(&self, evidence: &EvidenceRef) -> Option<GraphNodeId> {
        let line = evidence.span.as_ref()?.start_line;
        self.nodes
            .values()
            .filter_map(|node| {
                let GraphNode::Symbol(symbol) = node else {
                    return None;
                };
                if symbol.evidence.artifact_id != evidence.artifact_id {
                    return None;
                }
                let span = symbol.evidence.span.as_ref()?;
                (span.start_line <= line && line <= span.end_line)
                    .then_some((span.end_line - span.start_line, symbol.id.clone()))
            })
            .min_by_key(|(span_length, id)| (*span_length, id.clone()))
            .map(|(_, id)| id)
    }

    fn relate(
        &mut self,
        source: GraphNodeId,
        target: GraphNodeId,
        kind: RelationKind,
        confidence: Confidence,
        evidence: Vec<EvidenceRef>,
    ) {
        self.relate_with_provenance(
            source,
            target,
            kind,
            confidence,
            evidence,
            Some(RelationProvenance {
                language: None,
                resolver_strategy: "graph-builder".to_owned(),
                resolution: RelationResolution::SyntaxOnly,
                confidence,
            }),
        );
    }

    fn relate_with_provenance(
        &mut self,
        source: GraphNodeId,
        target: GraphNodeId,
        kind: RelationKind,
        confidence: Confidence,
        evidence: Vec<EvidenceRef>,
        provenance: Option<RelationProvenance>,
    ) {
        self.relation_count += 1;
        self.relations.push(Relation {
            id: format!("relation:{}", self.relation_count),
            source,
            target,
            kind,
            confidence,
            evidence,
            provenance,
        });
    }

    fn relate_if_absent(
        &mut self,
        source: GraphNodeId,
        target: GraphNodeId,
        kind: RelationKind,
        confidence: Confidence,
        evidence: Vec<EvidenceRef>,
        provenance: Option<RelationProvenance>,
    ) {
        if self.relations.iter().any(|relation| {
            relation.source == source && relation.target == target && relation.kind == kind
        }) {
            return;
        }
        self.relate_with_provenance(source, target, kind, confidence, evidence, provenance);
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
        let id = GraphNodeId::new(format!("package:{name}"));
        if !is_external && let Some(GraphNode::Package(existing)) = self.nodes.get_mut(&id) {
            existing.is_external = false;
            return id;
        }
        self.insert(GraphNode::Package(PackageNode {
            id,
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
            AnalyzerOutput::Python(analysis) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_python(&analysis));
                self.process_python(artifact, analysis, &node);
            }
            AnalyzerOutput::Rust(analysis) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_rust(&analysis));
                self.process_rust(artifact, analysis, &node);
            }
            AnalyzerOutput::TypeScript(analysis) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_typescript(&analysis));
                self.process_typescript(artifact, analysis, &node)
            }
            AnalyzerOutput::Requirements(profile) => {
                self.process_requirements(profile, &node);
            }
            AnalyzerOutput::Dockerfile(analysis) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_dockerfile(&analysis));
                self.process_dockerfile(artifact, analysis, &node);
            }
            AnalyzerOutput::Markdown(analysis) => self.process_markdown(artifact, analysis, &node),
            AnalyzerOutput::Compose(profile) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_compose(&profile));
                self.process_compose(artifact, profile, &node);
            }
            AnalyzerOutput::Actions(profile) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_actions(&profile));
                self.process_actions(artifact, profile, &node);
            }
            AnalyzerOutput::Cargo(profile) => self.process_cargo(profile, &node),
            AnalyzerOutput::PyProject(profile) => self.process_pyproject(profile, &node),
            AnalyzerOutput::Structured(_, analysis) => {
                self.environment_facts
                    .extend(EnvironmentFacts::from_structured(&analysis));
                self.process_structured(artifact, analysis, &node);
            }
            AnalyzerOutput::SyntaxIndexed(language, output) => {
                self.process_syntax_indexed(artifact, language, output, &node);
            }
            AnalyzerOutput::PackageManifest(format, analysis) => {
                self.process_package_manifest(format, analysis, &node);
            }
            AnalyzerOutput::Protocol(_, routes) => {
                self.process_protocol_routes(artifact, &routes, &node);
            }
            AnalyzerOutput::GenericText(findings) => {
                self.process_generic_text(artifact, &findings, &node);
            }
            AnalyzerOutput::Environment(facts) => self.environment_facts.extend(facts),
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
        let mut callable_ids: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();

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
            callable_ids
                .entry(class.name.clone())
                .or_default()
                .push(id.clone());
            for decorator in &class.decorators {
                let target = self.unresolved(decorator);
                self.relate_with_provenance(
                    id.clone(),
                    target,
                    RelationKind::Decorates,
                    Confidence::Low,
                    vec![class.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }

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
                    method_id.clone(),
                    RelationKind::Contains,
                    Confidence::High,
                    vec![method.evidence.clone()],
                );
                self.relate_with_provenance(
                    id.clone(),
                    method_id.clone(),
                    RelationKind::HasMethod,
                    Confidence::High,
                    vec![method.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::High,
                    )),
                );
                self.relate_with_provenance(
                    method_id.clone(),
                    id.clone(),
                    RelationKind::MemberOf,
                    Confidence::High,
                    vec![method.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::High,
                    )),
                );
                if let Some(return_type) = &method.return_type {
                    let target = self.unresolved(return_type);
                    self.relate_with_provenance(
                        method_id.clone(),
                        target,
                        RelationKind::UsesType,
                        Confidence::Low,
                        vec![method.evidence.clone()],
                        Some(format_provenance(
                            "python",
                            RelationResolution::SyntaxOnly,
                            Confidence::Low,
                        )),
                    );
                }
                callable_ids
                    .entry(method.name.clone())
                    .or_default()
                    .push(method_id);
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
            self.process_python_route_decorators(artifact, artifact_node, function, &id);
            for decorator in &function.decorators {
                let target = self.unresolved(decorator);
                self.relate_with_provenance(
                    id.clone(),
                    target,
                    RelationKind::Decorates,
                    Confidence::Low,
                    vec![function.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
            if let Some(return_type) = &function.return_type {
                let target = self.unresolved(return_type);
                self.relate_with_provenance(
                    id.clone(),
                    target,
                    RelationKind::UsesType,
                    Confidence::Low,
                    vec![function.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
            symbol_ids.insert(function.name.clone(), id.clone());
            callable_ids
                .entry(function.name.clone())
                .or_default()
                .push(id);
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
            self.process_python_reference(
                artifact,
                artifact_node,
                reference,
                &symbol_ids,
                &callable_ids,
            );
        }

        // Base classes (LIT-22.3.3): only resolves to a same-file class by
        // bare name -- cross-module base classes stay `Unresolved` rather
        // than guessing which import they came from (AC3).
        for class in &analysis.classes {
            let Some(class_id) = symbol_ids.get(&class.name) else {
                continue;
            };
            for base in &class.bases {
                let base_name = base.rsplit('.').next().unwrap_or(base.as_str());
                let target = symbol_ids
                    .get(base_name)
                    .cloned()
                    .unwrap_or_else(|| self.unresolved(base));
                self.relate_with_provenance(
                    class_id.clone(),
                    target,
                    RelationKind::Inherits,
                    Confidence::Low,
                    vec![class.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
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
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::Imports,
                        Confidence::High,
                        vec![import.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::HybridResolved,
                            Confidence::High,
                        )),
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
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Imports,
                    Confidence::High,
                    vec![import.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::HybridResolved,
                        Confidence::High,
                    )),
                );
            }
        }
    }

    /// Turns a module-level function's decorators that look like an HTTP
    /// route registration into a first-class `Route` config node
    /// (LIT-22.3.4 AC1). Class-based views/routers aren't covered -- most
    /// Flask/FastAPI routes are plain module-level functions, and guessing
    /// at class-method route registration without more evidence would risk
    /// false positives.
    fn process_python_route_decorators(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        function: &crate::analysis::PythonFunction,
        handler: &GraphNodeId,
    ) {
        for (index, decorator) in function.decorators.iter().enumerate() {
            let Some((method, path)) = python::parse_route_decorator(decorator) else {
                continue;
            };
            let key = format!("route.{}.{index}", function.name);
            let route_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Route,
                &format!("{method} {path}"),
                function.evidence.clone(),
            );
            self.relate_with_provenance(
                artifact_node.clone(),
                route_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
                Some(artifact_provenance(
                    artifact,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            self.relate_with_provenance(
                route_id,
                handler.clone(),
                RelationKind::HandlesRoute,
                Confidence::High,
                vec![function.evidence.clone()],
                Some(format_provenance(
                    "python",
                    RelationResolution::HybridResolved,
                    Confidence::High,
                )),
            );
        }
    }

    fn process_python_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::PythonReference,
        symbol_ids: &BTreeMap<String, GraphNodeId>,
        callable_ids: &BTreeMap<String, Vec<GraphNodeId>>,
    ) {
        match reference.kind {
            PythonReferenceKind::Call => {
                let simple = reference
                    .value
                    .rsplit('.')
                    .next()
                    .unwrap_or(&reference.value);
                if let Some([target]) = callable_ids.get(simple).map(Vec::as_slice) {
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target.clone(),
                        RelationKind::Calls,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::HybridResolved,
                            reference.confidence,
                        )),
                    );
                } else {
                    let target = self.unresolved(&reference.value);
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::Calls,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::SyntaxOnly,
                            reference.confidence,
                        )),
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
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Imports,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::Fallback,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::Ctypes => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::Fallback,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::ConfigPath => {
                let (target, path_confidence) = self.reference_target(&reference.value);
                let confidence = reference.confidence.min(path_confidence);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::HybridResolved,
                        confidence,
                    )),
                );
            }
            PythonReferenceKind::HttpCall => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::SyntaxOnly,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::Emits | PythonReferenceKind::ListensOn => {
                let kind = if reference.kind == PythonReferenceKind::Emits {
                    RelationKind::Emits
                } else {
                    RelationKind::ListensOn
                };
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    kind,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::SyntaxOnly,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::DataFlows => {
                if let Some(target) = symbol_ids.get(&reference.value).cloned() {
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::DataFlows,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::HybridResolved,
                            reference.confidence,
                        )),
                    );
                }
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
                .unwrap_or_else(|| match rust_std_crate(candidate) {
                    Some(crate_name) => self.package(crate_name, true),
                    None => self.unresolved(&use_.path),
                });
            self.relate_with_provenance(
                artifact_node.clone(),
                target,
                RelationKind::Imports,
                Confidence::High,
                vec![use_.evidence.clone()],
                Some(artifact_provenance(
                    artifact,
                    RelationResolution::HybridResolved,
                    Confidence::High,
                )),
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
            RustReferenceKind::Ffi => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Ffi,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(format_provenance(
                        "rust",
                        RelationResolution::SyntaxOnly,
                        reference.confidence,
                    )),
                );
            }
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

    /// Resolves a generic tree-sitter [`TreeSitterAdapterOutput`] (LIT-22.2.3)
    /// into a `Module` node, one `Symbol` node per definition fact, and an
    /// `Imports` relation per import fact. Unlike Python/Rust's specialized
    /// processing, this never resolves an import target to a known module or
    /// package -- it always lands on an `Unresolved` node with
    /// `RelationResolution::SyntaxOnly` provenance, since cross-file
    /// resolution for these languages is LIT-22.3's hybrid resolver, not
    /// this syntax-only pass (AC3: never overclaim `HybridResolved`).
    fn process_syntax_indexed(
        &mut self,
        artifact: &Artifact,
        language: SyntaxIndexedLanguage,
        output: TreeSitterAdapterOutput,
        artifact_node: &GraphNodeId,
    ) {
        let module_id = self.module(
            artifact.path.as_str(),
            ModuleLanguage::SyntaxIndexed(language),
            file_evidence(artifact),
        );
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        self.process_syntax_indexed_facts(artifact, language, output, artifact_node, &[]);
    }

    /// Applies syntax-level facts after a language-specific declaration pass.
    /// `typed_definition_kinds` suppresses generic `Definition` symbols for
    /// declarations the specialized pass already represented precisely.
    fn process_syntax_indexed_facts(
        &mut self,
        artifact: &Artifact,
        language: SyntaxIndexedLanguage,
        output: TreeSitterAdapterOutput,
        artifact_node: &GraphNodeId,
        typed_definition_kinds: &[&str],
    ) {
        let registry_id = language.registry_id();

        for definition in &output.definitions {
            if typed_definition_kinds.contains(&definition.kind.as_str()) {
                continue;
            }
            let evidence = syntax_fact_evidence(artifact, definition.span.clone());
            let qualified = format!(
                "{}::{}@L{}",
                artifact.path, definition.kind, definition.span.start_line
            );
            let symbol_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{qualified}")),
                kind: SymbolKind::Definition,
                qualified_name: qualified,
                doc: None,
                evidence: evidence.clone(),
            }));
            self.relate_with_provenance(
                artifact_node.clone(),
                symbol_id,
                RelationKind::Contains,
                Confidence::High,
                vec![evidence],
                Some(format_provenance(
                    registry_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }

        for import in &output.imports {
            let evidence = syntax_fact_evidence(artifact, import.span.clone());
            let target = self.unresolved(&import.text);
            self.relate_with_provenance(
                artifact_node.clone(),
                target,
                RelationKind::Imports,
                Confidence::Low,
                vec![evidence],
                Some(format_provenance(
                    registry_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::Low,
                )),
            );
        }

        // Type references and general use-site references (LIT-22.3.3):
        // one relation per distinct identifier text per file, deduplicated
        // (a single-file syntax pass has no scoping/symbol-table context to
        // tell which occurrence is meaningful, so keeping every occurrence
        // would just be noise) and targeting `Unresolved` -- this file's
        // syntax alone can't tell whether `Widget` is a locally-defined
        // type, an imported one, or a typo, so resolving it correctly is a
        // hybrid-resolver's job (AC3: never fabricate a match here).
        let mut seen_symbols: BTreeSet<&str> = BTreeSet::new();
        for symbol in &output.symbols {
            if !seen_symbols.insert(symbol.text.as_str()) {
                continue;
            }
            let kind = if symbol.kind == "type_identifier" {
                RelationKind::TypeRefs
            } else {
                RelationKind::Usages
            };
            let evidence = syntax_fact_evidence(artifact, symbol.span.clone());
            let target = self.unresolved(&symbol.text);
            self.relate_with_provenance(
                artifact_node.clone(),
                target,
                kind,
                Confidence::Low,
                vec![evidence],
                Some(format_provenance(
                    registry_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::Low,
                )),
            );
        }
    }

    /// Adds TypeScript/TSX's typed declaration symbols, then reuses the
    /// syntax-indexed fact pass for imports, type references, identifier
    /// usages, and definitions that do not yet have a richer symbol kind
    /// (such as type aliases and enums).
    fn process_typescript(
        &mut self,
        artifact: &Artifact,
        analysis: TypeScriptAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        let module_id = self.module(
            artifact.path.as_str(),
            ModuleLanguage::TypeScript(analysis.language),
            file_evidence(artifact),
        );
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        let language_id = analysis.language.registry_id();
        // A name can legitimately identify several methods in different
        // classes. Keeping every candidate lets us resolve only singleton
        // same-file names and leave ambiguous calls for the conservative
        // post-build resolver instead of choosing a plausible-but-wrong one.
        let mut callable_ids: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();
        for class in &analysis.classes {
            let qualified = format!("{}::{}", artifact.path, class.name);
            let class_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Class,
                qualified_name: qualified,
                doc: None,
                evidence: class.evidence.clone(),
            }));
            self.relate_with_provenance(
                artifact_node.clone(),
                class_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![class.evidence.clone()],
                Some(format_provenance(
                    language_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            for method in &class.methods {
                let qualified = format!("{}::{}::{}", artifact.path, class.name, method.name);
                let method_id = self.insert(GraphNode::Symbol(SymbolNode {
                    id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                    kind: SymbolKind::Method,
                    qualified_name: qualified,
                    doc: None,
                    evidence: method.evidence.clone(),
                }));
                self.relate_with_provenance(
                    class_id.clone(),
                    method_id.clone(),
                    RelationKind::Contains,
                    Confidence::High,
                    vec![method.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::SyntaxOnly,
                        Confidence::High,
                    )),
                );
                callable_ids
                    .entry(method.name.clone())
                    .or_default()
                    .push(method_id);
            }
        }

        for function in &analysis.functions {
            let qualified = format!("{}::{}", artifact.path, function.name);
            let function_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Function,
                qualified_name: qualified,
                doc: None,
                evidence: function.evidence.clone(),
            }));
            self.relate_with_provenance(
                artifact_node.clone(),
                function_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
                Some(format_provenance(
                    language_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            callable_ids
                .entry(function.name.clone())
                .or_default()
                .push(function_id);
        }

        for call in &analysis.calls {
            if let Some([target]) = callable_ids.get(call.name.as_str()).map(Vec::as_slice) {
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target.clone(),
                    RelationKind::Calls,
                    Confidence::High,
                    vec![call.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::HybridResolved,
                        Confidence::High,
                    )),
                );
            } else {
                let target = self.unresolved(&call.name);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Calls,
                    Confidence::Low,
                    vec![call.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
        }

        self.process_syntax_indexed_facts(
            artifact,
            match analysis.language {
                TypeScriptLanguage::TypeScript => SyntaxIndexedLanguage::TypeScript,
                TypeScriptLanguage::Tsx => SyntaxIndexedLanguage::Tsx,
            },
            analysis.syntax,
            artifact_node,
            &[
                "class_declaration",
                "abstract_class_declaration",
                "function_declaration",
                "generator_function_declaration",
                "method_definition",
            ],
        );
    }

    fn process_cargo(&mut self, profile: CargoProfile, artifact_node: &GraphNodeId) {
        let Some(package) = &profile.package else {
            return;
        };
        let Some(name) = &package.name else { return };
        let package_id = self.package(name, false);
        self.relate_with_provenance(
            artifact_node.clone(),
            package_id.clone(),
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![package.evidence.clone()],
            Some(format_provenance(
                "toml",
                RelationResolution::SyntaxOnly,
                Confidence::High,
            )),
        );
        for dependency in &profile.dependencies {
            let dependency_id = self.package(&dependency.name, true);
            self.relate_with_provenance(
                package_id.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence.clone()],
                Some(format_provenance(
                    "toml",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }

    fn process_pyproject(&mut self, profile: PyProjectProfile, artifact_node: &GraphNodeId) {
        let Some(project) = &profile.project else {
            return;
        };
        let Some(name) = &project.name else { return };
        let package_id = self.package(name, false);
        self.relate_with_provenance(
            artifact_node.clone(),
            package_id.clone(),
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![project.evidence.clone()],
            Some(format_provenance(
                "toml",
                RelationResolution::SyntaxOnly,
                Confidence::High,
            )),
        );
        for dependency in &project.dependencies {
            let dependency_name = python_dependency_name(&dependency.requirement);
            let dependency_id = self.package(dependency_name, true);
            self.relate_with_provenance(
                package_id.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence.clone()],
                Some(format_provenance(
                    "toml",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }

    fn process_requirements(&mut self, profile: RequirementsProfile, artifact_node: &GraphNodeId) {
        for requirement in &profile.requirements {
            let dependency_id = self.package(&requirement.name, true);
            self.relate_with_provenance(
                artifact_node.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![requirement.evidence.clone()],
                Some(format_provenance(
                    "requirements-txt",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }

    /// Resolves a [`PackageManifestAnalysis`] (LIT-22.2.4) into a local
    /// `Package` node (when the format declares one) and one external
    /// `Package` node per dependency, mirroring `process_cargo`/
    /// `process_pyproject`. Dependencies attach to the local package node
    /// when one exists (so `DependsOnPackage` reads package-to-package, like
    /// Cargo/pyproject), falling back to the artifact node otherwise (e.g.
    /// Gradle, which has no in-file local package name).
    fn process_package_manifest(
        &mut self,
        format: PackageManifestFormat,
        analysis: PackageManifestAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        let format_id = format.format_id();
        let local_package_id = analysis.local_package.map(|local| {
            let package_id = self.package(&local.name, false);
            self.relate_with_provenance(
                artifact_node.clone(),
                package_id.clone(),
                RelationKind::BelongsToPackage,
                Confidence::High,
                vec![local.evidence],
                Some(format_provenance(
                    format_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            package_id
        });

        for dependency in analysis.dependencies {
            let dependency_id = self.package(&dependency.name, true);
            let source = local_package_id
                .clone()
                .unwrap_or_else(|| artifact_node.clone());
            self.relate_with_provenance(
                source,
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence],
                Some(format_provenance(
                    format_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }

    /// Turns gRPC/protobuf `service.rpc` and GraphQL `Query`/`Mutation`
    /// field declarations (LIT-22.3.4 AC1/AC2) into first-class `Route`
    /// config nodes, the same node kind Python's route decorators produce
    /// (see `process_python_route_decorators`), so both surface uniformly
    /// in `KnowledgeIndex::architecture()`'s service links (AC3).
    fn process_protocol_routes(
        &mut self,
        artifact: &Artifact,
        routes: &[ProtocolRoute],
        artifact_node: &GraphNodeId,
    ) {
        for (index, route) in routes.iter().enumerate() {
            let key = format!("route.{index}");
            let route_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Route,
                &route.name,
                route.evidence.clone(),
            );
            self.relate_with_provenance(
                artifact_node.clone(),
                route_id,
                RelationKind::Contains,
                Confidence::High,
                vec![route.evidence.clone()],
                Some(artifact_provenance(
                    artifact,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
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
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::Command => {
                    let target = self.command(
                        artifact,
                        &finding.line.to_string(),
                        &finding.value,
                        evidence.clone(),
                    );
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::LocalPath => {
                    let target = self
                        .resolve_path(&finding.value)
                        .unwrap_or_else(|| self.unresolved(&finding.value));
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::Url
                | TextFindingKind::PackageOrImage
                | TextFindingKind::ImportOrInclude => {
                    let target = self.unresolved(&finding.value);
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::Section => {}
            }
        }
    }

    /// Emits `SimilarTo` relations between near-clone function/method pairs
    /// (LIT-22.3.6 AC2): deterministic Jaccard similarity over each
    /// symbol's lowercase word-token bag, read from its own evidence span
    /// -- never live embeddings or any external ranking service (AC3;
    /// semantic ranking stays a separate, later search concern).
    // ponytail: O(n^2) pairwise comparison over every function/method
    // symbol in the repo. Fine for a "minimum deterministic" baseline at
    // the scale this tool targets; if a very large repo makes this slow,
    // bucket candidates by token-count or line-count range first.
    fn detect_near_clones(
        &mut self,
        repo_root: &Path,
        detail: &GraphBuildTraceDetail,
        selectors: &[String],
    ) -> CloneDiagnostics {
        const MIN_BODY_LINES: u32 = 3;
        const SIMILAR_THRESHOLD: f64 = 0.6;
        const TRACE_NEAR_THRESHOLD: f64 = 0.35;
        const HIGH_CONFIDENCE_THRESHOLD: f64 = 0.85;

        let mut file_cache: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut candidates = Vec::new();
        for node in self.nodes.values() {
            let GraphNode::Symbol(symbol) = node else {
                continue;
            };
            if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            let Some(span) = &symbol.evidence.span else {
                continue;
            };
            if span.end_line.saturating_sub(span.start_line) + 1 < MIN_BODY_LINES {
                continue;
            }
            let path = symbol.evidence.path.as_str().to_owned();
            let text = file_cache
                .entry(path.clone())
                .or_insert_with(|| fs::read_to_string(repo_root.join(&path)).ok());
            let Some(text) = text else {
                continue;
            };
            let tokens = word_tokens(text, span.start_line, span.end_line);
            if tokens.is_empty() {
                continue;
            }
            let language = Path::new(symbol.evidence.path.as_str())
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            let size_band = usize::BITS - tokens.len().max(1).leading_zeros() - 1;
            candidates.push(CloneCandidate {
                id: symbol.id.clone(),
                evidence: symbol.evidence.clone(),
                tokens,
                language,
                size_band,
            });
        }

        let mut diagnostics = CloneDiagnostics {
            candidate_count: candidates.len() as u64,
            ..CloneDiagnostics::default()
        };
        let total_pairs = candidates
            .len()
            .saturating_mul(candidates.len().saturating_sub(1))
            / 2;
        let mut buckets = BTreeMap::<(String, u32), Vec<usize>>::new();
        for (index, candidate) in candidates.iter().enumerate() {
            buckets
                .entry((candidate.language.clone(), candidate.size_band))
                .or_default()
                .push(index);
        }
        let mut pairs = BTreeSet::new();
        for ((language, band), left) in &buckets {
            for candidate_band in [*band, band.saturating_add(1)] {
                let Some(right) = buckets.get(&(language.clone(), candidate_band)) else {
                    continue;
                };
                for &i in left {
                    for &j in right {
                        if i == j {
                            continue;
                        }
                        let smaller = candidates[i].tokens.len().min(candidates[j].tokens.len());
                        let larger = candidates[i].tokens.len().max(candidates[j].tokens.len());
                        if (smaller as f64) / (larger as f64) >= SIMILAR_THRESHOLD {
                            pairs.insert((i.min(j), i.max(j)));
                        }
                    }
                }
            }
        }
        diagnostics.pruned_count = total_pairs.saturating_sub(pairs.len()) as u64;
        for (i, j) in pairs {
            diagnostics.comparison_count += 1;
            let left = &candidates[i];
            let right = &candidates[j];
            let similarity = jaccard_similarity(&left.tokens, &right.tokens);
            let should_trace = (*detail == GraphBuildTraceDetail::Full && selectors.is_empty())
                || selectors.iter().any(|selector| {
                    left.id.as_str().contains(selector)
                        || right.id.as_str().contains(selector)
                        || left.evidence.path.as_str().contains(selector)
                        || right.evidence.path.as_str().contains(selector)
                });
            if similarity < SIMILAR_THRESHOLD {
                if similarity >= TRACE_NEAR_THRESHOLD {
                    diagnostics.rejected_near_threshold_count += 1;
                    if should_trace {
                        diagnostics.decisions.push(clone_decision(
                            &left.id,
                            &right.id,
                            &left.evidence,
                            &right.evidence,
                            similarity,
                            "rejected",
                            "exact Jaccard score was below the configured emission threshold",
                        ));
                    }
                }
                continue;
            }
            diagnostics.emitted_count += 1;
            if should_trace {
                diagnostics.decisions.push(clone_decision(
                    &left.id,
                    &right.id,
                    &left.evidence,
                    &right.evidence,
                    similarity,
                    "emitted",
                    "exact Jaccard score met the configured emission threshold",
                ));
            }
            let confidence = if similarity >= HIGH_CONFIDENCE_THRESHOLD {
                Confidence::High
            } else {
                Confidence::Low
            };
            let (source, target) = if left.id <= right.id {
                (left.id.clone(), right.id.clone())
            } else {
                (right.id.clone(), left.id.clone())
            };
            self.relate_with_provenance(
                source,
                target,
                RelationKind::SimilarTo,
                confidence,
                vec![left.evidence.clone(), right.evidence.clone()],
                Some(RelationProvenance {
                    language: None,
                    resolver_strategy: "lexical-jaccard-similarity".to_owned(),
                    resolution: RelationResolution::HybridResolved,
                    confidence,
                }),
            );
        }
        diagnostics
    }

    /// Produces a deterministic read-only checkpoint without consuming state.
    fn snapshot(&self) -> Graph {
        let mut nodes: Vec<GraphNode> = self.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.id().cmp(b.id()));
        let mut relations = self.relations.clone();
        relations
            .sort_by(|a, b| (&a.source, a.kind, &a.target).cmp(&(&b.source, b.kind, &b.target)));
        Graph { nodes, relations }
    }

    fn finish(self) -> Graph {
        self.snapshot()
    }
}

/// Lowercase word-shaped tokens (letters/digits/underscore runs longer
/// than one character) from `text`'s `start_line..=end_line` (one-based,
/// inclusive) -- deterministic lexical content for near-clone comparison
/// (LIT-22.3.6 AC2). Single-character tokens (`x`, `_`) are dropped:
/// they're common enough to dominate the Jaccard score without indicating
/// real similarity.
fn word_tokens(text: &str, start_line: u32, end_line: u32) -> BTreeSet<String> {
    let start = start_line.saturating_sub(1) as usize;
    let end = end_line as usize;
    text.lines()
        .enumerate()
        .filter(|(index, _)| *index >= start && *index < end)
        .flat_map(|(_, line)| line.split(|ch: char| !ch.is_alphanumeric() && ch != '_'))
        .filter(|token| token.len() > 1)
        .map(str::to_lowercase)
        .collect()
}

/// `|intersection| / |union|`, `0.0` when both sets are empty.
fn jaccard_similarity(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f64 {
    let union = a.union(b).count();
    if union == 0 {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    intersection as f64 / union as f64
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

fn syntax_fact_evidence(artifact: &Artifact, span: crate::domain::SourceSpan) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone()).with_span(span)
}

fn artifact_provenance(
    artifact: &Artifact,
    resolution: RelationResolution,
    confidence: Confidence,
) -> RelationProvenance {
    let language = artifact.detected_format.clone();
    let resolver_strategy = language
        .as_deref()
        .and_then(registry_language)
        .map(|entry| entry.resolver_strategy.to_owned())
        .unwrap_or_else(|| match resolution {
            RelationResolution::Fallback => "generic-text-fallback".to_owned(),
            RelationResolution::SyntaxOnly => "syntax-extraction".to_owned(),
            RelationResolution::HybridResolved => "hybrid-resolution".to_owned(),
        });
    RelationProvenance {
        language,
        resolver_strategy,
        resolution,
        confidence,
    }
}

fn format_provenance(
    language: &str,
    resolution: RelationResolution,
    confidence: Confidence,
) -> RelationProvenance {
    let resolver_strategy = registry_language(language)
        .map(|entry| entry.resolver_strategy.to_owned())
        .unwrap_or_else(|| "syntax-extraction".to_owned());
    RelationProvenance {
        language: Some(language.to_owned()),
        resolver_strategy,
        resolution,
        confidence,
    }
}

fn environment_provenance(source: FactSourceKind, confidence: Confidence) -> RelationProvenance {
    RelationProvenance {
        language: Some("environment".to_owned()),
        resolver_strategy: format!("environment-fact-{source:?}"),
        resolution: RelationResolution::SyntaxOnly,
        confidence,
    }
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::GraphBuilder;
    use crate::analysis::AnalysisCache;
    use crate::domain::Confidence;
    use crate::graph::GraphValidator;
    use crate::graph::{
        GRAPH_BUILD_PASS_ORDER, GRAPH_BUILD_PIPELINE_VERSION, GraphBuildPass,
        GraphBuildTraceConfig, GraphBuildTraceDetail, GraphNode, Relation, RelationKind,
        RelationResolution, SymbolKind,
    };
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
    fn build_report_exposes_deterministic_typed_passes() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let first = GraphBuilder.build_with_report(&root, &artifacts, None);
        let second = GraphBuilder.build_with_report(&root, &artifacts, None);

        assert_eq!(first.pipeline_version, GRAPH_BUILD_PIPELINE_VERSION);
        assert_eq!(
            first
                .passes
                .iter()
                .map(|pass| pass.pass)
                .collect::<Vec<_>>(),
            GRAPH_BUILD_PASS_ORDER
        );
        assert_eq!(first.graph, second.graph);
        assert_eq!(first.passes, second.passes);
        assert_eq!(
            first.trace.as_ref().map(|value| {
                value
                    .stages
                    .iter()
                    .map(|stage| {
                        (
                            stage.pass,
                            &stage.graph_hash,
                            &stage.counters,
                            &stage.decisions,
                        )
                    })
                    .collect::<Vec<_>>()
            }),
            second.trace.as_ref().map(|value| {
                value
                    .stages
                    .iter()
                    .map(|stage| {
                        (
                            stage.pass,
                            &stage.graph_hash,
                            &stage.counters,
                            &stage.decisions,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        );
        assert_eq!(first.graph, GraphBuilder.build(&root, &artifacts));
        Ok(())
    }

    #[test]
    fn opt_in_trace_is_deterministic_and_does_not_change_graph_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let baseline = GraphBuilder.build(&root, &artifacts);
        let config = GraphBuildTraceConfig {
            detail: GraphBuildTraceDetail::Full,
            selectors: vec!["service.py".to_owned(), "service.py".to_owned()],
        };
        let first = GraphBuilder.build_with_trace(&root, &artifacts, None, config.clone());
        let second = GraphBuilder.build_with_trace(&root, &artifacts, None, config);
        let trace = first.trace.as_ref().ok_or("missing trace")?;

        assert_eq!(first.graph, baseline);
        assert_eq!(first.graph, second.graph);
        assert_eq!(first.passes, second.passes);
        assert_eq!(
            first.trace.as_ref().map(|value| {
                value
                    .stages
                    .iter()
                    .map(|stage| {
                        (
                            stage.pass,
                            &stage.graph_hash,
                            &stage.counters,
                            &stage.decisions,
                        )
                    })
                    .collect::<Vec<_>>()
            }),
            second.trace.as_ref().map(|value| {
                value
                    .stages
                    .iter()
                    .map(|stage| {
                        (
                            stage.pass,
                            &stage.graph_hash,
                            &stage.counters,
                            &stage.decisions,
                        )
                    })
                    .collect::<Vec<_>>()
            })
        );
        assert_eq!(trace.selectors, vec!["service.py"]);
        assert_eq!(trace.stages.len(), GRAPH_BUILD_PASS_ORDER.len());
        assert_eq!(
            trace
                .stages
                .iter()
                .map(|stage| stage.pass)
                .collect::<Vec<_>>(),
            GRAPH_BUILD_PASS_ORDER
        );
        assert!(trace.stages.iter().all(|stage| stage.graph.is_some()));
        let enrichment = trace
            .stages
            .iter()
            .find(|stage| stage.pass == GraphBuildPass::Enrichment)
            .ok_or("missing enrichment trace")?;
        assert!(enrichment.counters.contains_key("clone_comparisons"));
        let resolution = trace
            .stages
            .iter()
            .find(|stage| stage.pass == GraphBuildPass::Resolution)
            .ok_or("missing resolution trace")?;
        assert!(resolution.decisions.iter().all(|decision| {
            decision.source.contains("service.py")
                || decision.target.contains("service.py")
                || decision
                    .evidence_paths
                    .iter()
                    .any(|path| path.contains("service.py"))
        }));
        assert!(
            resolution
                .decisions
                .iter()
                .any(|decision| decision.outcome == "selected")
        );
        Ok(())
    }

    #[test]
    fn full_trace_explains_near_threshold_clone_rejection() -> Result<(), Box<dyn std::error::Error>>
    {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("pairs.py"),
            "def alpha(value):\n    total = value\n    clean = strip(total)\n    return clean\n\ndef beta(value):\n    total = value\n    dirty = encode(total)\n    return dirty\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let output = GraphBuilder.build_with_trace(
            repo.path(),
            &artifacts,
            None,
            GraphBuildTraceConfig {
                detail: GraphBuildTraceDetail::Full,
                selectors: Vec::new(),
            },
        );
        let enrichment = output
            .trace
            .as_ref()
            .and_then(|trace| {
                trace
                    .stages
                    .iter()
                    .find(|stage| stage.pass == GraphBuildPass::Enrichment)
            })
            .ok_or("missing enrichment trace")?;
        assert!(
            enrichment.decisions.iter().any(|decision| {
                decision.kind == "near_clone" && decision.outcome == "rejected"
            })
        );
        Ok(())
    }

    #[test]
    fn clone_candidate_bands_prune_representative_pair_growth()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        let mut source = String::new();
        for index in 0..64usize {
            source.push_str(&format!(
                "def generated_{index}(value):\n    total = value\n"
            ));
            for token in 0..(1usize << (index % 6)) {
                source.push_str(&format!("    total += unique_{index}_{token}\n"));
            }
            source.push_str("    return total\n\n");
        }
        std::fs::write(repo.path().join("generated.py"), source)?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let output = GraphBuilder.build_with_trace(
            repo.path(),
            &artifacts,
            None,
            GraphBuildTraceConfig::default(),
        );
        let enrichment = output
            .trace
            .as_ref()
            .and_then(|trace| {
                trace
                    .stages
                    .iter()
                    .find(|stage| stage.pass == GraphBuildPass::Enrichment)
            })
            .ok_or("missing enrichment trace")?;
        let comparisons = enrichment.counters["clone_comparisons"];
        let total_pairs = 64 * 63 / 2;
        assert!(comparisons < total_pairs / 2);
        assert!(enrichment.counters["clone_pruned"] > comparisons);
        Ok(())
    }

    #[test]
    fn python_cross_file_calls_resolve_to_symbols() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("worker.py"),
            "def exported():\n    return 1\n",
        )?;
        std::fs::write(
            repo.path().join("app.py"),
            "from worker import exported\n\ndef start():\n    return exported()\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);

        let target = "symbol:worker.py#worker::exported";
        assert!(
            graph.relations.iter().any(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.target.as_str() == target
                    && relation.provenance.as_ref().is_some_and(|provenance| {
                        provenance.resolution == RelationResolution::HybridResolved
                    })
            }),
            "missing resolved call target {target}"
        );
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
    fn relations_store_language_and_resolution_provenance() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let python_import = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/python_app/__init__.py"
                    && relation.target.as_str() == "module:src.python_app.service"
            })
            .ok_or_else(|| std::io::Error::other("missing python import relation"))?;
        let provenance = python_import
            .provenance
            .as_ref()
            .ok_or_else(|| std::io::Error::other("missing python import provenance"))?;
        assert_eq!(provenance.language.as_deref(), Some("python"));
        assert_eq!(provenance.resolution, RelationResolution::HybridResolved);
        assert_eq!(provenance.resolver_strategy, "specialized-hybrid");
        assert_eq!(provenance.confidence, python_import.confidence);

        // App.tsx is now syntax-indexed (LIT-22.2.3): its import is a
        // syntax-only fact, not a generic-text fallback finding.
        let tsx_import = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:web/src/App.tsx"
                    && relation.provenance.as_ref().is_some_and(|provenance| {
                        provenance.language.as_deref() == Some("tsx")
                            && provenance.resolution == RelationResolution::SyntaxOnly
                    })
            })
            .ok_or_else(|| std::io::Error::other("missing tsx syntax-indexed import relation"))?;
        assert_eq!(tsx_import.confidence, crate::domain::Confidence::Low);

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

        // Known stdlib imports resolve to shared external Package nodes
        // rather than per-file Unresolved noise.
        assert!(has_node("package:os"), "expected package:os");
        assert!(
            has_node("package:__future__"),
            "expected package:__future__"
        );
        assert!(has_node("package:std"), "expected package:std");
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

    #[test]
    fn rust_impls_do_not_target_prelude_package_nodes() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("lib.rs"),
            "\
struct Route;

impl Drop for Route {
    fn drop(&mut self) {}
}
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);
        let issues = GraphValidator.validate(&graph, &artifacts);

        assert_eq!(issues, Vec::new());
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Implements
                    && relation.target.as_str() == "unresolved:Drop"),
            "expected the external Drop trait to stay unresolved for Implements"
        );
        assert!(
            !graph.relations.iter().any(|relation| {
                relation.kind == RelationKind::Implements
                    && relation.target.as_str() == "package:Drop"
            }),
            "Implements must not target package nodes"
        );

        Ok(())
    }

    /// LIT-22.2.4 AC1/AC2/AC4: an isolated repo (not the shared polyglot
    /// fixture, to avoid golden-snapshot churn across the rest of the test
    /// suite) exercising every wired package manifest format end to end --
    /// local vs. external `Package` nodes and `DependsOnPackage` edges.
    #[test]
    fn package_manifests_produce_local_and_external_package_nodes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        std::fs::write(
            root.join("package.json"),
            r#"{"name": "acme-web", "version": "1.0.0", "dependencies": {"react": "^18.0.0"}}"#,
        )?;
        std::fs::write(
            root.join("go.mod"),
            "module github.com/acme/svc\n\nrequire github.com/gin-gonic/gin v1.9.1\n",
        )?;
        std::fs::write(
            root.join("composer.json"),
            r#"{"name": "acme/php-app", "require": {"guzzlehttp/guzzle": "^7.0"}}"#,
        )?;
        std::fs::write(
            root.join("pom.xml"),
            "<project><groupId>com.acme</groupId><artifactId>svc</artifactId><version>1.0</version>\
             <dependencies><dependency><groupId>org.apache.commons</groupId>\
             <artifactId>commons-lang3</artifactId><version>3.14.0</version></dependency>\
             </dependencies></project>",
        )?;
        std::fs::write(
            root.join("build.gradle"),
            "dependencies {\n    implementation(\"com.squareup.okhttp3:okhttp:4.12.0\")\n}\n",
        )?;
        std::fs::create_dir_all(root.join("dotnet"))?;
        std::fs::write(
            root.join("dotnet/App.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup>
                <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
            </ItemGroup></Project>"#,
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let graph = GraphBuilder.build(root, &artifacts);

        let expectations = [
            ("acme-web", false, "react", true),
            (
                "github.com/acme/svc",
                false,
                "github.com/gin-gonic/gin",
                true,
            ),
            ("acme/php-app", false, "guzzlehttp/guzzle", true),
            (
                "com.acme:svc",
                false,
                "org.apache.commons:commons-lang3",
                true,
            ),
            ("com.squareup.okhttp3:okhttp", true, "", false),
            ("App", false, "Newtonsoft.Json", true),
        ];
        for (local_name, local_is_external, dependency_name, has_dependency) in expectations {
            let local = graph
                .nodes
                .iter()
                .find_map(|node| match node {
                    GraphNode::Package(package) if package.name == local_name => Some(package),
                    _ => None,
                })
                .ok_or_else(|| std::io::Error::other(format!("missing package {local_name}")))?;
            assert_eq!(
                local.is_external, local_is_external,
                "{local_name} is_external mismatch"
            );

            if !has_dependency {
                continue;
            }
            let dependency = graph
                .nodes
                .iter()
                .find_map(|node| match node {
                    GraphNode::Package(package) if package.name == dependency_name => Some(package),
                    _ => None,
                })
                .ok_or_else(|| {
                    std::io::Error::other(format!("missing dependency {dependency_name}"))
                })?;
            assert!(dependency.is_external, "{dependency_name} must be external");
            assert!(
                graph.relations.iter().any(|relation| {
                    relation.kind == RelationKind::DependsOnPackage
                        && relation.target == dependency.id
                        && relation
                            .provenance
                            .as_ref()
                            .is_some_and(|p| p.resolution == RelationResolution::SyntaxOnly)
                }),
                "missing DependsOnPackage relation to {dependency_name}"
            );
        }

        Ok(())
    }

    /// LIT-22.3.3 AC1/AC3: a same-file base class resolves to the base
    /// class's own `Symbol` node; a base class defined elsewhere (no
    /// same-file evidence) stays `Unresolved` rather than being guessed.
    #[test]
    fn python_base_classes_produce_inherits_relations() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("models.py"),
            "class Base:\n    pass\n\n\nclass Derived(Base):\n    pass\n\n\nclass External(SomeImportedBase):\n    pass\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let inherits: Vec<&Relation> = graph
            .relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::Inherits)
            .collect();
        assert_eq!(inherits.len(), 2);
        let base_symbol_id = graph
            .nodes
            .iter()
            .find_map(|node| match node {
                GraphNode::Symbol(symbol) if symbol.qualified_name.ends_with("::Base") => {
                    Some(node.id())
                }
                _ => None,
            })
            .ok_or("missing Base symbol node")?;
        assert!(
            inherits
                .iter()
                .any(|relation| &relation.target == base_symbol_id)
        );
        assert!(inherits.iter().any(|relation| {
            graph
                .nodes
                .iter()
                .any(|node| node.id() == &relation.target
                    && matches!(node, GraphNode::Unresolved(unresolved) if unresolved.value == "SomeImportedBase"))
        }));

        Ok(())
    }

    /// LIT-22.3.3 AC1: `extern "C" { ... }` declarations produce `Ffi`
    /// relations to `Unresolved` nodes (the C symbol they name has no
    /// corresponding Rust graph node).
    #[test]
    fn rust_extern_block_produces_ffi_relations() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/lib.rs"),
            "extern \"C\" {\n    fn c_add(a: i32, b: i32) -> i32;\n    static VERSION: i32;\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let ffi: Vec<&Relation> = graph
            .relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::Ffi)
            .collect();
        assert_eq!(ffi.len(), 2);
        for relation in &ffi {
            assert_eq!(relation.confidence, Confidence::High);
            assert!(matches!(
                graph
                    .nodes
                    .iter()
                    .find(|node| node.id() == &relation.target),
                Some(GraphNode::Unresolved(_))
            ));
        }
        assert!(ffi.iter().any(|relation| {
            graph.nodes.iter().any(|node| {
                node.id() == &relation.target
                    && matches!(node, GraphNode::Unresolved(u) if u.value == "c_add")
            })
        }));

        Ok(())
    }

    /// LIT-22.3.3 AC1/AC2: syntax-indexed languages (LIT-22.2.3) produce
    /// `TypeRefs` for `type_identifier` facts and `Usages` for other
    /// identifier facts, deduplicated per file.
    #[test]
    fn syntax_indexed_symbols_produce_type_refs_and_usages()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("widget.ts"),
            "class Widget {\n    hello(): void {\n        console.log(this);\n    }\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::TypeRefs)
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Usages)
        );
        for relation in graph.relations.iter().filter(|relation| {
            matches!(relation.kind, RelationKind::TypeRefs | RelationKind::Usages)
        }) {
            assert_eq!(relation.confidence, Confidence::Low);
            assert_eq!(
                relation
                    .provenance
                    .as_ref()
                    .ok_or("missing provenance")?
                    .resolution,
                RelationResolution::SyntaxOnly
            );
        }

        Ok(())
    }

    /// LIT-23.5: TypeScript's specialized analyzer emits named typed
    /// symbols while its tree-sitter imports, type references, and generic
    /// usages remain part of the graph.
    #[test]
    fn typescript_deep_analysis_keeps_syntax_facts_and_typed_symbols()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.ts"),
            "import type { Config } from \"./types\";\nexport class Service {\n  run(config: Config): void {}\n}\nexport const start = (config: Config) => new Service().run(config);\n",
        )?;
        std::fs::write(temp.path().join("types.ts"), "export type Config = {};\n")?;
        std::fs::write(
            temp.path().join("App.tsx"),
            "export function App() { return <main />; }\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let typed_symbols: Vec<(&str, SymbolKind)> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Symbol(symbol)
                    if matches!(
                        symbol.qualified_name.as_str(),
                        "service.ts::Service"
                            | "service.ts::Service::run"
                            | "service.ts::start"
                            | "App.tsx::App"
                    ) =>
                {
                    Some((symbol.qualified_name.as_str(), symbol.kind))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            typed_symbols,
            vec![
                ("App.tsx::App", SymbolKind::Function),
                ("service.ts::Service", SymbolKind::Class),
                ("service.ts::Service::run", SymbolKind::Method),
                ("service.ts::start", SymbolKind::Function),
            ]
        );
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Imports
                && relation.source.as_str() == "artifact:service.ts"
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::TypeRefs
                && relation.source.as_str() == "artifact:service.ts"
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Usages
                && relation.source.as_str() == "artifact:service.ts"
        }));

        Ok(())
    }

    /// LIT-23.6: direct TypeScript calls resolve only to a unique local
    /// callable symbol. Missing and ambiguous names remain low-confidence
    /// unresolved calls rather than being assigned a plausible target.
    #[test]
    fn typescript_same_file_calls_resolve_conservatively() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.ts"),
            "function helper() {}\nclass First { run() {} }\nclass Second { run() {} }\nhelper();\nrun();\nmissing();\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let resolved = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.target.as_str() == "symbol:service.ts#service.ts::helper"
            })
            .ok_or("missing resolved same-file call")?;
        assert_eq!(resolved.confidence, Confidence::High);
        assert_eq!(
            resolved
                .provenance
                .as_ref()
                .ok_or("missing call provenance")?
                .resolution,
            RelationResolution::HybridResolved
        );

        for name in ["run", "missing"] {
            let relation = graph
                .relations
                .iter()
                .find(|relation| {
                    relation.kind == RelationKind::Calls
                        && matches!(
                            graph.nodes.iter().find(|node| node.id() == &relation.target),
                            Some(GraphNode::Unresolved(unresolved)) if unresolved.value == name
                        )
                })
                .ok_or("missing unresolved conservative call")?;
            assert_eq!(relation.confidence, Confidence::Low);
            assert_eq!(
                relation
                    .provenance
                    .as_ref()
                    .ok_or("missing unresolved call provenance")?
                    .resolution,
                RelationResolution::SyntaxOnly
            );
        }

        Ok(())
    }

    /// LIT-23.6: a named binding from a relative local TypeScript import
    /// resolves after all artifacts have contributed their typed symbols.
    #[test]
    fn typescript_imported_call_resolves_to_local_export() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("lib.ts"),
            "export function greet(): void {}\n",
        )?;
        std::fs::write(
            temp.path().join("app.ts"),
            "import { greet as say } from \"./lib\";\nsay();\nunknown();\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let relation = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.source.as_str() == "artifact:app.ts"
                    && relation.target.as_str() == "symbol:lib.ts#lib.ts::greet"
            })
            .ok_or("missing imported TypeScript call")?;
        assert_eq!(relation.confidence, Confidence::High);
        let provenance = relation
            .provenance
            .as_ref()
            .ok_or("missing call provenance")?;
        assert_eq!(provenance.resolution, RelationResolution::HybridResolved);
        assert_eq!(
            provenance.resolver_strategy,
            "typescript-import-binding-call"
        );

        let unknown = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.source.as_str() == "artifact:app.ts"
                    && matches!(
                        graph.nodes.iter().find(|node| node.id() == &relation.target),
                        Some(GraphNode::Unresolved(unresolved)) if unresolved.value == "unknown"
                    )
            })
            .ok_or("missing unresolved imported-file call")?;
        assert_eq!(unknown.confidence, Confidence::Low);

        Ok(())
    }

    /// LIT-22.3.4 AC1/AC4: a route-decorated handler produces a `Route`
    /// config node; a literal HTTP client call produces a high-confidence
    /// reference; a *dynamic* call target (an f-string, not a literal)
    /// stays low-confidence rather than being reported as if resolved.
    #[test]
    fn python_http_routes_and_calls_are_first_class_graph_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "import requests\n\n\n@app.get(\"/users/{id}\")\ndef get_user(id, dynamic_url):\n    requests.get(\"https://auth.example.test/verify\")\n    requests.get(dynamic_url)\n    return None\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let route = graph
            .nodes
            .iter()
            .find_map(|node| match node {
                GraphNode::Config(config) if config.name == "GET /users/{id}" => Some(config),
                _ => None,
            })
            .ok_or("missing route config node")?;
        assert_eq!(route.kind, crate::graph::model::ConfigNodeKind::Route);
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Contains && relation.target == route.id
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::HandlesRoute
                && relation.source == route.id
                && matches!(
                    graph.nodes.iter().find(|node| node.id() == &relation.target),
                    Some(GraphNode::Symbol(symbol)) if symbol.qualified_name == "service::get_user"
                )
                && relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolution == RelationResolution::HybridResolved
                        && provenance.language.as_deref() == Some("python")
                })
        }));

        let literal_call = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::References
                    && graph.nodes.iter().any(|node| {
                        node.id() == &relation.target
                            && matches!(node, GraphNode::Unresolved(u) if u.value == "https://auth.example.test/verify")
                    })
            })
            .ok_or("missing literal HTTP call relation")?;
        assert_eq!(literal_call.confidence, Confidence::High);

        let dynamic_call = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::References && graph.nodes.iter().any(|node| {
                    node.id() == &relation.target
                        && matches!(node, GraphNode::Unresolved(u) if u.value.contains("dynamic"))
                })
            })
            .ok_or("missing dynamic HTTP call relation")?;
        assert_eq!(dynamic_call.confidence, Confidence::Low);

        Ok(())
    }

    /// LIT-22.3.4 AC2: `.proto` and `.graphql` schema facts produce `Route`
    /// config nodes.
    #[test]
    fn proto_and_graphql_schemas_produce_route_config_nodes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("api.proto"),
            "service Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply) {}\n}\n",
        )?;
        std::fs::write(
            temp.path().join("schema.graphql"),
            "type Query {\n  user(id: ID!): User\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let route_names: Vec<&str> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Config(config)
                    if config.kind == crate::graph::model::ConfigNodeKind::Route =>
                {
                    Some(config.name.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(route_names.contains(&"Greeter.SayHello"));
        assert!(route_names.contains(&"Query.user"));

        Ok(())
    }

    /// LIT-22.2.5 AC1: files with common syntax errors (unclosed braces,
    /// unterminated strings, malformed JSON) never panic the walker or
    /// graph builder; each broken file still gets an artifact node, and
    /// any symbols a tolerant parser did manage to extract before the
    /// error are Low confidence, never fabricated as fully resolved.
    #[test]
    fn syntax_error_fixture_degrades_gracefully_without_panicking()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/syntax_errors");

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        assert_eq!(artifacts.len(), 3);
        let artifact_paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();
        assert!(artifact_paths.contains(&"broken.py"));
        assert!(artifact_paths.contains(&"broken.json"));
        assert!(artifact_paths.contains(&"broken.rs"));

        // Every broken artifact still got an Artifact graph node: a parse
        // failure degrades what can be extracted from a file, it never
        // drops the file from the graph entirely.
        for path in ["broken.py", "broken.json", "broken.rs"] {
            assert!(
                graph.nodes.iter().any(
                    |node| matches!(node, GraphNode::Artifact(artifact) if artifact.path == path)
                ),
                "missing artifact node for {path}"
            );
        }

        Ok(())
    }

    /// LIT-22.3.5 AC1/AC4: producer (`emit`) and consumer (`on`) calls
    /// become `Emits`/`ListensOn` relations to a shared Unresolved node
    /// per channel name, carrying evidence and confidence.
    #[test]
    fn emit_and_on_calls_produce_emits_and_listens_on_relations()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("realtime.py"),
            "def notify(socket):\n    socket.emit(\"user.updated\", payload)\n\n\ndef handler(socket):\n    socket.on(\"user.updated\", on_update)\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let emits = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::Emits)
            .ok_or("expected an Emits relation")?;
        let listens = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::ListensOn)
            .ok_or("expected a ListensOn relation")?;
        assert_eq!(emits.confidence, crate::domain::Confidence::High);
        assert_eq!(listens.confidence, crate::domain::Confidence::High);
        // Both call sites cite the same literal channel name, so they
        // converge on one shared target node rather than two.
        assert_eq!(emits.target, listens.target);
        assert!(graph.nodes.iter().any(
            |node| matches!(node, GraphNode::Unresolved(node) if node.value == "user.updated")
        ));

        Ok(())
    }

    /// LIT-22.3.6 AC1/AC4: a call that passes an argument to a
    /// locally-defined function produces a `DataFlows` relation to that
    /// function's symbol, distinct from the always-present `Calls` relation.
    #[test]
    fn call_with_argument_produces_a_dataflows_relation() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "def build(x):\n    return x\n\n\ndef run(value):\n    return build(value)\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let data_flows = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::DataFlows)
            .ok_or("expected a DataFlows relation")?;
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Calls
                    && relation.target == data_flows.target)
        );

        Ok(())
    }

    /// LIT-22.3.6 AC2/AC4: two near-identical functions (same shape,
    /// trivially renamed) produce a `SimilarTo` relation via deterministic
    /// lexical similarity; a clearly different function does not pair
    /// with either.
    #[test]
    fn near_identical_functions_produce_a_similar_to_relation()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("clones.py"),
            "\
def calculate_total(items):
    total = 0
    for item in items:
        total += item.price
    return total


def calculate_total_v2(items):
    total = 0
    for item in items:
        total += item.price * 2
    return total


def render_report(data):
    output = []
    for section in data.sections:
        output.append(section.title)
    return \"\\n\".join(output)
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let symbol_id = |name: &str| -> Option<&crate::graph::model::GraphNodeId> {
            graph.nodes.iter().find_map(|node| match node {
                GraphNode::Symbol(symbol) if symbol.qualified_name.ends_with(name) => {
                    Some(&symbol.id)
                }
                _ => None,
            })
        };
        let total_id = symbol_id("calculate_total").ok_or("missing calculate_total symbol")?;
        let total_v2_id =
            symbol_id("calculate_total_v2").ok_or("missing calculate_total_v2 symbol")?;
        let report_id = symbol_id("render_report").ok_or("missing render_report symbol")?;

        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::SimilarTo
                    && ((&relation.source == total_id && &relation.target == total_v2_id)
                        || (&relation.source == total_v2_id && &relation.target == total_id)))
        );
        assert!(!graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::SimilarTo
                && (&relation.source == report_id || &relation.target == report_id)
        }));

        Ok(())
    }

    /// LIT-22.3.6 AC2/AC4: near-clone scoring is a pure deterministic
    /// function of source text -- building the same repository twice
    /// yields byte-identical `SimilarTo` relations, never live/varying
    /// embedding-based scores.
    #[test]
    fn near_clone_detection_is_deterministic_across_repeated_builds()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("clones.py"),
            "def calculate_total(items):\n    total = 0\n    for item in items:\n        total += item.price\n    return total\n\n\ndef calculate_total_v2(items):\n    total = 0\n    for item in items:\n        total += item.price * 2\n    return total\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;

        let first = GraphBuilder.build(temp.path(), &artifacts);
        let second = GraphBuilder.build(temp.path(), &artifacts);

        let similar_relations = |graph: &crate::graph::model::Graph| {
            graph
                .relations
                .iter()
                .filter(|relation| relation.kind == RelationKind::SimilarTo)
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(similar_relations(&first), similar_relations(&second));
        assert!(!similar_relations(&first).is_empty());

        Ok(())
    }

    /// LIT-23.1: a multi-line, type-only TypeScript import resolves through
    /// the hybrid resolver pipeline (wired into `build`/`build_with_cache`)
    /// to a real Artifact target, and the now-orphaned raw-multi-line-text
    /// `Unresolved` node it originally targeted is pruned from the graph
    /// entirely -- reproducing and closing the exact bug found on an
    /// external repository, where such nodes previously lingered with a
    /// value containing the whole raw statement, newlines included.
    #[test]
    fn resolved_multi_line_import_prunes_its_orphaned_unresolved_node()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/types.ts"),
            "export type CameraRig = { fov: number };\nexport type RouteSummary = { name: string };\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import type {\n  CameraRig,\n  RouteSummary,\n} from \"./types\";\nexport function noop(): CameraRig | RouteSummary | null {\n  return null;\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph.nodes.iter().all(|node| match node {
                GraphNode::Unresolved(unresolved) => !unresolved.value.contains('\n'),
                _ => true,
            }),
            "no Unresolved node should retain a raw multi-line import statement as its value"
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Imports
                    && relation.target.as_str() == "artifact:src/types.ts"),
            "the import should still resolve to the real local artifact"
        );

        Ok(())
    }

    /// A relation that stays genuinely unresolved keeps its `Unresolved`
    /// node in the graph -- pruning only removes nodes no relation targets
    /// anymore, never a node a caller might still want to inspect.
    #[test]
    fn unresolved_nodes_still_targeted_by_a_relation_are_not_pruned()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/main/java"))?;
        std::fs::write(
            temp.path().join("src/main/java/App.java"),
            "import com.example.totally.unknown.Widget;\nclass App {}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let relation = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/main/java/App.java"
            })
            .ok_or("missing import relation")?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id() == &relation.target
                    && matches!(node, GraphNode::Unresolved(_))),
            "an unresolvable import's Unresolved node must still be present, not pruned"
        );

        Ok(())
    }

    /// LIT-23.2: CSS class/id selectors are declaration syntax (what a
    /// rule_set is), not references to something else, so they must never
    /// produce `Usages`/`TypeRefs` relations the way a code identifier
    /// use-site does. Confirmed live: a single real-world stylesheet
    /// produced 105 spurious `Usages` relations before this fix.
    #[test]
    fn css_selectors_produce_no_usages_relations() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("styles.css"),
            "#root {\n  height: 100%;\n}\n\n.app-shell {\n  display: flex;\n}\n\n.brand-mark {\n  color: #fff;\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph.nodes.iter().any(
                |node| matches!(node, GraphNode::Artifact(artifact) if artifact.path == "styles.css")
            ),
            "a bare Artifact node must still exist for the CSS file"
        );
        assert!(
            !graph.relations.iter().any(|relation| matches!(
                relation.kind,
                RelationKind::Usages | RelationKind::TypeRefs
            ) && relation.source.as_str()
                == "artifact:styles.css"),
            "CSS selectors must not produce Usages/TypeRefs relations"
        );
        // The rule_set/at_rule structural facts (LIT-22.2.3) are unaffected:
        // each selector's rule still contributes a Symbol via Contains.
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Contains
                    && relation.source.as_str() == "artifact:styles.css"),
            "CSS rule_set definitions should still produce Contains relations"
        );

        Ok(())
    }

    /// LIT-23.3: `package-lock.json`'s internal dependency-tree fields
    /// (`resolved` URLs, `bin` entries, integrity hashes) must not produce
    /// spurious reference/config relations the way hand-written JSON
    /// config would. Confirmed live: a single real-world lockfile produced
    /// 504 spurious relations before this fix.
    #[test]
    fn package_lock_json_produces_no_spurious_reference_relations()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("package-lock.json"),
            r#"{
  "name": "app",
  "lockfileVersion": 3,
  "packages": {
    "": { "dependencies": { "esbuild": "^0.21.0" } },
    "node_modules/esbuild": {
      "version": "0.21.5",
      "resolved": "https://registry.npmjs.org/esbuild/-/esbuild-0.21.5.tgz",
      "integrity": "sha512-abc123==",
      "bin": { "esbuild": "bin/esbuild" },
      "engines": { "node": ">=12" }
    }
  }
}
"#,
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph.nodes.iter().any(
                |node| matches!(node, GraphNode::Artifact(artifact) if artifact.path == "package-lock.json")
            ),
            "a bare Artifact node must still exist for the lockfile"
        );
        assert!(
            !graph
                .relations
                .iter()
                .any(|relation| relation.source.as_str() == "artifact:package-lock.json"),
            "a lockfile must produce no relations at all from its content"
        );

        Ok(())
    }
}
