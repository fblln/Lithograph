//! Agent-style repository research artifacts used as the intermediate
//! memory layer between graph indexing and documentation composition.

use crate::domain::{Artifact, ArtifactCategory, SupportTier};
use crate::graph::{Graph, GraphNode, RelationKind};
use crate::inventory::language::{RegistryIndexTier, by_name as registry_language};
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
}

impl AgentMemory {
    /// Persists one JSON file per agent, plus a combined `agent-memory.json`.
    pub fn persist(&self, research_dir: &Path) -> std::io::Result<()> {
        JsonStore.write_if_changed(&research_dir.join("agent-memory.json"), self)?;
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
        Ok(())
    }
}

/// Minimal deterministic agent interface mirroring deepwiki-rs' staged shape.
pub(crate) trait KnowledgeAgent {
    /// Output report type.
    type Output: Serialize + Clone + PartialEq + Eq;

    /// Stable agent name and memory key.
    fn memory_key(&self) -> &'static str;

    /// Computes this agent's output from already-indexed facts and earlier reports.
    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output;
}

pub(crate) struct ResearchInput<'a> {
    artifacts: &'a [Artifact],
    graph: &'a Graph,
    modules: &'a [DocumentationModule],
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
}

/// Builds deterministic research artifacts from the already-validated graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResearchBuilder;

impl ResearchBuilder {
    /// Derives compact agent-style reports without calling a model.
    pub fn build(
        &self,
        artifacts: &[Artifact],
        graph: &Graph,
        modules: &[DocumentationModule],
    ) -> ResearchBrief {
        let input = ResearchInput {
            artifacts,
            graph,
            modules,
        };
        let system_context = execute_agent(SystemContextResearcher, &input);
        let domain_modules = execute_agent(DomainModulesDetector, &input);
        let architecture = execute_agent(ArchitectureResearcher, &input);
        let workflows = execute_agent(WorkflowResearcher, &input);
        let boundaries = execute_agent(BoundaryAnalyzer, &input);
        let key_modules = execute_agent(KeyModulesInsight, &input);
        let database = execute_agent(DatabaseOverviewAnalyzer, &input);
        let database = if database.database_facts.is_empty() {
            None
        } else {
            Some(database)
        };
        let agent_memory = AgentMemory {
            system_context,
            domain_modules,
            architecture,
            workflows: workflows.clone(),
            boundaries: boundaries.clone(),
            key_modules: key_modules.clone(),
            database,
        };

        ResearchBrief {
            system_context: legacy_system_context(&agent_memory),
            workflows: workflows.workflows,
            boundaries: boundaries.boundaries,
            configuration: configuration(artifacts, graph),
            key_modules: key_modules.modules,
            agent_memory,
        }
    }
}

fn execute_agent<A: KnowledgeAgent>(agent: A, input: &ResearchInput<'_>) -> A::Output {
    let _memory_key = agent.memory_key();
    agent.execute(input)
}

#[derive(Debug, Clone, Copy, Default)]
struct SystemContextResearcher;

impl KnowledgeAgent for SystemContextResearcher {
    type Output = SystemContextReport;

    fn memory_key(&self) -> &'static str {
        "system-context"
    }

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        let components: Vec<String> = input
            .modules
            .iter()
            .take(12)
            .map(|module| format!("{} ({:?})", module.name, module.kind))
            .collect();
        let external_systems = input
            .graph
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
                input.artifacts.len(),
                input.graph.nodes.len(),
                input.graph.relations.len(),
                input.modules.len()
            ),
            included_components: components,
            external_systems,
            evidence: artifact_evidence(input.artifacts, 8),
            confidence: confidence_for(input.artifacts.len()),
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

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        let modules = input
            .modules
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
        let labels = node_labels(input.graph);
        let relations = input
            .graph
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
            modules,
            relations,
            evidence: artifact_evidence(input.artifacts, 8),
            confidence: confidence_for(input.modules.len()),
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

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        let languages = language_support(input.artifacts);
        let mut architecture_facts = Vec::new();
        architecture_facts.push(format!(
            "knowledge graph schema: {} node(s), {} relation(s)",
            input.graph.nodes.len(),
            input.graph.relations.len()
        ));
        architecture_facts.extend(input.modules.iter().take(10).map(|module| {
            format!(
                "container/component candidate: {} ({:?})",
                module.name, module.kind
            )
        }));
        let decisions_and_docs = input
            .artifacts
            .iter()
            .filter(|artifact| artifact.category == ArtifactCategory::Documentation)
            .filter(|artifact| {
                let path = artifact.path.as_str().to_lowercase();
                path.contains("architecture") || path.contains("adr") || path.contains("decision")
            })
            .map(|artifact| format!("existing architecture knowledge: {}", artifact.path))
            .take(20)
            .collect();
        let hotspots = key_modules(input.modules, input.graph);
        Self::Output {
            languages,
            architecture_facts,
            hotspots,
            decisions_and_docs,
            mermaid: architecture_mermaid(input.modules, input.graph),
            evidence: artifact_evidence(input.artifacts, 12),
            confidence: confidence_for(input.graph.nodes.len()),
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

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        Self::Output {
            workflows: workflows(input.graph),
            evidence: relation_evidence(input.graph, 12),
            confidence: confidence_for(input.graph.relations.len()),
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

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        Self::Output {
            boundaries: boundaries(input.graph),
            evidence: relation_evidence(input.graph, 12),
            confidence: confidence_for(input.graph.relations.len()),
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

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        Self::Output {
            modules: key_modules(input.modules, input.graph),
            evidence: artifact_evidence(input.artifacts, 10),
            confidence: confidence_for(input.modules.len()),
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

    fn execute(&self, input: &ResearchInput<'_>) -> Self::Output {
        let database_facts = input
            .artifacts
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
            evidence: artifact_evidence(input.artifacts, 10),
            confidence: 80,
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
    use super::{LanguageSupportTier, ResearchBuilder, agent_memory_path};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::plan::ModulePlanner;
    use std::path::Path;

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

        brief
            .agent_memory
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
        assert_eq!(go.tier, LanguageSupportTier::Detected);
        assert_eq!(go.target_tier, LanguageSupportTier::HybridResolved);
        assert_eq!(sql.tier, LanguageSupportTier::Detected);
        assert_eq!(sql.target_tier, LanguageSupportTier::SyntaxIndexed);

        Ok(())
    }
}
