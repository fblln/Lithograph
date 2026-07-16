//! Agent-style repository research artifacts used as the intermediate
//! memory layer between graph indexing and documentation composition.

use crate::architecture::{ArchitectureLayer, LayerDetector};
use crate::domain::{Artifact, ArtifactCategory, SupportTier};
use crate::graph::{ConfigNodeKind, Graph, GraphNode, RelationKind};
use crate::inventory::language::{RegistryIndexTier, by_name as registry_language};
use crate::knowledge_agent::{
    AgentContext, DataSourceKey, DataSourceResolution, DataSourceSpec, DataSourceValue,
    KnowledgeAgent,
};
use crate::plan::DocumentationModule;
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Language indexing support tier exposed to research and architecture docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LanguageSupportTier {
    /// The language or format was detected and counted.
    Detected,
    /// Syntax or structured facts are extracted deterministically.
    SyntaxIndexed,
    /// Syntax facts are eligible for package/import/type-aware refinement.
    HybridResolved,
}

/// Count and support level for one detected language or structured format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSupportFact {
    /// Stable language or format name.
    pub language: String,
    /// Highest current support tier for this language.
    pub tier: LanguageSupportTier,
    /// Planned Phase 1 support tier for this language or format.
    pub target_tier: LanguageSupportTier,
    /// Registry resolver strategy used for current indexing.
    pub resolver_strategy: String,
    /// Number of repository artifacts detected for this language/format.
    pub artifact_count: usize,
}

/// Common evidence pointer carried by deterministic agent reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchEvidence {
    /// Repository path, graph node id, or query name that supports the claim.
    pub reference: String,
}

/// Project purpose, users, external systems, and system boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemContextReport {
    /// Concise project positioning inferred from modules and existing docs.
    pub project_summary: String,
    /// Major code/config areas that are in scope for the generated docs.
    pub included_components: Vec<String>,
    /// External systems or unresolved references observed in the graph.
    pub external_systems: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Domain-level module map and relationships.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainModulesReport {
    /// Important modules or domains.
    pub modules: Vec<DomainModuleFact>,
    /// Cross-module relationships.
    pub relations: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// One domain or module fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainModuleFact {
    /// Module name.
    pub name: String,
    /// Module kind.
    pub kind: String,
    /// Number of graph members owned by this module.
    pub member_count: usize,
    /// Representative evidence paths or graph ids.
    pub evidence: Vec<String>,
}

/// Architecture-focused report used by the Architecture editor context.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureReport {
    /// Detected language and format support coverage.
    pub languages: Vec<LanguageSupportFact>,
    /// Per-artifact architecture layer classification (LIT-22.5.2).
    pub layers: Vec<ArchitectureLayer>,
    /// Architecture style and container/component clues.
    pub architecture_facts: Vec<String>,
    /// Hotspots or central modules.
    pub hotspots: Vec<String>,
    /// Known technical decisions and existing architecture docs.
    pub decisions_and_docs: Vec<String>,
    /// Mermaid diagram authored from deterministic graph facts.
    pub mermaid: Option<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Workflow and execution path report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowReport {
    /// Command/runtime/configuration facts that suggest workflows.
    pub workflows: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Boundary and interface report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoundaryReport {
    /// External packages, env vars, unresolved refs, and boundary relations.
    pub boundaries: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Detailed report for central modules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyModulesReport {
    /// Largest or most connected modules to prioritize in summaries.
    pub modules: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Optional database report, emitted only when database artifacts exist.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseReport {
    /// Database schemas, migrations, or database-related modules.
    pub database_facts: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Optional cross-service report (LIT-22.6.3 AC2), emitted only when the
/// graph has at least one HTTP/RPC/GraphQL route (LIT-22.3.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossServiceReport {
    /// Route/RPC/GraphQL field facts.
    pub routes: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Optional deployment report (LIT-22.6.3 AC2), emitted only when the
/// repository has container/compose/CI deployment evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentReport {
    /// Container images, Compose services, and deployment-definition facts.
    pub deployment_facts: Vec<String>,
    /// Evidence backing this report.
    pub evidence: Vec<ResearchEvidence>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Persisted output of the deterministic agent-style research pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemory {
    /// System context researcher output.
    pub system_context: SystemContextReport,
    /// Domain modules detector output.
    pub domain_modules: DomainModulesReport,
    /// Architecture researcher output.
    pub architecture: ArchitectureReport,
    /// Workflow researcher output.
    pub workflows: WorkflowReport,
    /// Boundary analyzer output.
    pub boundaries: BoundaryReport,
    /// Key modules insight output.
    pub key_modules: KeyModulesReport,
    /// Optional database analyzer output.
    pub database: Option<DatabaseReport>,
    /// Optional cross-service researcher output (LIT-22.6.3 AC2).
    pub cross_service: Option<CrossServiceReport>,
    /// Optional deployment researcher output (LIT-22.6.3 AC2).
    pub deployment: Option<DeploymentReport>,
}

impl AgentMemory {
    /// Persists one JSON file per agent (AC2). Does not write the combined
    /// index; see [`AgentMemoryIndex::persist`].
    pub fn persist(&self, research_dir: &Path) -> std::io::Result<()> {
        JsonStore.write_if_changed(
            &research_dir.join("system-context.json"),
            &self.system_context,
        )?;
        JsonStore.write_if_changed(
            &research_dir.join("domain-modules.json"),
            &self.domain_modules,
        )?;
        JsonStore.write_if_changed(
            &research_dir.join("architecture-report.json"),
            &self.architecture,
        )?;
        JsonStore.write_if_changed(&research_dir.join("workflows.json"), &self.workflows)?;
        JsonStore.write_if_changed(&research_dir.join("boundaries.json"), &self.boundaries)?;
        JsonStore.write_if_changed(&research_dir.join("key-modules.json"), &self.key_modules)?;
        if let Some(database) = &self.database {
            JsonStore.write_if_changed(&research_dir.join("database.json"), database)?;
        }
        if let Some(cross_service) = &self.cross_service {
            JsonStore.write_if_changed(&research_dir.join("cross-service.json"), cross_service)?;
        }
        if let Some(deployment) = &self.deployment {
            JsonStore.write_if_changed(&research_dir.join("deployment.json"), deployment)?;
        }
        Ok(())
    }

    /// Memory keys of every report actually populated this run, in a
    /// stable order: always-present reports first, then optional ones
    /// that are `Some` (LIT-22.6.5 AC1).
    pub fn present_report_keys(&self) -> Vec<String> {
        let mut keys = vec![
            "system-context".to_owned(),
            "domain-modules".to_owned(),
            "architecture-report".to_owned(),
            "workflows".to_owned(),
            "boundaries".to_owned(),
            "key-modules".to_owned(),
        ];
        if self.database.is_some() {
            keys.push("database".to_owned());
        }
        if self.cross_service.is_some() {
            keys.push("cross-service".to_owned());
        }
        if self.deployment.is_some() {
            keys.push("deployment".to_owned());
        }
        keys
    }
}

/// Current schema version stamped on every newly written
/// [`AgentMemoryIndex`] (LIT-22.6.5 AC1/AC3). Bump this whenever
/// `AgentMemory`'s persisted shape changes in a way a reader must know
/// about; a file with a lower `schema_version` is stale and should be
/// treated as absent (regenerate) rather than read as current.
pub const AGENT_MEMORY_SCHEMA_VERSION: u32 = 1;

/// Versioned envelope persisted as `agent-memory.json` (LIT-22.6.5 AC1):
/// schema version, an input hash over the artifacts/graph that produced
/// this memory, which pipeline stage produced it, and which per-agent
/// report keys are actually populated -- so a future agent or MCP tool can
/// tell what it is looking at without re-deriving it from the report
/// bodies. `#[serde(flatten)]` keeps every `AgentMemory` field at the JSON
/// top level (unchanged shape for existing readers), with the envelope
/// fields as additional sibling keys.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemoryIndex {
    /// Schema version this file was written with. `#[serde(default)]`
    /// means a file written before this field existed reads back as `0`
    /// -- an explicit, checkable "pre-versioning" marker (AC3) rather
    /// than a guess.
    #[serde(default)]
    pub schema_version: u32,
    /// Hash over the artifact content hashes and graph node ids that
    /// produced this memory.
    pub input_hash: String,
    /// Pipeline stage that produced this memory (always `Research`,
    /// recorded explicitly rather than assumed).
    pub produced_by_stage: crate::run::PipelineStage,
    /// Memory keys of every report actually populated (see
    /// [`AgentMemory::present_report_keys`]).
    pub report_keys: Vec<String>,
    /// The reports themselves.
    #[serde(flatten)]
    pub memory: AgentMemory,
}

impl AgentMemoryIndex {
    /// Wraps `memory` in a current-schema-version index.
    pub fn new(memory: AgentMemory, input_hash: String) -> Self {
        let report_keys = memory.present_report_keys();
        Self {
            schema_version: AGENT_MEMORY_SCHEMA_VERSION,
            input_hash,
            produced_by_stage: crate::run::PipelineStage::Research,
            report_keys,
            memory,
        }
    }

    /// True when this index was written by the current schema version
    /// (AC3). A caller that finds `false` should treat the memory as
    /// stale rather than trust its shape.
    pub fn is_current_schema(&self) -> bool {
        self.schema_version == AGENT_MEMORY_SCHEMA_VERSION
    }

    /// Persists the combined index (AC1) and every per-agent report file
    /// (AC2).
    pub fn persist(&self, research_dir: &Path) -> std::io::Result<()> {
        JsonStore.write_if_changed(&research_dir.join("agent-memory.json"), self)?;
        self.memory.persist(research_dir)
    }
}

/// Hashes the artifact content hashes and graph node ids that produced one
/// research run, so a persisted [`AgentMemoryIndex`] can be checked against
/// a later run without re-comparing every report field by field.
fn research_input_hash(artifacts: &[Artifact], graph: &Graph) -> String {
    let mut artifact_hashes: Vec<&str> = artifacts
        .iter()
        .map(|artifact| artifact.content_hash.as_str())
        .collect();
    artifact_hashes.sort_unstable();
    let mut node_ids: Vec<&str> = graph.nodes.iter().map(|node| node.id().as_str()).collect();
    node_ids.sort_unstable();
    let combined = format!("{}\n{}", artifact_hashes.join(","), node_ids.join(","));
    blake3::hash(combined.as_bytes()).to_hex().to_string()
}

/// Structured, persisted research facts for one repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchBrief {
    /// High-level module facts and likely responsibilities.
    pub system_context: Vec<String>,
    /// Command/runtime/configuration facts that suggest workflows.
    pub workflows: Vec<String>,
    /// External packages, env vars, unresolved refs, and boundary relations.
    pub boundaries: Vec<String>,
    /// Manifests, deployment/config files, packages, and env vars.
    pub configuration: Vec<String>,
    /// Largest or most connected modules to prioritize in summaries.
    pub key_modules: Vec<String>,
    /// Typed agent-style reports used by architecture documentation contexts.
    pub agent_memory: AgentMemory,
    /// Hash over the artifacts/graph that produced `agent_memory`
    /// (LIT-22.6.5 AC1); carried into [`AgentMemoryIndex::input_hash`]
    /// when persisted.
    pub input_hash: String,
}

/// Builds deterministic research artifacts from the already-validated graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResearchBuilder;

impl ResearchBuilder {
    /// Derives compact agent-style reports without calling a model. Runs
    /// every agent (AC1) through the shared `KnowledgeAgent` framework
    /// (LIT-22.6.2), in a fixed order (AC4), against one shared
    /// [`AgentContext`] built from `artifacts`/`graph`/`modules`.
    pub fn build(
        &self,
        artifacts: &[Artifact],
        graph: &Graph,
        modules: &[DocumentationModule],
    ) -> ResearchBrief {
        let context = AgentContext::new()
            .with(
                DataSourceKey::Artifacts,
                DataSourceValue::Artifacts(artifacts),
            )
            .with(DataSourceKey::Graph, DataSourceValue::Graph(graph))
            .with(DataSourceKey::Modules, DataSourceValue::Modules(modules));

        let system_context = run_agent(SystemContextResearcher, &context);
        let domain_modules = run_agent(DomainModulesDetector, &context);
        let architecture = run_agent(ArchitectureResearcher, &context);
        let workflows = run_agent(WorkflowResearcher, &context);
        let boundaries = run_agent(BoundaryAnalyzer, &context);
        let key_modules = run_agent(KeyModulesInsight, &context);
        let database = run_agent(DatabaseOverviewAnalyzer, &context);
        let database = if database.database_facts.is_empty() {
            None
        } else {
            Some(database)
        };
        let cross_service = run_agent(CrossServiceResearcher, &context);
        let cross_service = if cross_service.routes.is_empty() {
            None
        } else {
            Some(cross_service)
        };
        let deployment = run_agent(DeploymentResearcher, &context);
        let deployment = if deployment.deployment_facts.is_empty() {
            None
        } else {
            Some(deployment)
        };
        let agent_memory = AgentMemory {
            system_context,
            domain_modules,
            architecture,
            workflows: workflows.clone(),
            boundaries: boundaries.clone(),
            key_modules: key_modules.clone(),
            database,
            cross_service,
            deployment,
        };

        ResearchBrief {
            system_context: legacy_system_context(&agent_memory),
            workflows: workflows.workflows,
            boundaries: boundaries.boundaries,
            configuration: configuration(artifacts, graph),
            key_modules: key_modules.modules,
            input_hash: research_input_hash(artifacts, graph),
            agent_memory,
        }
    }
}

/// Runs one agent against a context that `ResearchBuilder::build` always
/// populates with `Artifacts`/`Graph`/`Modules`, so every agent declared
/// here (all of which require only that trio) cannot actually observe a
/// missing-required-source error; `unreachable!` documents that invariant
/// rather than threading an infallible `Result` through an infallible
/// `ResearchBrief`-returning `build()`.
fn run_agent<A: KnowledgeAgent>(agent: A, context: &AgentContext<'_>) -> A::Output {
    match agent.run(context) {
        Ok(output) => output,
        Err(error) => unreachable!("{} agent failed unexpectedly: {error}", agent.memory_key()),
    }
}

fn artifacts_graph_modules_required() -> DataSourceSpec {
    DataSourceSpec {
        required: &[
            DataSourceKey::Artifacts,
            DataSourceKey::Graph,
            DataSourceKey::Modules,
        ],
        optional: &[],
    }
}

fn required_artifacts<'a>(context: &AgentContext<'a>) -> &'a [Artifact] {
    context
        .artifacts()
        .unwrap_or_else(|| unreachable!("Artifacts declared required"))
}

fn required_graph<'a>(context: &AgentContext<'a>) -> &'a Graph {
    context
        .graph()
        .unwrap_or_else(|| unreachable!("Graph declared required"))
}

fn required_modules<'a>(context: &AgentContext<'a>) -> &'a [DocumentationModule] {
    context
        .modules()
        .unwrap_or_else(|| unreachable!("Modules declared required"))
}

#[derive(Debug, Clone, Copy, Default)]
struct SystemContextResearcher;

impl KnowledgeAgent for SystemContextResearcher {
    type Output = SystemContextReport;

    fn memory_key(&self) -> &'static str {
        "system-context"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let artifacts = required_artifacts(context);
        let graph = required_graph(context);
        let modules = required_modules(context);
        let components: Vec<String> = modules
            .iter()
            .take(12)
            .map(|module| format!("{} ({:?})", module.name, module.kind))
            .collect();
        let external_systems = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Package(package) if package.is_external => Some(package.name.clone()),
                GraphNode::Unresolved(unresolved) => Some(unresolved.value.clone()),
                _ => None,
            })
            .take(20)
            .collect();
        Self::Output {
            project_summary: format!(
                "Repository contains {} artifact(s), {} graph node(s), {} graph relation(s), and {} documentation module(s).",
                artifacts.len(),
                graph.nodes.len(),
                graph.relations.len(),
                modules.len()
            ),
            included_components: components,
            external_systems,
            evidence: artifact_evidence(artifacts, 8),
            confidence: confidence_for(artifacts.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct DomainModulesDetector;

impl KnowledgeAgent for DomainModulesDetector {
    type Output = DomainModulesReport;

    fn memory_key(&self) -> &'static str {
        "domain-modules"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let artifacts = required_artifacts(context);
        let graph = required_graph(context);
        let modules = required_modules(context);
        let module_facts = modules
            .iter()
            .take(20)
            .map(|module| DomainModuleFact {
                name: module.name.clone(),
                kind: format!("{:?}", module.kind),
                member_count: module.members.len(),
                evidence: module
                    .members
                    .iter()
                    .take(4)
                    .map(|member| member.as_str().to_owned())
                    .collect(),
            })
            .collect();
        let labels = node_labels(graph);
        let relations = graph
            .relations
            .iter()
            .filter(|relation| {
                matches!(
                    relation.kind,
                    RelationKind::Imports
                        | RelationKind::Calls
                        | RelationKind::References
                        | RelationKind::DependsOnPackage
                )
            })
            .take(40)
            .map(|relation| {
                format!(
                    "{} -[{:?}]-> {} ({:?})",
                    labels
                        .get(relation.source.as_str())
                        .map_or(relation.source.as_str(), String::as_str),
                    relation.kind,
                    labels
                        .get(relation.target.as_str())
                        .map_or(relation.target.as_str(), String::as_str),
                    relation.confidence
                )
            })
            .collect();
        Self::Output {
            modules: module_facts,
            relations,
            evidence: artifact_evidence(artifacts, 8),
            confidence: confidence_for(modules.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ArchitectureResearcher;

impl KnowledgeAgent for ArchitectureResearcher {
    type Output = ArchitectureReport;

    fn memory_key(&self) -> &'static str {
        "architecture-report"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let artifacts = required_artifacts(context);
        let graph = required_graph(context);
        let modules = required_modules(context);
        let languages = language_support(artifacts);
        let mut architecture_facts = Vec::new();
        architecture_facts.push(format!(
            "knowledge graph schema: {} node(s), {} relation(s)",
            graph.nodes.len(),
            graph.relations.len()
        ));
        architecture_facts.extend(modules.iter().take(10).map(|module| {
            format!(
                "container/component candidate: {} ({:?})",
                module.name, module.kind
            )
        }));
        let decisions_and_docs = artifacts
            .iter()
            .filter(|artifact| artifact.category == ArtifactCategory::Documentation)
            .filter(|artifact| {
                let path = artifact.path.as_str().to_lowercase();
                path.contains("architecture") || path.contains("adr") || path.contains("decision")
            })
            .map(|artifact| format!("existing architecture knowledge: {}", artifact.path))
            .take(20)
            .collect();
        let hotspots = key_modules(modules, graph);
        Self::Output {
            languages,
            layers: LayerDetector.detect(graph),
            architecture_facts,
            hotspots,
            decisions_and_docs,
            mermaid: architecture_mermaid(modules, graph),
            evidence: artifact_evidence(artifacts, 12),
            confidence: confidence_for(graph.nodes.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct WorkflowResearcher;

impl KnowledgeAgent for WorkflowResearcher {
    type Output = WorkflowReport;

    fn memory_key(&self) -> &'static str {
        "workflows"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let graph = required_graph(context);
        Self::Output {
            workflows: workflows(graph),
            evidence: relation_evidence(graph, 12),
            confidence: confidence_for(graph.relations.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct BoundaryAnalyzer;

impl KnowledgeAgent for BoundaryAnalyzer {
    type Output = BoundaryReport;

    fn memory_key(&self) -> &'static str {
        "boundaries"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let graph = required_graph(context);
        Self::Output {
            boundaries: boundaries(graph),
            evidence: relation_evidence(graph, 12),
            confidence: confidence_for(graph.relations.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct KeyModulesInsight;

impl KnowledgeAgent for KeyModulesInsight {
    type Output = KeyModulesReport;

    fn memory_key(&self) -> &'static str {
        "key-modules"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let artifacts = required_artifacts(context);
        let graph = required_graph(context);
        let modules = required_modules(context);
        Self::Output {
            modules: key_modules(modules, graph),
            evidence: artifact_evidence(artifacts, 10),
            confidence: confidence_for(modules.len()),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct DatabaseOverviewAnalyzer;

impl KnowledgeAgent for DatabaseOverviewAnalyzer {
    type Output = DatabaseReport;

    fn memory_key(&self) -> &'static str {
        "database"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let artifacts = required_artifacts(context);
        let database_facts = artifacts
            .iter()
            .filter(|artifact| {
                matches!(
                    artifact.category,
                    ArtifactCategory::DatabaseSchema | ArtifactCategory::DatabaseMigration
                ) || artifact.path.as_str().ends_with(".sql")
            })
            .map(|artifact| format!("{:?}: {}", artifact.category, artifact.path))
            .collect();
        Self::Output {
            database_facts,
            evidence: artifact_evidence(artifacts, 10),
            confidence: 80,
        }
    }
}

/// Optional (AC2): only produces routes when the graph has at least one
/// HTTP/RPC/GraphQL route node (LIT-22.3.4); `ResearchBuilder::build`
/// turns an empty report into `None`.
#[derive(Debug, Clone, Copy, Default)]
struct CrossServiceResearcher;

impl KnowledgeAgent for CrossServiceResearcher {
    type Output = CrossServiceReport;

    fn memory_key(&self) -> &'static str {
        "cross-service"
    }

    fn data_sources(&self) -> DataSourceSpec {
        DataSourceSpec {
            required: &[DataSourceKey::Graph],
            optional: &[],
        }
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let graph = required_graph(context);
        let routes: Vec<String> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Config(config) if config.kind == ConfigNodeKind::Route => {
                    Some(config.name.clone())
                }
                _ => None,
            })
            .take(40)
            .collect();
        let evidence = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Config(config) if config.kind == ConfigNodeKind::Route => {
                    Some(ResearchEvidence {
                        reference: config.id.as_str().to_owned(),
                    })
                }
                _ => None,
            })
            .take(12)
            .collect();
        let confidence = confidence_for(routes.len());
        Self::Output {
            routes,
            evidence,
            confidence,
        }
    }
}

/// Optional (AC2): only produces facts when the repository has
/// container/compose/deployment evidence; `ResearchBuilder::build` turns
/// an empty report into `None`.
#[derive(Debug, Clone, Copy, Default)]
struct DeploymentResearcher;

impl KnowledgeAgent for DeploymentResearcher {
    type Output = DeploymentReport;

    fn memory_key(&self) -> &'static str {
        "deployment"
    }

    fn data_sources(&self) -> DataSourceSpec {
        artifacts_graph_modules_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let artifacts = required_artifacts(context);
        let graph = required_graph(context);
        let mut deployment_facts: Vec<String> = artifacts
            .iter()
            .filter(|artifact| {
                matches!(
                    artifact.category,
                    ArtifactCategory::ContainerDefinition | ArtifactCategory::DeploymentDefinition
                )
            })
            .map(|artifact| format!("{:?}: {}", artifact.category, artifact.path))
            .collect();
        deployment_facts.extend(graph.nodes.iter().filter_map(|node| match node {
            GraphNode::Config(config) if config.kind == ConfigNodeKind::Service => {
                Some(format!("compose service: {}", config.name))
            }
            GraphNode::Container(container) => {
                Some(format!("container image: {}", container.reference))
            }
            _ => None,
        }));
        deployment_facts.truncate(40);
        Self::Output {
            confidence: confidence_for(deployment_facts.len()),
            evidence: artifact_evidence(artifacts, 10),
            deployment_facts,
        }
    }
}

fn language_support(artifacts: &[Artifact]) -> Vec<LanguageSupportFact> {
    let mut counts: BTreeMap<(String, LanguageSupportTier, LanguageSupportTier, String), usize> =
        BTreeMap::new();
    for artifact in artifacts {
        let Some(language) = artifact.detected_format.as_deref() else {
            continue;
        };
        let capability = support_capability_for(language, artifact.support_tier);
        *counts
            .entry((
                language.to_owned(),
                capability.current_tier,
                capability.target_tier,
                capability.resolver_strategy,
            ))
            .or_default() += 1;
    }
    let mut by_language: BTreeMap<String, LanguageSupportFact> = BTreeMap::new();
    for ((language, tier, target_tier, resolver_strategy), artifact_count) in counts {
        by_language
            .entry(language.clone())
            .and_modify(|fact| {
                fact.tier = fact.tier.max(tier);
                fact.target_tier = fact.target_tier.max(target_tier);
                fact.artifact_count += artifact_count;
            })
            .or_insert(LanguageSupportFact {
                language,
                tier,
                target_tier,
                resolver_strategy,
                artifact_count,
            });
    }
    by_language.into_values().collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LanguageCapability {
    current_tier: LanguageSupportTier,
    target_tier: LanguageSupportTier,
    resolver_strategy: String,
}

fn support_capability_for(language: &str, inventory_tier: SupportTier) -> LanguageCapability {
    if let Some(entry) = registry_language(language) {
        return LanguageCapability {
            current_tier: registry_tier(entry.current_tier),
            target_tier: registry_tier(entry.target_tier),
            resolver_strategy: entry.resolver_strategy.to_owned(),
        };
    }
    let tier = match inventory_tier {
        SupportTier::DeepLanguage | SupportTier::StructuredFormat => {
            LanguageSupportTier::SyntaxIndexed
        }
        SupportTier::GenericText | SupportTier::Opaque => LanguageSupportTier::Detected,
    };
    LanguageCapability {
        current_tier: tier,
        target_tier: tier,
        resolver_strategy: "inventory-fallback".to_owned(),
    }
}

fn registry_tier(tier: RegistryIndexTier) -> LanguageSupportTier {
    match tier {
        RegistryIndexTier::Detected => LanguageSupportTier::Detected,
        RegistryIndexTier::SyntaxIndexed => LanguageSupportTier::SyntaxIndexed,
        RegistryIndexTier::HybridResolved => LanguageSupportTier::HybridResolved,
    }
}

fn legacy_system_context(memory: &AgentMemory) -> Vec<String> {
    let mut facts = vec![memory.system_context.project_summary.clone()];
    facts.extend(memory.domain_modules.modules.iter().take(12).map(|module| {
        format!(
            "{} ({}) owns {} graph member(s)",
            module.name, module.kind, module.member_count
        )
    }));
    facts.extend(
        memory
            .architecture
            .languages
            .iter()
            .take(12)
            .map(|language| {
                format!(
                    "language support: {} current {:?}, target {:?} via {} ({} artifact(s))",
                    language.language,
                    language.tier,
                    language.target_tier,
                    language.resolver_strategy,
                    language.artifact_count
                )
            }),
    );
    facts
}

fn workflows(graph: &Graph) -> Vec<String> {
    let labels = node_labels(graph);
    graph
        .relations
        .iter()
        .filter(|relation| {
            matches!(
                relation.kind,
                RelationKind::RunsCommand
                    | RelationKind::UsesImage
                    | RelationKind::BuildsImage
                    | RelationKind::PublishesImage
                    | RelationKind::ReadsEnv
            )
        })
        .take(40)
        .map(|relation| {
            format!(
                "{} -[{:?}]-> {} ({:?})",
                labels
                    .get(relation.source.as_str())
                    .map_or(relation.source.as_str(), String::as_str),
                relation.kind,
                labels
                    .get(relation.target.as_str())
                    .map_or(relation.target.as_str(), String::as_str),
                relation.confidence
            )
        })
        .collect()
}

fn boundaries(graph: &Graph) -> Vec<String> {
    let labels = node_labels(graph);
    let mut facts = Vec::new();
    for node in &graph.nodes {
        match node {
            GraphNode::Package(package) if package.is_external => {
                facts.push(format!("external package: {}", package.name));
            }
            GraphNode::EnvVar(env) => facts.push(format!("environment variable: {}", env.name)),
            GraphNode::Unresolved(unresolved) => {
                facts.push(format!("unresolved reference: {}", unresolved.value));
            }
            _ => {}
        }
        if facts.len() >= 40 {
            return facts;
        }
    }
    for relation in &graph.relations {
        if matches!(
            relation.kind,
            RelationKind::DependsOnPackage | RelationKind::ReadsEnv | RelationKind::References
        ) {
            facts.push(format!(
                "{} crosses boundary via {:?} to {}",
                labels
                    .get(relation.source.as_str())
                    .map_or(relation.source.as_str(), String::as_str),
                relation.kind,
                labels
                    .get(relation.target.as_str())
                    .map_or(relation.target.as_str(), String::as_str)
            ));
        }
        if facts.len() >= 40 {
            break;
        }
    }
    facts
}

fn configuration(artifacts: &[Artifact], graph: &Graph) -> Vec<String> {
    let mut facts: Vec<String> = artifacts
        .iter()
        .filter(|artifact| {
            matches!(
                artifact.category,
                ArtifactCategory::Configuration
                    | ArtifactCategory::BuildDefinition
                    | ArtifactCategory::PackageManifest
                    | ArtifactCategory::DependencyLockfile
                    | ArtifactCategory::ContainerDefinition
                    | ArtifactCategory::DeploymentDefinition
                    | ArtifactCategory::ContinuousIntegration
                    | ArtifactCategory::DatabaseSchema
                    | ArtifactCategory::DatabaseMigration
            )
        })
        .take(30)
        .map(|artifact| format!("{:?}: {}", artifact.category, artifact.path))
        .collect();
    for node in &graph.nodes {
        match node {
            GraphNode::Config(config) => facts.push(format!("{:?}: {}", config.kind, config.name)),
            GraphNode::Package(package) => facts.push(format!(
                "package: {}{}",
                package.name,
                if package.is_external {
                    " (external)"
                } else {
                    ""
                }
            )),
            _ => {}
        }
        if facts.len() >= 60 {
            break;
        }
    }
    facts
}

fn key_modules(modules: &[DocumentationModule], graph: &Graph) -> Vec<String> {
    let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
    for module in modules {
        degree.insert(module.id.as_str(), module.members.len());
    }
    for relation in &graph.relations {
        for module in modules {
            if (module.members.contains(&relation.source)
                || module.members.contains(&relation.target))
                && let Some(count) = degree.get_mut(module.id.as_str())
            {
                *count += 1;
            }
        }
    }
    let mut modules_by_score: Vec<(&DocumentationModule, usize)> = modules
        .iter()
        .map(|module| (module, degree.get(module.id.as_str()).copied().unwrap_or(0)))
        .collect();
    modules_by_score.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
    modules_by_score
        .into_iter()
        .take(10)
        .map(|(module, score)| format!("{} ({:?}) score {}", module.name, module.kind, score))
        .collect()
}

fn architecture_mermaid(modules: &[DocumentationModule], graph: &Graph) -> Option<String> {
    if modules.is_empty() {
        return None;
    }
    let mut node_ids = BTreeMap::new();
    let mut lines = vec!["```mermaid".to_owned(), "flowchart LR".to_owned()];
    for (index, module) in modules.iter().take(10).enumerate() {
        let id = format!("M{}", index + 1);
        node_ids.insert(module.id.as_str(), id.clone());
        lines.push(format!("  {id}[\"{}\"]", mermaid_label(&module.name)));
    }
    for relation in graph.relations.iter().take(30) {
        let source_owner = modules
            .iter()
            .find(|module| module.members.contains(&relation.source))
            .and_then(|module| node_ids.get(module.id.as_str()));
        let target_owner = modules
            .iter()
            .find(|module| module.members.contains(&relation.target))
            .and_then(|module| node_ids.get(module.id.as_str()));
        let (Some(source), Some(target)) = (source_owner, target_owner) else {
            continue;
        };
        if source != target {
            lines.push(format!("  {source} -->|{:?}| {target}", relation.kind));
        }
        if lines.len() >= 32 {
            break;
        }
    }
    lines.push("```".to_owned());
    Some(lines.join("\n"))
}

fn mermaid_label(label: &str) -> String {
    label.replace('"', "'")
}

fn artifact_evidence(artifacts: &[Artifact], limit: usize) -> Vec<ResearchEvidence> {
    artifacts
        .iter()
        .take(limit)
        .map(|artifact| ResearchEvidence {
            reference: artifact.path.as_str().to_owned(),
        })
        .collect()
}

fn relation_evidence(graph: &Graph, limit: usize) -> Vec<ResearchEvidence> {
    graph
        .relations
        .iter()
        .take(limit)
        .map(|relation| ResearchEvidence {
            reference: relation.id.clone(),
        })
        .collect()
}

fn confidence_for(count: usize) -> u8 {
    if count == 0 {
        30
    } else if count < 3 {
        60
    } else {
        85
    }
}

fn node_labels(graph: &Graph) -> BTreeMap<&str, String> {
    graph
        .nodes
        .iter()
        .map(|node| {
            let label = match node {
                GraphNode::Artifact(node) => node.path.clone(),
                GraphNode::Rationale(node) => node.text.clone(),
                GraphNode::Symbol(node) => node.qualified_name.clone(),
                GraphNode::Config(node) => node.name.clone(),
                GraphNode::Documentation(node) => node.title.clone(),
                GraphNode::Container(node) => node.reference.clone(),
                GraphNode::Command(node) => node.text.clone(),
                GraphNode::EnvVar(node) => node.name.clone(),
                GraphNode::Module(node) => node.path.clone(),
                GraphNode::Package(node) => node.name.clone(),
                GraphNode::Unresolved(node) => node.value.clone(),
            };
            (node.id().as_str(), label)
        })
        .collect()
}

/// Path to the combined agent-memory file under a repository root.
pub fn agent_memory_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".lithograph/research/agent-memory.json")
}

#[cfg(test)]
mod tests {
    use super::{AgentMemoryIndex, LanguageSupportTier, ResearchBuilder, agent_memory_path};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::plan::ModulePlanner;
    use std::path::Path;

    type Fixture = (
        Vec<crate::domain::Artifact>,
        crate::graph::Graph,
        Vec<crate::plan::DocumentationModule>,
    );

    fn polyglot_fixture() -> Result<Fixture, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        Ok((artifacts, graph, modules))
    }

    #[test]
    fn builds_agent_memory_from_polyglot_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        assert!(!brief.system_context.is_empty());
        assert!(!brief.configuration.is_empty());
        assert!(!brief.key_modules.is_empty());
        assert!(!brief.agent_memory.domain_modules.modules.is_empty());
        assert!(
            brief
                .agent_memory
                .architecture
                .languages
                .iter()
                .any(|language| language.tier == LanguageSupportTier::HybridResolved)
        );

        Ok(())
    }

    #[test]
    fn persists_agent_memory_as_individual_reports() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        let temp = tempfile::TempDir::new()?;

        AgentMemoryIndex::new(brief.agent_memory, brief.input_hash)
            .persist(&temp.path().join(".lithograph/research"))?;

        assert!(agent_memory_path(temp.path()).exists());
        assert!(
            temp.path()
                .join(".lithograph/research/architecture-report.json")
                .exists()
        );

        Ok(())
    }

    #[test]
    fn language_support_facts_use_registry_current_and_target_tiers()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::create_dir_all(repo.path().join("cmd"))?;
        std::fs::create_dir_all(repo.path().join("schema"))?;
        std::fs::write(repo.path().join("app.py"), "import os\n")?;
        std::fs::write(repo.path().join("cmd/main.go"), "package main\n")?;
        std::fs::write(
            repo.path().join("schema/tables.sql"),
            "create table users(id int);\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        let languages = &brief.agent_memory.architecture.languages;
        let python = languages
            .iter()
            .find(|fact| fact.language == "python")
            .ok_or_else(|| std::io::Error::other("missing python support fact"))?;
        let go = languages
            .iter()
            .find(|fact| fact.language == "go")
            .ok_or_else(|| std::io::Error::other("missing go support fact"))?;
        let sql = languages
            .iter()
            .find(|fact| fact.language == "sql")
            .ok_or_else(|| std::io::Error::other("missing sql support fact"))?;

        assert_eq!(python.tier, LanguageSupportTier::HybridResolved);
        assert_eq!(python.target_tier, LanguageSupportTier::HybridResolved);
        // go now has a wired tree-sitter adapter (LIT-22.2.3): syntax-indexed
        // today, hybrid resolution is still LIT-22.3's job.
        assert_eq!(go.tier, LanguageSupportTier::SyntaxIndexed);
        assert_eq!(go.target_tier, LanguageSupportTier::HybridResolved);
        assert_eq!(sql.tier, LanguageSupportTier::SyntaxIndexed);
        assert_eq!(sql.target_tier, LanguageSupportTier::SyntaxIndexed);

        Ok(())
    }

    /// LIT-22.6.3 AC1: every agent named in the AC runs and lands in
    /// `AgentMemory`.
    #[test]
    fn every_named_agent_runs_through_the_shared_framework()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;

        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        let memory = &brief.agent_memory;

        assert!(!memory.system_context.project_summary.is_empty());
        assert!(!memory.domain_modules.modules.is_empty());
        assert!(!memory.architecture.languages.is_empty());
        assert!(!memory.workflows.workflows.is_empty());
        assert!(!memory.boundaries.boundaries.is_empty());
        assert!(!memory.key_modules.modules.is_empty());

        Ok(())
    }

    /// LIT-22.6.3 AC2/AC4 (optional data): the polyglot fixture has
    /// container/compose evidence but no HTTP route decorators, so
    /// `deployment` is populated and `cross_service` stays `None`.
    #[test]
    fn optional_researchers_run_only_when_their_evidence_exists()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;

        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        assert!(brief.agent_memory.deployment.is_some());
        assert!(brief.agent_memory.cross_service.is_none());

        Ok(())
    }

    /// LIT-22.6.3 AC2/AC4: a repository with an HTTP route decorator
    /// populates `cross_service`; one with no database/container/route
    /// evidence at all leaves every optional report `None`.
    #[test]
    fn cross_service_researcher_runs_when_a_route_exists() -> Result<(), Box<dyn std::error::Error>>
    {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("service.py"),
            "import requests\n\n\n@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        let cross_service = brief
            .agent_memory
            .cross_service
            .ok_or("expected a cross-service report")?;
        assert!(cross_service.routes.contains(&"GET /users/{id}".to_owned()));
        assert!(!cross_service.evidence.is_empty());
        assert!(brief.agent_memory.database.is_none());
        assert!(brief.agent_memory.deployment.is_none());

        Ok(())
    }

    /// LIT-22.6.3 AC3: every report's evidence list cites a real
    /// artifact path or graph node/relation id, not a placeholder.
    #[test]
    fn every_report_evidence_cites_real_references() -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        let memory = &brief.agent_memory;

        let artifact_paths: std::collections::BTreeSet<&str> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();
        let relation_ids: std::collections::BTreeSet<&str> = graph
            .relations
            .iter()
            .map(|relation| relation.id.as_str())
            .collect();

        for evidence in &memory.system_context.evidence {
            assert!(artifact_paths.contains(evidence.reference.as_str()));
        }
        assert!(!memory.workflows.evidence.is_empty());
        for evidence in &memory.workflows.evidence {
            assert!(relation_ids.contains(evidence.reference.as_str()));
        }

        Ok(())
    }

    /// LIT-22.6.3 AC4 (typed parsing): `AgentMemory` round-trips through
    /// JSON without loss, including the new optional reports.
    #[test]
    fn agent_memory_round_trips_through_json() -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        let json = serde_json::to_string(&brief.agent_memory)?;
        let parsed: super::AgentMemory = serde_json::from_str(&json)?;

        assert_eq!(parsed, brief.agent_memory);

        Ok(())
    }

    /// LIT-22.6.3 AC4 (ordering): running the same inputs through the
    /// framework twice produces byte-identical output -- no agent's
    /// output depends on iteration order over a non-deterministic
    /// collection.
    #[test]
    fn build_is_deterministic_across_repeated_runs() -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;

        let first = ResearchBuilder.build(&artifacts, &graph, &modules);
        let second = ResearchBuilder.build(&artifacts, &graph, &modules);

        assert_eq!(first, second);

        Ok(())
    }

    /// LIT-22.6.5 AC1: the persisted index carries the current schema
    /// version, an input hash matching the brief, and every populated
    /// report's memory key (the polyglot fixture has container/compose
    /// evidence but no routes, so `deployment` is present and
    /// `cross-service` is not).
    #[test]
    fn agent_memory_index_records_schema_version_input_hash_and_report_keys()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        let index = AgentMemoryIndex::new(brief.agent_memory.clone(), brief.input_hash.clone());

        assert_eq!(index.schema_version, super::AGENT_MEMORY_SCHEMA_VERSION);
        assert!(index.is_current_schema());
        assert_eq!(index.input_hash, brief.input_hash);
        assert!(!index.input_hash.is_empty());
        assert!(index.report_keys.contains(&"system-context".to_owned()));
        assert!(index.report_keys.contains(&"deployment".to_owned()));
        assert!(!index.report_keys.contains(&"cross-service".to_owned()));

        Ok(())
    }

    /// LIT-22.6.5 AC3: a file written before schema versioning existed (no
    /// `schema_version` key at all) deserializes with `schema_version: 0`
    /// rather than failing, and `is_current_schema` reports it as stale so
    /// a caller knows to regenerate instead of trusting its shape.
    #[test]
    fn pre_versioning_agent_memory_json_is_detected_as_a_stale_schema()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        let current = AgentMemoryIndex::new(brief.agent_memory, brief.input_hash);
        let mut json: serde_json::Value = serde_json::to_value(&current)?;
        json.as_object_mut()
            .ok_or("expected a JSON object")?
            .remove("schema_version");

        let parsed: AgentMemoryIndex = serde_json::from_value(json)?;

        assert_eq!(parsed.schema_version, 0);
        assert!(!parsed.is_current_schema());

        Ok(())
    }

    /// LIT-22.6.5 AC4: persisting the same index twice does not rewrite
    /// `agent-memory.json` the second time (no-op write stability).
    #[test]
    fn persisting_agent_memory_index_twice_is_a_no_op_write()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifacts, graph, modules) = polyglot_fixture()?;
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        let index = AgentMemoryIndex::new(brief.agent_memory, brief.input_hash);
        let temp = tempfile::TempDir::new()?;
        let research_dir = temp.path().join(".lithograph/research");

        index.persist(&research_dir)?;
        let first_modified = std::fs::metadata(agent_memory_path(temp.path()))?.modified()?;
        index.persist(&research_dir)?;
        let second_modified = std::fs::metadata(agent_memory_path(temp.path()))?.modified()?;

        assert_eq!(first_modified, second_modified);

        Ok(())
    }
}
