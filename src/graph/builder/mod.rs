//! Graph construction: merges artifact inventory and per-artifact analyzer
//! output into one typed semantic graph.
//!
//! Split (LIT-41.1) from a single ~5600-line `builder.rs` into this
//! coordinating module plus cohesive per-pass submodules: [`python_lang`]
//! (named to avoid shadowing `crate::analysis::python`),
//! [`rust`], [`typescript`] (per-language artifact processing),
//! [`packages`] (package/config manifest and infra-config relation
//! resolution), [`evidence`] (environment/config-fact and syntax-fact
//! evidence handling), [`clones`] and [`clone_tokens`] (near-clone detection
//! and verification), [`dispatch`] (analyzer selection/caching), and
//! [`materialize`] (final graph snapshot/finish). `BuilderState` and its
//! fields are defined here; every submodule adds further `impl BuilderState`
//! blocks for its own pass. This is a move-only split: each submodule is a
//! descendant module of `graph::builder`, so it can already reach
//! `BuilderState`'s private fields and this module's private helpers
//! directly, per Rust's usual privacy rule (a private item is visible to its
//! defining module and all descendants). Only items moved *out* of this
//! module that it (or a sibling submodule) still calls are marked
//! `pub(super)`.

use crate::analysis::{
    ActionsProfile, ActionsProfileAnalyzer, ActionsStepHint, AnalysisCache, AnalyzerKind,
    AnalyzerOutput, CargoProfile, CargoProfileAnalyzer, ComposeProfile, ComposeProfileAnalyzer,
    ConfigReferenceKind, DockerCommandKind, DockerfileAnalysis, DockerfileAnalyzer,
    EnvironmentFacts, GenericTextExtractor, MarkdownAnalysis, MarkdownAnalyzer,
    PackageManifestAnalysis, PackageManifestFormat, ProtocolFormat, ProtocolRoute,
    PyProjectAnalyzer, PyProjectProfile, PythonAnalysis, PythonAnalyzer, PythonImportKind,
    PythonReferenceKind, RequirementsAnalyzer, RequirementsProfile, RustAnalysis, RustAnalyzer,
    RustReferenceKind, RustWorkspaceAnalysis, RustWorkspaceAnalyzer, StructuredAnalysis,
    StructuredAnalyzer, StructuredFormat, SyntaxIndexedLanguage, TextFinding, TextFindingKind,
    TreeSitterAdapterOutput, TypeScriptAnalysis, TypeScriptAnalyzer, TypeScriptLanguage,
    TypeScriptReExportKind, is_python_stdlib_module, normalize_python_package_name, python,
    rust_source, rust_std_crate,
};
use crate::domain::{
    AnalyzerSelection, Artifact, ArtifactId, Confidence, EvidenceRef, ModelExposurePolicy,
    TextStatus,
};
use crate::graph::model::{
    ArtifactNode, CommandNode, CommandProvenance, ConfigNode, ConfigNodeKind, ContainerImageNode,
    DocumentationNode, EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode,
    PackageNode, Relation, RelationKind, RelationProvenance, RelationResolution, SymbolKind,
    SymbolNode, UnresolvedNode,
};
use crate::graph::{
    GRAPH_BUILD_PIPELINE_VERSION, GraphBuildOutput, GraphBuildPass, GraphBuildTraceConfig,
    GraphBuildTraceDetail, GraphDecisionTrace,
};
use crate::inventory::language::by_name as registry_language;
use crate::resolve::{ConfigFact, EnvFact, FactRole, FactSourceKind};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::time::Instant;

mod clone_tokens;
mod clones;
mod dispatch;
mod evidence;
mod materialize;
mod packages;
mod python_lang;
mod rationale;
mod rust;
mod typescript;

use dispatch::{analyzer_kind, artifact_cache_key, compute_fresh};

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

        // LIT-45.2: read every `tsconfig.json` before indexing any TS file, so
        // an aliased import resolves regardless of artifact walk order. Like
        // the manifest pre-pass below, this deliberately bypasses the analysis
        // cache: these files are small, few, and not analyzed anywhere else,
        // so a cache entry would cost more bookkeeping than the parse.
        state.ts_aliases =
            crate::resolve::TsAliasMap::build(&collect_ts_configs(repo_root, artifacts));

        // Collect this repo's own declared Python dependency names (LIT-44.1)
        // before indexing any Python file, so `python_external_target` can
        // classify a third-party import as a known project dependency
        // regardless of artifact walk order (a `.py` file can sort before
        // `pyproject.toml` in `artifacts`). Deliberately bypasses the
        // analysis cache: these are the same artifacts the main definitions
        // pass below analyzes (and caches) under the same `AnalyzerKind`, so
        // routing this pre-read through `cache.get`/`cache.put` too would
        // double-count every manifest as an extra cache hit without
        // reflecting any real reuse. Manifests are small; re-parsing them
        // once here is cheap.
        for artifact in artifacts {
            let Some(
                kind @ (AnalyzerKind::PyProject
                | AnalyzerKind::Requirements
                | AnalyzerKind::Cargo
                | AnalyzerKind::PackageManifest(PackageManifestFormat::Npm)),
            ) = analyzer_kind(artifact)
            else {
                continue;
            };
            if artifact.text_status != TextStatus::Text
                || artifact.model_policy == ModelExposurePolicy::Never
            {
                continue;
            }
            let Ok(text) = fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
                continue;
            };
            let output = compute_fresh(artifact, &text, repo_root, kind);
            state.register_python_manifest_packages(&output);
            state.register_rust_manifest_packages(&output);
            state.register_javascript_manifest_packages(&output);
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
            cache,
        );
        let clone_duration_us = clone_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        // LIT-39: the enrichment stage is more than clone detection. Time the
        // remaining sub-phases (environment-fact materialization and the final
        // graph assembly) so their component_*_us observations account for
        // stage_enrichment_us instead of being an untimed blind spot.
        let env_facts_started = Instant::now();
        state.materialize_environment_facts();
        let env_facts_us = env_facts_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        // Time final graph assembly *and* the snapshot clone into `output`
        // together: the clone materializes the enrichment-stage graph that
        // `record` hashes, so it is enrichment work, not free (LIT-39).
        let finish_started = Instant::now();
        // LIT-57: `finish` consumes the state, so the propagation facts it
        // accumulated are taken out first; the resolution stage below is the
        // only consumer.
        let type_facts = std::mem::take(&mut state.type_facts);
        let ts_aliases = std::mem::take(&mut state.ts_aliases);
        let mut graph = state.finish();
        output.graph = graph.clone();
        let enrichment_finish_us = finish_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
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
                (
                    "clone_peak_candidate_pairs".to_owned(),
                    clone_diagnostics.peak_candidate_pairs,
                ),
                (
                    "clone_prefilter_pairs".to_owned(),
                    clone_diagnostics.prefilter_pairs,
                ),
                ("clone_cache_hit".to_owned(), clone_diagnostics.cache_hit),
            ]),
            clone_diagnostics.decisions,
        );
        output.record_component_duration(
            GraphBuildPass::Enrichment,
            "clone_detection",
            clone_duration_us,
        );
        // LIT-35.1/35.5 AC4/AC1: attribute the clone bottleneck to its
        // sub-phases so regressions can be localized to tokenization, candidate
        // generation (filtering), exact verification, cache lookup, or merge.
        for (component, duration) in [
            ("clone_tokenization", clone_diagnostics.tokenize_us),
            (
                "clone_candidate_generation",
                clone_diagnostics.candidate_gen_us,
            ),
            (
                "clone_exact_verification",
                clone_diagnostics.exact_verify_us,
            ),
            ("clone_cache_lookup", clone_diagnostics.cache_lookup_us),
            ("clone_merge", clone_diagnostics.merge_us),
            // LIT-39: non-clone enrichment sub-phases so the component timings
            // sum to stage_enrichment_us within a small residual.
            ("environment_fact_materialization", env_facts_us),
            ("enrichment_finish", enrichment_finish_us),
        ] {
            output.record_component_duration(GraphBuildPass::Enrichment, component, duration);
        }
        // LIT-23.1: applies to every caller (init/update, inspect, MCP
        // tools, tests) uniformly, the same way detect_near_clones already
        // does -- a post-processing pass belongs here, not bolted onto one
        // caller, or callers would see inconsistently-resolved graphs.
        let resolution_started = Instant::now();
        let relations_before_resolution = graph.relations.clone();
        // LIT-57: types receivers across files before the name-based
        // resolvers run. It appends `HybridResolved` relations, which the
        // pipeline below never revisits, so the two cannot contend for the
        // same call.
        crate::resolve::propagate_types(&mut graph, &type_facts);
        // LIT-79: the barrel re-export map lets the import-binding resolver
        // chase a use site imported through a barrel (`from '../client'`) to
        // the module that actually declares it. Built from the same per-file
        // facts `propagate_types` uses; empty for repositories with no barrels.
        let re_exports = crate::resolve::re_export_map(&type_facts);
        crate::resolve::HybridResolverPipeline::default_pipeline()
            .resolve_with_aliases_and_re_exports(&mut graph, ts_aliases, re_exports);
        crate::resolve::resolve_environment_links(&mut graph);
        classify_javascript_builtins(&mut graph);
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

/// Removes `Unresolved` nodes no relation references anymore after hybrid
/// resolution (LIT-23.1): when every relation that targeted a raw
/// syntax-only fact gets upgraded to a real node, the placeholder is dead
/// weight -- leaving it in the graph would still surface the very raw-text
/// noise resolution was meant to eliminate, just disconnected from every
/// relation. A node created and immediately shared by several relations
/// (the common case, since `BuilderState::unresolved` deduplicates by id)
/// survives as long as at least one relation still targets it.
/// LIT-77: retargets references to bare JavaScript/TypeScript builtin globals
/// (`Array`, `Blob`, `JSON`, `btoa`, `Record`, ...) from a per-file
/// `Unresolved` node onto a shared external builtin `Symbol` -- the JS
/// analogue of LIT-6's Python-stdlib and LIT-66's Rust-prelude
/// classification. It runs *after* resolution, so a name a local definition
/// or a relative/package import already claimed (LIT-71/75) never reaches
/// here; a name still unresolved at this point is one nothing local or
/// imported explained, which is exactly when a global builtin is the honest
/// classification rather than a guess. Only TS/JS-language Calls, Usages, and
/// type references are eligible -- an import statement never names a global.
fn classify_javascript_builtins(graph: &mut Graph) {
    let unresolved_names: BTreeMap<&GraphNodeId, &str> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Unresolved(node) => Some((&node.id, node.value.as_str())),
            _ => None,
        })
        .collect();
    // Builtin name -> evidence to attach to its interned symbol (from the
    // first eligible reference, in relation order, so it is deterministic).
    let mut builtins: BTreeMap<String, EvidenceRef> = BTreeMap::new();
    let mut retargets: Vec<(usize, String)> = Vec::new();
    for (index, relation) in graph.relations.iter().enumerate() {
        if !matches!(
            relation.kind,
            RelationKind::Calls
                | RelationKind::Usages
                | RelationKind::TypeRefs
                | RelationKind::UsesType
        ) {
            continue;
        }
        let is_js = relation
            .provenance
            .as_ref()
            .and_then(|provenance| provenance.language.as_deref())
            .is_some_and(|language| matches!(language, "typescript" | "tsx" | "javascript"));
        if !is_js {
            continue;
        }
        let Some(name) = unresolved_names.get(&relation.target) else {
            continue;
        };
        if !crate::analysis::is_javascript_builtin(name) {
            continue;
        }
        // Every such reference carries its call/use-site evidence; without it
        // there is nothing to attach to the interned symbol, so leave it be.
        let Some(evidence) = relation.evidence.first() else {
            continue;
        };
        builtins
            .entry((*name).to_owned())
            .or_insert_with(|| evidence.clone());
        retargets.push((index, (*name).to_owned()));
    }
    if retargets.is_empty() {
        return;
    }
    // Intern one external `Symbol` per builtin (`javascript::Array`) plus its
    // `BelongsToPackage` edge to a shared external `javascript` package, the
    // same shape `python_external_symbol` produces for stdlib members.
    let package_id = GraphNodeId::new("package:javascript");
    if !graph.nodes.iter().any(|node| node.id() == &package_id) {
        graph.nodes.push(GraphNode::Package(PackageNode {
            id: package_id.clone(),
            name: "javascript".to_owned(),
            is_external: true,
        }));
    }
    for (name, evidence) in &builtins {
        let symbol_id = GraphNodeId::new(format!("symbol:javascript::{name}"));
        graph.nodes.push(GraphNode::Symbol(SymbolNode {
            id: symbol_id.clone(),
            kind: SymbolKind::External,
            qualified_name: format!("javascript::{name}"),
            doc: None,
            evidence: evidence.clone(),
        }));
        graph.relations.push(Relation {
            id: format!(
                "belongs-to-package:{}:{}",
                symbol_id.as_str(),
                package_id.as_str()
            ),
            source: symbol_id,
            target: package_id.clone(),
            kind: RelationKind::BelongsToPackage,
            confidence: Confidence::High,
            evidence: vec![evidence.clone()],
            provenance: None,
        });
    }
    for (index, name) in retargets {
        graph.relations[index].target = GraphNodeId::new(format!("symbol:javascript::{name}"));
    }
}

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

struct BuilderState {
    nodes: BTreeMap<GraphNodeId, GraphNode>,
    relations: Vec<Relation>,
    relation_count: usize,
    // LIT-40.2: (source, target, kind) keys of existing relations, populated
    // only while materializing environment facts so `relate_if_absent` dedups
    // with a set lookup instead of scanning every relation per fact.
    env_relation_keys: BTreeSet<(GraphNodeId, GraphNodeId, RelationKind)>,
    // LIT-40.1: symbol (start_line, end_line, id) tuples grouped by artifact,
    // populated only while materializing environment facts so
    // `smallest_symbol_owner` queries one file's symbols instead of scanning
    // every node per source-code fact.
    env_symbols_by_artifact: HashMap<ArtifactId, Vec<(u32, u32, GraphNodeId)>>,
    environment_facts: EnvironmentFacts,
    artifact_paths: BTreeSet<String>,
    python_modules: BTreeMap<String, GraphNodeId>,
    /// LIT-76: each Python module also indexed by its *package-relative*
    /// dotted path -- the path from the source root a Python interpreter
    /// would put on `sys.path`, i.e. with the leading non-package directories
    /// stripped. A file `backend/app/core/config.py` in a repo where `app` is
    /// the top-level package (its `__init__.py` sits under `backend/`) is
    /// keyed `app.core.config` here, matching how the code imports it (`from
    /// app.core.config import ...`), while its node identity stays the
    /// whole-repo `module:backend.app.core.config`. A pure resolution
    /// fallback: entries exist only where the package-relative path differs
    /// from the whole-repo one, so it never changes an already-resolvable
    /// import.
    python_package_relative_modules: BTreeMap<String, GraphNodeId>,
    rust_modules: BTreeMap<String, GraphNodeId>,
    /// Repository-relative source root directory of every known Cargo
    /// build target (e.g. `"rust/src"` for a `rust/src/lib.rs` target),
    /// resolved from `cargo metadata` via [`RustWorkspaceAnalyzer`]. Used to
    /// compute a file's true crate-relative module path instead of
    /// `rust_source::module_path`'s naive whole-repo-relative guess.
    rust_crate_roots: BTreeSet<String>,
    /// Crate names that live in this repository (LIT-66).
    rust_local_crates: BTreeSet<String>,
    /// Normalized ([`normalize_python_package_name`]) dependency names
    /// declared by this repo's `pyproject.toml`/`requirements.txt` (LIT-44.1),
    /// populated by a pre-pass before any Python file is indexed so
    /// `python_external_target` can classify a third-party import as a known
    /// project dependency regardless of artifact walk order.
    /// LIT-57: per-file receiver-typing facts, accumulated while each file is
    /// analyzed and consumed once by the cross-file propagation pass after
    /// every symbol exists. They are carried here rather than staged as
    /// `Unresolved` `Calls` edges because an unresolved value like `p.dumps`
    /// would be claimed by `SymbolNameResolver`'s unique simple-name match and
    /// fabricate an edge into whatever `dumps` happened to be unique (LIT-63).
    type_facts: crate::resolve::TypeFacts,
    /// LIT-45.2: tsconfig `compilerOptions.paths` aliases, read once before
    /// any TypeScript file is indexed.
    ts_aliases: crate::resolve::TsAliasMap,
    python_manifest_packages: BTreeSet<String>,
    /// Crate names declared in Cargo.toml (LIT-66).
    rust_manifest_packages: BTreeSet<String>,
    /// npm dependency names declared by this repo's own `package.json`
    /// (LIT-71), populated by the same pre-pass as `python_manifest_packages`
    /// so a bare-package import's usage sites can be classified regardless
    /// of artifact walk order.
    js_manifest_packages: BTreeSet<String>,
}

impl BuilderState {
    fn new(artifacts: &[Artifact]) -> Self {
        Self {
            nodes: BTreeMap::new(),
            relations: Vec::new(),
            relation_count: 0,
            env_relation_keys: BTreeSet::new(),
            env_symbols_by_artifact: HashMap::new(),
            environment_facts: EnvironmentFacts::default(),
            artifact_paths: artifacts
                .iter()
                .map(|artifact| artifact.path.as_str().to_owned())
                .collect(),
            python_modules: BTreeMap::new(),
            python_package_relative_modules: BTreeMap::new(),
            rust_modules: BTreeMap::new(),
            rust_crate_roots: BTreeSet::new(),
            rust_local_crates: BTreeSet::new(),
            type_facts: crate::resolve::TypeFacts::new(),
            ts_aliases: crate::resolve::TsAliasMap::default(),
            python_manifest_packages: BTreeSet::new(),
            rust_manifest_packages: BTreeSet::new(),
            js_manifest_packages: BTreeSet::new(),
        }
    }
    /// Records each resolved Cargo target's source root directory, so
    /// [`Self::rust_module_path`] can compute crate-relative module paths.
    /// Safe to call more than once for the same workspace (a `BTreeSet`).
    fn register_rust_crate_roots(&mut self, workspace: &RustWorkspaceAnalysis) {
        for package in &workspace.packages {
            // LIT-66: `cargo metadata` is authoritative about which crates
            // live here. ripgrep declares its own `grep` crate as
            // `{ version = "0.3.2", path = "crates/grep" }`, and the manifest
            // pass records only the version -- so a name-based check would
            // call an in-repository crate a third-party dependency.
            self.rust_local_crates.insert(package.name.clone());
            self.rust_local_crates
                .insert(package.name.replace('-', "_"));
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
    /// The package-relative dotted module path of a Python file: the path
    /// from its source root, found by dropping the leading directories that
    /// are not themselves Python packages. `app` is the top-level package
    /// when `.../app/__init__.py` exists but its parent has no `__init__.py`,
    /// so the source root is that parent and `app` begins the import path.
    /// Returns `None` when the file sits in no package at all (no ancestor
    /// `__init__.py`), where the whole-repo module path is already correct.
    fn python_package_relative_module(&self, artifact_path: &str) -> Option<String> {
        let trimmed = artifact_path.strip_suffix(".py")?;
        // For a package's own `__init__.py`, the module is the package
        // directory chain itself; for a plain module, drop the file name.
        let chain = trimmed.strip_suffix("/__init__").unwrap_or(trimmed);
        let segments: Vec<&str> = chain.split('/').collect();
        // The first prefix that is a package directory (has `__init__.py`)
        // starts the import path; everything before it is the source root.
        let package_start = (0..segments.len()).find(|&end| {
            let dir = segments[..=end].join("/");
            self.artifact_paths.contains(&format!("{dir}/__init__.py"))
        })?;
        (package_start > 0).then(|| segments[package_start..].join("."))
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
        // LIT-40.2: dedup via the key set seeded in `materialize_environment_facts`
        // (the only caller), replacing the per-fact scan of every relation.
        let key = (source.clone(), target.clone(), kind);
        if !self.env_relation_keys.insert(key) {
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
                // LIT-76: also index the module under the package-relative
                // path the code actually imports, when a source-root prefix
                // was stripped (`backend.app.core.config` -> `app.core.config`).
                if let Some(relative) = self.python_package_relative_module(artifact.path.as_str())
                    && relative != module_path
                {
                    self.python_package_relative_modules
                        .insert(relative, id.clone());
                }
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
        let provenance = command_provenance(artifact, &evidence);
        self.command_with_provenance(artifact, key, text, evidence, provenance)
    }
    fn command_with_provenance(
        &mut self,
        artifact: &Artifact,
        key: &str,
        text: &str,
        evidence: EvidenceRef,
        provenance: CommandProvenance,
    ) -> GraphNodeId {
        self.insert(GraphNode::Command(CommandNode {
            id: GraphNodeId::new(format!("command:{}#{key}", artifact.path)),
            text: text.to_owned(),
            provenance,
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
    /// a known stdlib module, or a module whose top-level segment matches a
    /// dependency this repo's own `pyproject.toml`/`requirements.txt`
    /// declares (LIT-44.1), becomes a shared external `Package` node (one
    /// per module name, deduplicated across the whole repo) instead of a
    /// per-file `Unresolved` node. Anything else -- a genuinely unknown or
    /// undeclared third-party module -- still becomes `Unresolved`.
    /// Interns the symbol `name` imported from the external module
    /// `module`, plus the `BelongsToPackage` edge tying it to its package
    /// node.
    ///
    /// LIT-56: relation kinds like `Calls`, `Decorates`, and `UsesType`
    /// accept only `Symbol` targets -- you call a package's member, never the
    /// package. Pointing them at `package:<name>` produced graphs
    /// `GraphValidator` rejects, which failed `init` outright on any
    /// repository calling an imported name (e.g. `from multiprocessing import
    /// cpu_count`). The symbol node keeps the same knowledge in a shape the
    /// graph's own rules allow, and is more precise besides: `cpu_count`
    /// rather than all of `multiprocessing`.
    fn python_external_symbol(
        &mut self,
        module: &str,
        name: &str,
        evidence: EvidenceRef,
    ) -> GraphNodeId {
        let qualified_name = format!("{module}::{name}");
        let symbol_id = self.insert(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(format!("symbol:{qualified_name}")),
            kind: SymbolKind::External,
            qualified_name,
            doc: None,
            evidence: evidence.clone(),
        }));
        let top_level = module.split('.').next().unwrap_or(module);
        let package_id = self.package(top_level, true);
        self.relate(
            symbol_id.clone(),
            package_id,
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![evidence],
        );
        symbol_id
    }

    fn python_external_target(&mut self, dotted_name: &str) -> GraphNodeId {
        let top_level = dotted_name.split('.').next().unwrap_or(dotted_name);
        if is_python_stdlib_module(dotted_name)
            || self
                .python_manifest_packages
                .contains(&normalize_python_package_name(top_level))
        {
            self.package(top_level, true)
        } else {
            self.unresolved(dotted_name)
        }
    }
    /// LIT-71: TS/JS counterpart of `python_external_symbol`. Interns the
    /// member `name` of the declared npm package `package`, plus its
    /// `BelongsToPackage` edge, so a call/usage/type-reference site that
    /// resolves through `Self::typescript_bare_package_imports` gets the
    /// same shared external-symbol node an `Imports` edge to this package
    /// would already imply, instead of a fresh per-file `Unresolved` node.
    fn typescript_external_symbol(
        &mut self,
        package: &str,
        name: &str,
        evidence: EvidenceRef,
    ) -> GraphNodeId {
        let qualified_name = format!("{package}::{name}");
        let symbol_id = self.insert(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(format!("symbol:{qualified_name}")),
            kind: SymbolKind::External,
            qualified_name,
            doc: None,
            evidence: evidence.clone(),
        }));
        let package_id = self.package(package, true);
        self.relate(
            symbol_id.clone(),
            package_id,
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![evidence],
        );
        symbol_id
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
    /// Resolves a path mentioned by documentation without manufacturing an
    /// `Unresolved` node when it is prose, a broken link, or a quoted code
    /// fragment. Documentation analyzers retain those findings (and Markdown
    /// drift records), while the graph only records references that point to
    /// an inventoried repository artifact.
    fn resolve_documentation_path(&self, artifact: &Artifact, value: &str) -> Option<GraphNodeId> {
        let path = value.split(['#', '?']).next()?.trim();
        if path.is_empty() {
            return None;
        }

        let normalized = if path.starts_with('/') {
            path.trim_start_matches('/').to_owned()
        } else {
            let parent = artifact
                .path
                .as_str()
                .rsplit_once('/')
                .map_or("", |(parent, _)| parent);
            crate::resolve::imports::resolve_relative_path(std::path::Path::new(parent), path)
        };
        self.artifact_paths
            .contains(&normalized)
            .then(|| GraphNodeId::new(format!("artifact:{normalized}")))
            .or_else(|| self.resolve_path(path))
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
}

fn command_provenance(artifact: &Artifact, evidence: &EvidenceRef) -> CommandProvenance {
    let is_documentation_path = evidence.path.as_str().split('/').any(|component| {
        component.eq_ignore_ascii_case("doc")
            || component.eq_ignore_ascii_case("docs")
            || component.eq_ignore_ascii_case("documentation")
    });
    if artifact.category == crate::domain::ArtifactCategory::Documentation || is_documentation_path
    {
        CommandProvenance::DocumentationExample
    } else if matches!(
        artifact.category,
        crate::domain::ArtifactCategory::BuildDefinition
            | crate::domain::ArtifactCategory::ContinuousIntegration
    ) {
        CommandProvenance::BuildAutomation
    } else {
        CommandProvenance::Executable
    }
}

fn artifact_node_id(artifact: &Artifact) -> GraphNodeId {
    GraphNodeId::new(format!("artifact:{}", artifact.path))
}

fn file_evidence(artifact: &Artifact) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone())
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

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// LIT-45.2: every `tsconfig.json` in the repository, plus the configs they
/// extend.
///
/// Discovery starts from `tsconfig*.json` because that is what a repository
/// names its configs. An `extends` target need not follow that convention
/// (`./base.json` is common), so the chain is followed explicitly rather than
/// hoping the name matches -- otherwise inherited `paths` silently vanish.
/// Only files already in `artifacts` are read, so this never reaches outside
/// the scanned tree or into `node_modules`.
fn collect_ts_configs(
    repo_root: &Path,
    artifacts: &[Artifact],
) -> BTreeMap<String, crate::analysis::TsConfigProfile> {
    let readable: BTreeMap<&str, &Artifact> = artifacts
        .iter()
        .filter(|artifact| {
            artifact.path.as_str().ends_with(".json")
                && artifact.text_status == TextStatus::Text
                && artifact.model_policy != ModelExposurePolicy::Never
        })
        .map(|artifact| (artifact.path.as_str(), artifact))
        .collect();

    let read = |path: &str| -> Option<crate::analysis::TsConfigProfile> {
        readable.get(path)?;
        let text = std::fs::read_to_string(repo_root.join(path)).ok()?;
        crate::analysis::parse_tsconfig(&text)
    };

    let mut configs: BTreeMap<String, crate::analysis::TsConfigProfile> = readable
        .keys()
        .filter(|path| file_name(path).starts_with("tsconfig."))
        .filter_map(|path| Some(((*path).to_owned(), read(path)?)))
        .collect();

    // Pull in extended configs until the set closes. Bounded by `readable`,
    // and each pass only adds paths not already present, so a cyclic
    // `extends` cannot loop here.
    loop {
        let wanted: Vec<String> = configs
            .iter()
            .filter_map(|(path, profile)| {
                let extends = profile.extends.as_deref()?;
                // A bare specifier lives in node_modules, which is not scanned.
                if !(extends.starts_with("./") || extends.starts_with("../")) {
                    return None;
                }
                let directory = path.rsplit_once('/').map_or("", |(parent, _)| parent);
                let mut resolved =
                    crate::resolve::imports::resolve_relative_path(Path::new(directory), extends);
                if !resolved.ends_with(".json") {
                    resolved.push_str(".json");
                }
                (!configs.contains_key(&resolved)).then_some(resolved)
            })
            .collect();
        if wanted.is_empty() {
            break;
        }
        let mut added = false;
        for path in wanted {
            match read(&path) {
                Some(profile) => {
                    configs.insert(path, profile);
                    added = true;
                }
                // Record the miss so the loop does not ask for it forever.
                None => {
                    configs.insert(path, crate::analysis::TsConfigProfile::default());
                    added = true;
                }
            }
        }
        if !added {
            break;
        }
    }

    configs
}

#[cfg(test)]
fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
}

#[cfg(test)]
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

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GRAPH_BUILD_PASS_ORDER;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-57 AC2, end to end through the real analyzers and builder rather
    /// than hand-built facts: `provider.dumps()` in one file resolves to the
    /// method on a class constructed from an import of another file, and the
    /// same-named method on an unrelated class is not what it resolves to.
    #[test]
    fn cross_file_receiver_calls_resolve_through_the_real_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/provider.py"),
            "class Provider:\n    def dumps(self):\n        return self.helper()\n\n    def helper(self):\n        return 1\n",
        )?;
        // A same-named method on an unrelated class: resolving by method name
        // alone would make `dumps` ambiguous and could land here.
        std::fs::write(
            temp.path().join("src/other.py"),
            "class Unrelated:\n    def dumps(self):\n        return 2\n",
        )?;
        // A relative import, which is how a package imports its own modules
        // and what every intra-project import in the pinned Flask corpus
        // looks like.
        std::fs::write(temp.path().join("src/__init__.py"), "")?;
        std::fs::write(
            temp.path().join("src/app.py"),
            "from .provider import Provider\n\nprovider = Provider()\n\n\ndef run():\n    return provider.dumps()\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let propagated: Vec<&str> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                })
            })
            .map(|relation| relation.target.as_str())
            .collect();

        assert_eq!(
            propagated,
            vec![
                // `provider.dumps()` in app.py, typed by the construction of
                // an imported class.
                "symbol:src/provider.py#src.provider::Provider::dumps",
                // `self.helper()` inside Provider.dumps.
                "symbol:src/provider.py#src.provider::Provider::helper",
            ],
            "receiver calls must resolve to the receiver's class, not to any same-named method",
        );

        Ok(())
    }

    /// LIT-57 AC2 for TypeScript, end to end. Uses a direct relative import:
    /// a barrel (`from '../../client'` re-exporting the class) needs LIT-45.3
    /// before it can resolve, which is why the pinned Full Stack FastAPI
    /// corpus produces no TypeScript propagation yet.
    #[test]
    fn typescript_receiver_calls_resolve_through_the_real_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/provider.ts"),
            "export class Provider {\n  dumps() {\n    return this.helper();\n  }\n  helper() {\n    return 1;\n  }\n}\n",
        )?;
        std::fs::write(
            temp.path().join("src/other.ts"),
            "export class Unrelated {\n  dumps() {\n    return 2;\n  }\n}\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { Provider } from './provider';\n\nconst provider = new Provider();\n\nexport function run() {\n  return provider.dumps();\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let propagated: Vec<&str> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                })
            })
            .map(|relation| relation.target.as_str())
            .collect();

        assert_eq!(
            propagated,
            vec![
                // `provider.dumps()` in app.ts, typed by `new Provider()`
                // where `Provider` came from the import.
                "symbol:src/provider.ts#src/provider.ts::Provider::dumps",
                // `this.helper()` inside Provider.dumps.
                "symbol:src/provider.ts#src/provider.ts::Provider::helper",
            ],
            "the same-named `dumps` on Unrelated must not be what this resolves to",
        );

        Ok(())
    }

    /// LIT-45.5 AC2 end to end: TypeScript inheritance must be extracted by
    /// the analyzer, materialized as a real graph edge, and visible to the
    /// receiver resolver before it handles `this.inherited()`.
    #[test]
    fn typescript_inherited_receiver_calls_resolve_through_the_real_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/base.ts"),
            "export class Base {\n  inherited() { return 1; }\n}\n",
        )?;
        std::fs::write(
            temp.path().join("src/child.ts"),
            "import { Base } from './base';\nexport class Child extends Base {\n  run() { return this.inherited(); }\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Inherits
                && relation.source.as_str() == "symbol:src/child.ts#src/child.ts::Child"
                && relation.target.as_str() == "symbol:src/base.ts#src/base.ts::Base"
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Calls
                && relation.source.as_str() == "artifact:src/child.ts"
                && relation.target.as_str() == "symbol:src/base.ts#src/base.ts::Base::inherited"
                && relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                })
        }));

        Ok(())
    }

    /// Import facts are file-global, so two conditional imports that bind the
    /// same local name to different classes are ambiguous. Neither branch may
    /// win merely because it was visited last.
    #[test]
    fn conditional_import_name_collisions_do_not_type_receivers()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(temp.path().join("src/__init__.py"), "")?;
        std::fs::write(
            temp.path().join("src/left.py"),
            "class Service:\n    def run(self):\n        return 'left'\n",
        )?;
        std::fs::write(
            temp.path().join("src/right.py"),
            "class Service:\n    def run(self):\n        return 'right'\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.py"),
            "import typing as t\n\nif t.TYPE_CHECKING:\n    from .left import Service\n\nif t.TYPE_CHECKING:\n    from .right import Service\n\n\ndef use(service: Service):\n    return service.run()\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(!graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Calls
                && relation.source.as_str() == "artifact:src/app.py"
                && relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                })
        }));

        Ok(())
    }

    /// Two calls to the same target can share a physical source line. Their
    /// relation identities must remain distinct so indexes and storage never
    /// collapse one call site into the other.
    #[test]
    fn same_line_receiver_calls_have_distinct_relation_ids()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "class App { helper() {} run() { this.helper(); this.helper(); } }\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let calls: Vec<_> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.target.as_str() == "symbol:src/app.ts#src/app.ts::App::helper"
                    && relation.provenance.as_ref().is_some_and(|provenance| {
                        provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                    })
            })
            .collect();
        let ids: BTreeSet<_> = calls.iter().map(|relation| relation.id.as_str()).collect();

        assert_eq!(calls.len(), 2);
        assert_eq!(
            ids.len(),
            2,
            "each same-line call site needs its own relation id"
        );

        Ok(())
    }

    /// LIT-45.3 AC1/AC2 end to end, in the exact shape the pinned Full Stack
    /// FastAPI corpus uses: `import { ItemsService } from '../../client'`
    /// where `client/` is a directory whose `index.ts` re-exports the class
    /// from a nested barrel. Before this, the import named no file at all
    /// (directory imports were not candidates) and the barrel declared
    /// nothing, so the call resolved to nothing.
    #[test]
    fn barrel_re_exported_classes_resolve_to_the_declaring_module()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/client/services"))?;
        std::fs::write(
            temp.path().join("src/client/services/ItemsService.ts"),
            "export class ItemsService {\n  static deleteItem(id: string) {\n    return id;\n  }\n}\n",
        )?;
        // Two hops: client/index.ts -> services/index.ts -> ItemsService.ts,
        // with a star export for the first hop and a named one for the second.
        std::fs::write(
            temp.path().join("src/client/services/index.ts"),
            "export { ItemsService } from './ItemsService';\n",
        )?;
        std::fs::write(
            temp.path().join("src/client/index.ts"),
            "export * from './services';\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { ItemsService } from './client';\n\nexport function run() {\n  return ItemsService.deleteItem('1');\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let propagated: Vec<&str> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                })
            })
            .map(|relation| relation.target.as_str())
            .collect();

        assert_eq!(
            propagated,
            vec![
                "symbol:src/client/services/ItemsService.ts#src/client/services/ItemsService.ts::ItemsService::deleteItem",
            ],
            "the call must reach the declaring module, not stop at either barrel",
        );

        Ok(())
    }

    /// LIT-45.2 AC1/AC4 end to end: an aliased import resolves to the file the
    /// alias names, and an alias whose target does not exist resolves to
    /// nothing rather than to a plausible-looking neighbour.
    #[test]
    fn tsconfig_path_aliases_resolve_imports_and_misses_fall_through()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("tsconfig.json"),
            // JSONC, as the pinned NestJS config is: a comment and a trailing
            // comma, both of which plain JSON rejects.
            "{\n  // Path aliases.\n  \"compilerOptions\": {\n    \"baseUrl\": \".\",\n    \"paths\": {\n      \"@app/*\": [\"./src/*\"],\n    },\n  },\n}\n",
        )?;
        std::fs::write(
            temp.path().join("src/util.ts"),
            "export function helper() {\n  return 1;\n}\n",
        )?;
        std::fs::write(
            temp.path().join("src/main.ts"),
            "import { helper } from '@app/util';\nimport { nope } from '@app/does-not-exist';\n\nexport function run() {\n  return helper();\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let imports: Vec<&str> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/main.ts"
            })
            .map(|relation| relation.target.as_str())
            .collect();

        assert!(
            imports.contains(&"artifact:src/util.ts"),
            "`@app/util` must resolve through the alias to src/util.ts, got {imports:?}",
        );
        assert!(
            imports
                .iter()
                .any(|target| target.starts_with("unresolved:")),
            "`@app/does-not-exist` matches the alias pattern but names no file, \
             so it must stay unresolved rather than resolve to src/util.ts; got {imports:?}",
        );

        Ok(())
    }

    /// LIT-45.5: a parameter annotation types the receiver, in the exact shape
    /// that dominates the pinned Flask corpus -- `def serve(app: Flask)`
    /// followed by `app.run()`, where `Flask` is imported from another file.
    #[test]
    fn annotated_parameters_type_receivers_through_the_real_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(temp.path().join("src/__init__.py"), "")?;
        std::fs::write(
            temp.path().join("src/app.py"),
            "class Flask:\n    def run(self):\n        return 1\n",
        )?;
        // The import sits under `if TYPE_CHECKING:`, as it does for every
        // annotation receiver in the pinned Flask corpus -- a top-level-only
        // import scan misses it and the annotation types nothing.
        std::fs::write(
            temp.path().join("src/serve.py"),
            "import typing as t\n\nif t.TYPE_CHECKING:\n    from .app import Flask\n\n\ndef serve(app: Flask):\n    return app.run()\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let propagated: Vec<(&str, &str)> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolver_strategy == crate::resolve::PROPAGATE_STRATEGY
                })
            })
            .map(|relation| (relation.source.as_str(), relation.target.as_str()))
            .collect();

        assert_eq!(
            propagated,
            vec![(
                "artifact:src/serve.py",
                "symbol:src/app.py#src.app::Flask::run",
            )],
            "`app` is typed only by its annotation; the edge must cross into app.py",
        );

        Ok(())
    }

    /// LIT-45.4 AC1-AC3: CommonJS requires resolve like ESM imports -- a
    /// relative require to the local artifact (through LIT-45.1's candidate
    /// order, so `./util` finds `util.js`), a bare require to the declared
    /// package, and a dynamic require to nothing at all.
    #[test]
    fn commonjs_requires_resolve_through_the_import_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("package.json"),
            "{\n  \"name\": \"app\",\n  \"dependencies\": {\n    \"react\": \"^18.0.0\"\n  }\n}\n",
        )?;
        std::fs::write(temp.path().join("src/util.js"), "module.exports = {};\n")?;
        std::fs::write(
            temp.path().join("src/main.js"),
            "const util = require('./util');\nconst react = require('react');\nconst dynamic = require(process.env.MODULE);\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let imports: Vec<&str> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/main.js"
            })
            .map(|relation| relation.target.as_str())
            .collect();

        assert!(
            imports.contains(&"artifact:src/util.js"),
            "require('./util') must resolve to the local artifact, got {imports:?}",
        );
        assert!(
            imports.contains(&"package:react"),
            "require('react') must resolve to the declared dependency, got {imports:?}",
        );
        assert!(
            !imports
                .iter()
                .any(|target| target.contains("process.env") || target.contains("MODULE")),
            "a dynamic require must produce no import fact at all, got {imports:?}",
        );

        Ok(())
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
}
