//! Bounded, evidence-backed model context and prompt construction for
//! module and repository-wide wiki pages.

use crate::adr::AdrSummary;
use crate::domain::{Artifact, ModelExposurePolicy, TextStatus};
use crate::drift::DriftReport;
use crate::generation::llm::ModelRequest;
use crate::graph::{Graph, GraphNode, GraphNodeId};
use crate::inventory::SafetyPolicy;
use crate::manifest::TaskKind;
use crate::plan::DocumentationModule;
use crate::research::ResearchBrief;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// Extra context for the architecture page only (LIT-22.7.1): documentation
/// drift findings and existing ADR summaries, so the page can validate
/// implementation intent against what's actually recorded (AC3) and
/// surface both as explicit Risks/Drift sections (AC1). `None` for every
/// other page kind, since drift/ADR scanning is only worth the cost when
/// actually building the architecture page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchitectureViewContext {
    /// Documentation drift findings for the current repository state.
    pub drift: DriftReport,
    /// Summaries of every persisted architecture decision record.
    pub adr_summaries: Vec<AdrSummary>,
}

/// Maximum lines of a single artifact's content included as an excerpt.
const MAX_EXCERPT_LINES: usize = 120;
/// Maximum relation lines listed per section, so a highly-connected module
/// can't blow out the prompt budget.
const MAX_RELATION_LINES: usize = 40;

/// One artifact excerpt actually sent to the model, recorded so tests (and
/// later the evidence validator) can prove what evidence was available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextExcerpt {
    /// Repository-relative artifact path.
    pub artifact_path: String,
    /// Model exposure policy applied when selecting this excerpt.
    pub policy: ModelExposurePolicy,
    /// Lines actually included.
    pub included_lines: usize,
    /// True when the artifact had more content than `included_lines`.
    pub truncated: bool,
}

/// A fully assembled, bounded model context for one page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelContext {
    /// System/instruction prompt (schema + evidence rules).
    pub system_prompt: String,
    /// User/context prompt: members, summaries, excerpts, relations.
    pub user_prompt: String,
    /// Every artifact excerpt actually included in `user_prompt`.
    pub excerpts: Vec<ContextExcerpt>,
    /// Hash over the context's inputs.
    pub input_hash: String,
    /// Page category this context is for.
    pub task_kind: TaskKind,
}

impl ModelContext {
    /// Converts this context into a [`ModelRequest`] for a specific model/prompt version.
    pub fn into_request(
        self,
        model: impl Into<String>,
        prompt_version: impl Into<String>,
    ) -> ModelRequest {
        ModelRequest {
            model: model.into(),
            prompt_version: prompt_version.into(),
            task_kind: self.task_kind,
            input_hash: self.input_hash,
            system_prompt: self.system_prompt,
            user_prompt: self.user_prompt,
        }
    }
}

/// Builds bounded, evidence-backed contexts for documentation generation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContextBuilder;

impl ContextBuilder {
    /// Builds the leaf context for one module: members, summaries,
    /// source/config/doc excerpts, inbound/outbound relations, unresolved
    /// references, and existing docs evidence.
    pub fn build_module_context(
        &self,
        module: &DocumentationModule,
        graph: &Graph,
        artifacts: &[Artifact],
        repo_root: &Path,
    ) -> ModelContext {
        let artifact_by_path: BTreeMap<&str, &Artifact> = artifacts
            .iter()
            .map(|artifact| (artifact.path.as_str(), artifact))
            .collect();
        let node_by_id: BTreeMap<&GraphNodeId, &GraphNode> =
            graph.nodes.iter().map(|node| (node.id(), node)).collect();
        let member_set: std::collections::BTreeSet<&GraphNodeId> = module.members.iter().collect();

        let mut sections = Vec::new();
        sections.push(format!("## Module: {} ({:?})\n", module.name, module.kind));

        let mut excerpts = Vec::new();
        let mut symbol_lines = Vec::new();
        let mut summary_lines = Vec::new();
        let mut unresolved_lines = Vec::new();

        for member_id in &module.members {
            let Some(node) = node_by_id.get(member_id) else {
                continue;
            };
            match node {
                GraphNode::Artifact(artifact_node) => {
                    if let Some(artifact) = artifact_by_path.get(artifact_node.path.as_str()) {
                        if let Some(excerpt) = read_excerpt(artifact, repo_root) {
                            sections.push(format!(
                                "### {} ({:?})\nEVIDENCE: {}\n```\n{}\n```\n",
                                artifact.path, artifact.category, artifact.path, excerpt.text
                            ));
                            excerpts.push(excerpt.record);
                        } else {
                            summary_lines.push(format!(
                                "- {} (no content included: {:?})",
                                artifact.path, artifact.model_policy
                            ));
                        }
                    }
                }
                // LIT-46: the author's own words about why the code is
                // this way -- the one input no parser can recover, so it is
                // quoted verbatim with its source span rather than
                // summarized.
                GraphNode::Rationale(rationale) => {
                    summary_lines.push(format!(
                        "- {} `{}` ({}): {}",
                        rationale.kind.id(),
                        rationale.evidence.path,
                        rationale
                            .evidence
                            .span
                            .as_ref()
                            .map_or_else(|| "whole file".to_owned(), |span| span.to_string()),
                        rationale.text
                    ));
                }
                GraphNode::Symbol(symbol) => {
                    let doc = symbol.doc.as_deref().unwrap_or("(no docstring)");
                    symbol_lines.push(format!(
                        "- {:?} `{}`: {}",
                        symbol.kind, symbol.qualified_name, doc
                    ));
                }
                GraphNode::Config(config) => {
                    summary_lines.push(format!("- config {:?} `{}`", config.kind, config.name));
                }
                GraphNode::Documentation(doc) => {
                    summary_lines.push(format!("- doc heading `{}`", doc.title));
                }
                GraphNode::Container(image) => {
                    summary_lines.push(format!("- container image `{}`", image.reference));
                }
                GraphNode::Command(command) => {
                    summary_lines.push(format!("- command `{}`", command.text));
                }
                GraphNode::EnvVar(env) => {
                    summary_lines.push(format!("- env var `{}`", env.name));
                }
                GraphNode::Package(package) => {
                    summary_lines.push(format!(
                        "- package `{}`{}",
                        package.name,
                        if package.is_external {
                            " (external)"
                        } else {
                            ""
                        }
                    ));
                }
                GraphNode::Unresolved(unresolved) => {
                    unresolved_lines.push(format!("- `{}`", unresolved.value));
                }
                GraphNode::Module(_) => {}
            }
        }

        if !symbol_lines.is_empty() {
            sections.push(format!("### Symbols\n{}\n", symbol_lines.join("\n")));
        }
        if !summary_lines.is_empty() {
            sections.push(format!("### Other members\n{}\n", summary_lines.join("\n")));
        }

        let relation_lines = relation_lines(graph, &member_set, MAX_RELATION_LINES);
        if !relation_lines.is_empty() {
            sections.push(format!(
                "### Relations (inbound and outbound)\n{}\n",
                relation_lines.join("\n")
            ));
        }
        if !unresolved_lines.is_empty() {
            sections.push(format!(
                "### Unresolved references\n{}\n",
                unresolved_lines.join("\n")
            ));
        }

        ModelContext {
            system_prompt: system_prompt(TaskKind::ModulePage),
            user_prompt: sections.join("\n"),
            excerpts,
            input_hash: module.input_hash.clone(),
            task_kind: TaskKind::ModulePage,
        }
    }

    /// Builds repository-wide summary contexts: child module summaries,
    /// graph-derived facts, deterministic diagrams, and path-only evidence
    /// records. Summary contexts intentionally do not include full source
    /// excerpts, but they still expose source paths for bare-path evidence
    /// validation.
    pub fn build_summary_context(
        &self,
        task_kind: TaskKind,
        modules: &[DocumentationModule],
        graph: &Graph,
        artifacts: &[Artifact],
        research: Option<&ResearchBrief>,
        architecture_context: Option<&ArchitectureViewContext>,
    ) -> ModelContext {
        let owner_by_member: BTreeMap<&GraphNodeId, &str> = modules
            .iter()
            .flat_map(|module| {
                module
                    .members
                    .iter()
                    .map(move |member| (member, module.id.as_str()))
            })
            .collect();
        let artifact_by_path: BTreeMap<&str, &Artifact> = artifacts
            .iter()
            .map(|artifact| (artifact.path.as_str(), artifact))
            .collect();
        let node_by_id: BTreeMap<&GraphNodeId, &GraphNode> =
            graph.nodes.iter().map(|node| (node.id(), node)).collect();

        let mut sections = vec![format!("## Repository focus: {:?}\n", task_kind)];
        if let Some(research) = research {
            append_research_sections(&mut sections, task_kind, research, architecture_context);
        }
        sections.push("## Repository modules\n".to_owned());
        for module in modules {
            sections.push(format!(
                "- `{}` ({:?}, {} members, ~{} tokens)",
                module.name,
                module.kind,
                module.members.len(),
                module.estimated_tokens
            ));
        }

        let source_map = source_map_lines(modules, graph, &node_by_id, 20);
        if !source_map.is_empty() {
            sections.push(format!(
                "\n## High-level source map\n{}",
                source_map.join("\n")
            ));
        }

        let mut cross_module_lines = Vec::new();
        for relation in &graph.relations {
            let Some(source_owner) = owner_by_member.get(&relation.source) else {
                continue;
            };
            let Some(target_owner) = owner_by_member.get(&relation.target) else {
                continue;
            };
            if source_owner == target_owner {
                continue;
            }
            cross_module_lines.push(format!(
                "- {source_owner} -[{:?}]-> {target_owner} ({:?})",
                relation.kind, relation.confidence
            ));
            if cross_module_lines.len() >= MAX_RELATION_LINES {
                cross_module_lines.push("- ... (truncated)".to_owned());
                break;
            }
        }
        if !cross_module_lines.is_empty() {
            sections.push(format!(
                "\n## Cross-module relations\n{}",
                cross_module_lines.join("\n")
            ));
        }

        match task_kind {
            TaskKind::Overview => {
                sections.push(
                    "\n## Overview guidance\nSummarize repository purpose, major module responsibilities, entry points, and the source map. Keep setup detail brief and link readers to quickstart/configuration pages."
                        .to_owned(),
                );
            }
            TaskKind::Quickstart => {
                sections.push(
                    "\n## Quickstart guidance\nExplain how to identify setup, build, run, and test entry points from manifests, commands, containers, and configuration nodes."
                        .to_owned(),
                );
                append_configuration_sections(&mut sections, graph, &node_by_id);
            }
            TaskKind::Architecture => {
                if let Some(diagram) = module_relation_diagram(modules, graph, &owner_by_member) {
                    sections.push(format!(
                        "\n## Deterministic architecture diagram\n{diagram}"
                    ));
                }
            }
            TaskKind::Workflows => {
                append_workflow_sections(&mut sections, graph, &node_by_id);
                if let Some(diagram) = workflow_diagram(graph, &node_by_id) {
                    sections.push(format!("\n## Deterministic workflow diagram\n{diagram}"));
                }
            }
            TaskKind::Boundaries => {
                append_boundary_sections(&mut sections, graph, &node_by_id);
                if let Some(diagram) = boundary_diagram(graph, &node_by_id) {
                    sections.push(format!("\n## Deterministic boundary diagram\n{diagram}"));
                }
            }
            TaskKind::Configuration => {
                append_configuration_sections(&mut sections, graph, &node_by_id);
            }
            TaskKind::Database => {
                sections.push(
                    "\n## Database guidance\nSummarize database schemas, migrations, and SQL evidence strictly from the given facts; state plainly when no database evidence was found."
                        .to_owned(),
                );
            }
            TaskKind::KeyModules => {
                sections.push(
                    "\n## Key modules guidance\nHighlight the largest or most connected modules and why they matter, strictly from the given facts."
                        .to_owned(),
                );
            }
            TaskKind::AdrDrift => {
                sections.push(
                    "\n## Architecture decisions and drift guidance\nSummarize recorded architecture decisions and documentation drift findings strictly from the given facts; state plainly when none exist."
                        .to_owned(),
                );
            }
            TaskKind::ModulePage => {}
        }

        let (evidence_lines, excerpts) = summary_evidence(graph, &artifact_by_path, 40);
        if !evidence_lines.is_empty() {
            sections.push(format!(
                "\n## Source evidence candidates\n{}",
                evidence_lines.join("\n")
            ));
        }

        let input_hash = summary_input_hash(modules);
        ModelContext {
            system_prompt: system_prompt(task_kind),
            user_prompt: sections.join("\n"),
            excerpts,
            input_hash,
            task_kind,
        }
    }
}

fn append_research_sections(
    sections: &mut Vec<String>,
    task_kind: TaskKind,
    research: &ResearchBrief,
    architecture_context: Option<&ArchitectureViewContext>,
) {
    append_agent_memory_sections(sections, task_kind, research, architecture_context);

    let facts = match task_kind {
        TaskKind::Overview | TaskKind::Architecture => &research.system_context,
        TaskKind::Workflows => &research.workflows,
        TaskKind::Boundaries => &research.boundaries,
        TaskKind::Configuration | TaskKind::Quickstart | TaskKind::Database => {
            &research.configuration
        }
        TaskKind::ModulePage | TaskKind::KeyModules => &research.key_modules,
        TaskKind::AdrDrift => &research.boundaries,
    };
    if !facts.is_empty() {
        sections.push(format!(
            "\n## Research memory\n{}",
            facts
                .iter()
                .take(30)
                .map(|fact| format!("- {fact}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !research.key_modules.is_empty() && task_kind != TaskKind::Configuration {
        sections.push(format!(
            "\n## Key module research\n{}",
            research
                .key_modules
                .iter()
                .take(10)
                .map(|fact| format!("- {fact}"))
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
}

fn append_agent_memory_sections(
    sections: &mut Vec<String>,
    task_kind: TaskKind,
    research: &ResearchBrief,
    architecture_context: Option<&ArchitectureViewContext>,
) {
    let memory = &research.agent_memory;
    match task_kind {
        TaskKind::Overview => {
            sections.push(format!(
                "\n## Agent research: system context\n- Summary: {}\n- Confidence: {}/100\n- Included components: {}",
                memory.system_context.project_summary,
                memory.system_context.confidence,
                join_or_none(&memory.system_context.included_components)
            ));
        }
        TaskKind::Architecture => {
            append_c4_architecture_sections(sections, memory, architecture_context);
        }
        TaskKind::Workflows => {
            sections.push(format!(
                "\n## Agent research: workflow memory\n{}",
                bullet_lines(&memory.workflows.workflows)
            ));
        }
        TaskKind::Boundaries => {
            sections.push(format!(
                "\n## Agent research: boundary memory\n{}",
                bullet_lines(&memory.boundaries.boundaries)
            ));
        }
        TaskKind::Configuration | TaskKind::Quickstart => {
            if let Some(database) = &memory.database {
                sections.push(format!(
                    "\n## Agent research: database memory\n{}",
                    bullet_lines(&database.database_facts)
                ));
            }
        }
        TaskKind::Database => {
            let facts = match &memory.database {
                Some(database) => bullet_lines(&database.database_facts),
                None => "- no database schema, migration, or SQL evidence was found".to_owned(),
            };
            sections.push(format!("\n## Database\n{facts}"));
        }
        TaskKind::KeyModules => {
            sections.push(format!(
                "\n## Key Modules\n{}",
                bullet_lines(&memory.key_modules.modules)
            ));
        }
        TaskKind::AdrDrift => {
            append_adr_and_drift_sections(sections, architecture_context);
        }
        TaskKind::ModulePage => {
            sections.push(format!(
                "\n## Agent research: key module memory\n{}",
                bullet_lines(&memory.key_modules.modules)
            ));
        }
    }
}

/// Builds the architecture-decisions-and-drift page's content
/// (LIT-22.7.3 AC1): real persisted ADR summaries and real documentation
/// drift findings, or an explicit "none found" statement rather than a
/// fabricated summary.
fn append_adr_and_drift_sections(
    sections: &mut Vec<String>,
    architecture_context: Option<&ArchitectureViewContext>,
) {
    let Some(context) = architecture_context else {
        sections.push(
            "\n## Architecture Decisions\n- none recorded\n\n## Drift\n- no documentation drift detected"
                .to_owned(),
        );
        return;
    };
    let adr_lines: Vec<String> = context
        .adr_summaries
        .iter()
        .map(|adr| format!("ADR {} [{:?}]: {}", adr.id, adr.status, adr.title))
        .collect();
    sections.push(format!(
        "\n## Architecture Decisions\n{}",
        bullet_lines(&adr_lines)
    ));

    let drift_lines: Vec<String> = context
        .drift
        .findings
        .iter()
        .map(|finding| {
            format!(
                "{:?}: {} ({})",
                finding.kind, finding.detail, finding.artifact_path
            )
        })
        .collect();
    sections.push(format!("\n## Drift\n{}", bullet_lines(&drift_lines)));
}

/// Builds the architecture page's explicit C4-oriented sections
/// (LIT-22.7.1 AC1): System Context, Container View, Component View,
/// Deployment/Runtime View, Workflows, Risks, and Drift, plus existing
/// architecture docs/ADRs used to validate implementation intent (AC3).
fn append_c4_architecture_sections(
    sections: &mut Vec<String>,
    memory: &crate::research::AgentMemory,
    architecture_context: Option<&ArchitectureViewContext>,
) {
    let language_lines: Vec<String> = memory
        .architecture
        .languages
        .iter()
        .map(|language| {
            format!(
                "- {}: {:?} ({} artifact(s))",
                language.language, language.tier, language.artifact_count
            )
        })
        .collect();
    let domain_lines: Vec<String> = memory
        .domain_modules
        .modules
        .iter()
        .take(20)
        .map(|module| {
            format!(
                "- {} ({}) owns {} member(s); evidence: {}",
                module.name,
                module.kind,
                module.member_count,
                join_or_none(&module.evidence)
            )
        })
        .collect();
    sections.push(format!(
        "\n## System Context\n{}\n\n### Language support tiers\n{}\n\n### Domain modules\n{}",
        memory.system_context.project_summary,
        language_lines.join("\n"),
        domain_lines.join("\n"),
    ));
    sections.push(format!(
        "\n## Container View\n{}",
        bullet_lines(&memory.architecture.architecture_facts)
    ));
    sections.push(format!(
        "\n## Component View\n{}",
        bullet_lines(&memory.architecture.hotspots)
    ));
    let deployment_lines = match &memory.deployment {
        Some(deployment) => bullet_lines(&deployment.deployment_facts),
        None => "- no container/deployment evidence found".to_owned(),
    };
    sections.push(format!(
        "\n## Deployment / Runtime View\n{deployment_lines}"
    ));
    sections.push(format!(
        "\n## Workflows\n{}",
        bullet_lines(&memory.workflows.workflows)
    ));

    let mut risk_lines: Vec<String> = [
        ("system context", memory.system_context.confidence),
        ("architecture", memory.architecture.confidence),
        ("workflows", memory.workflows.confidence),
        ("boundaries", memory.boundaries.confidence),
    ]
    .into_iter()
    .filter(|&(_, confidence)| confidence < 60)
    .map(|(label, confidence)| format!("- low confidence in {label} research ({confidence}/100)"))
    .collect();
    if !memory.system_context.external_systems.is_empty() {
        risk_lines.push(format!(
            "- {} external system(s)/unresolved reference(s) not fully resolved",
            memory.system_context.external_systems.len()
        ));
    }
    if let Some(context) = architecture_context
        && !context.drift.findings.is_empty()
    {
        risk_lines.push(format!(
            "- {} documentation drift finding(s) detected",
            context.drift.findings.len()
        ));
    }
    sections.push(format!("\n## Risks\n{}", bullet_lines(&risk_lines)));

    let drift_lines: Vec<String> = architecture_context
        .map(|context| {
            context
                .drift
                .findings
                .iter()
                .map(|finding| {
                    format!(
                        "- {:?}: {} ({})",
                        finding.kind, finding.detail, finding.artifact_path
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    sections.push(format!(
        "\n## Drift\n{}",
        if drift_lines.is_empty() {
            "- no documentation drift detected".to_owned()
        } else {
            drift_lines.join("\n")
        }
    ));

    let mut knowledge_lines = memory.architecture.decisions_and_docs.clone();
    if let Some(context) = architecture_context {
        knowledge_lines.extend(
            context
                .adr_summaries
                .iter()
                .map(|adr| format!("ADR {} [{:?}]: {}", adr.id, adr.status, adr.title)),
        );
    }
    if !knowledge_lines.is_empty() {
        sections.push(format!(
            "\n## Existing architecture knowledge and ADRs\n{}",
            bullet_lines(&knowledge_lines)
        ));
    }
    if let Some(diagram) = &memory.architecture.mermaid {
        sections.push(format!("\n## Agent-authored C4 seed diagram\n{diagram}"));
    }
}

fn bullet_lines(lines: &[String]) -> String {
    if lines.is_empty() {
        return "- none observed".to_owned();
    }
    lines
        .iter()
        .take(30)
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn join_or_none(lines: &[String]) -> String {
    if lines.is_empty() {
        "none observed".to_owned()
    } else {
        lines.iter().take(8).cloned().collect::<Vec<_>>().join(", ")
    }
}

struct ExcerptResult {
    text: String,
    record: ContextExcerpt,
}

fn read_excerpt(artifact: &Artifact, repo_root: &Path) -> Option<ExcerptResult> {
    if artifact.model_policy == ModelExposurePolicy::Never
        || artifact.text_status != TextStatus::Text
    {
        return None;
    }
    let content = std::fs::read_to_string(repo_root.join(artifact.path.as_str())).ok()?;
    let content = if artifact.model_policy == ModelExposurePolicy::Redacted {
        SafetyPolicy.redact_text(&content)
    } else {
        content
    };

    let total_lines = content.lines().count();
    let take = MAX_EXCERPT_LINES.min(total_lines);
    let text: String = content.lines().take(take).collect::<Vec<_>>().join("\n");
    let truncated = total_lines > take;

    Some(ExcerptResult {
        text,
        record: ContextExcerpt {
            artifact_path: artifact.path.as_str().to_owned(),
            policy: artifact.model_policy,
            included_lines: take,
            truncated,
        },
    })
}

fn relation_lines(graph: &Graph, member_set: &BTreeSet<&GraphNodeId>, limit: usize) -> Vec<String> {
    let mut lines = Vec::new();
    for relation in &graph.relations {
        if !member_set.contains(&relation.source) && !member_set.contains(&relation.target) {
            continue;
        }
        lines.push(format!(
            "- {} -[{:?}]-> {} ({:?})",
            display_id(&relation.source),
            relation.kind,
            display_id(&relation.target),
            relation.confidence
        ));
        if lines.len() >= limit {
            lines.push("- ... (truncated)".to_owned());
            break;
        }
    }
    lines
}

fn display_id(id: &GraphNodeId) -> &str {
    id.as_str()
        .split_once(':')
        .map_or(id.as_str(), |(_, rest)| rest)
}

fn source_map_lines(
    modules: &[DocumentationModule],
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
    limit: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for module in modules {
        let artifacts: Vec<String> = module
            .members
            .iter()
            .filter_map(|id| match node_by_id.get(id) {
                Some(GraphNode::Artifact(artifact)) => Some(artifact.path.clone()),
                _ => None,
            })
            .take(4)
            .collect();
        lines.push(format!(
            "- `{}` owns {} graph member(s); representative files: {}",
            module.name,
            module.members.len(),
            if artifacts.is_empty() {
                "(none)".to_owned()
            } else {
                artifacts.join(", ")
            }
        ));
        if lines.len() >= limit {
            lines.push("- ... (truncated)".to_owned());
            break;
        }
    }
    if lines.is_empty() && !graph.nodes.is_empty() {
        lines.push(format!(
            "- Repository graph contains {} node(s) and {} relation(s).",
            graph.nodes.len(),
            graph.relations.len()
        ));
    }
    lines
}

fn append_workflow_sections(
    sections: &mut Vec<String>,
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
) {
    let mut command_lines = Vec::new();
    let mut container_lines = Vec::new();
    for node in &graph.nodes {
        match node {
            GraphNode::Command(command) => command_lines.push(format!("- `{}`", command.text)),
            GraphNode::Container(container) => {
                container_lines.push(format!("- `{}`", container.reference))
            }
            _ => {}
        }
    }
    if !command_lines.is_empty() {
        sections.push(format!(
            "\n## Commands and execution entry points\n{}",
            command_lines
                .into_iter()
                .take(MAX_RELATION_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !container_lines.is_empty() {
        sections.push(format!(
            "\n## Container/runtime nodes\n{}",
            container_lines
                .into_iter()
                .take(MAX_RELATION_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let flow_relations: Vec<String> = graph
        .relations
        .iter()
        .filter(|relation| {
            matches!(
                relation.kind,
                crate::graph::RelationKind::RunsCommand
                    | crate::graph::RelationKind::UsesImage
                    | crate::graph::RelationKind::BuildsImage
                    | crate::graph::RelationKind::PublishesImage
                    | crate::graph::RelationKind::ReadsEnv
            )
        })
        .map(|relation| {
            format!(
                "- {} -[{:?}]-> {} ({:?})",
                node_label(&relation.source, node_by_id),
                relation.kind,
                node_label(&relation.target, node_by_id),
                relation.confidence
            )
        })
        .take(MAX_RELATION_LINES)
        .collect();
    if !flow_relations.is_empty() {
        sections.push(format!(
            "\n## Workflow relations\n{}",
            flow_relations.join("\n")
        ));
    }
}

fn append_boundary_sections(
    sections: &mut Vec<String>,
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
) {
    let mut package_lines = Vec::new();
    let mut env_lines = Vec::new();
    let mut unresolved_lines = Vec::new();
    for node in &graph.nodes {
        match node {
            GraphNode::Package(package) if package.is_external => {
                package_lines.push(format!("- external package `{}`", package.name));
            }
            GraphNode::EnvVar(env) => env_lines.push(format!("- env var `{}`", env.name)),
            GraphNode::Unresolved(unresolved) => {
                unresolved_lines.push(format!("- unresolved `{}`", unresolved.value));
            }
            _ => {}
        }
    }
    if !package_lines.is_empty() {
        sections.push(format!(
            "\n## External packages\n{}",
            package_lines
                .into_iter()
                .take(MAX_RELATION_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !env_lines.is_empty() {
        sections.push(format!(
            "\n## Environment boundary\n{}",
            env_lines
                .into_iter()
                .take(MAX_RELATION_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }
    if !unresolved_lines.is_empty() {
        sections.push(format!(
            "\n## Unresolved boundaries\n{}",
            unresolved_lines
                .into_iter()
                .take(MAX_RELATION_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let boundary_relations: Vec<String> = graph
        .relations
        .iter()
        .filter(|relation| {
            let target = node_by_id.get(&relation.target);
            matches!(
                target,
                Some(GraphNode::Package(package)) if package.is_external
            ) || matches!(
                target,
                Some(GraphNode::EnvVar(_)) | Some(GraphNode::Unresolved(_))
            )
        })
        .map(|relation| {
            format!(
                "- {} -[{:?}]-> {} ({:?})",
                node_label(&relation.source, node_by_id),
                relation.kind,
                node_label(&relation.target, node_by_id),
                relation.confidence
            )
        })
        .take(MAX_RELATION_LINES)
        .collect();
    if !boundary_relations.is_empty() {
        sections.push(format!(
            "\n## Boundary relations\n{}",
            boundary_relations.join("\n")
        ));
    }
}

fn append_configuration_sections(
    sections: &mut Vec<String>,
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
) {
    let mut config_lines = Vec::new();
    for node in &graph.nodes {
        match node {
            GraphNode::Artifact(artifact)
                if matches!(
                    artifact.category,
                    crate::domain::ArtifactCategory::Configuration
                        | crate::domain::ArtifactCategory::BuildDefinition
                        | crate::domain::ArtifactCategory::PackageManifest
                        | crate::domain::ArtifactCategory::DependencyLockfile
                        | crate::domain::ArtifactCategory::ContainerDefinition
                        | crate::domain::ArtifactCategory::DeploymentDefinition
                        | crate::domain::ArtifactCategory::ContinuousIntegration
                ) =>
            {
                config_lines.push(format!("- {:?}: `{}`", artifact.category, artifact.path));
            }
            GraphNode::Config(config) => {
                config_lines.push(format!("- {:?}: `{}`", config.kind, config.name));
            }
            GraphNode::EnvVar(env) => {
                config_lines.push(format!("- env var: `{}`", env.name));
            }
            GraphNode::Package(package) => {
                config_lines.push(format!(
                    "- package: `{}`{}",
                    package.name,
                    if package.is_external {
                        " (external)"
                    } else {
                        ""
                    }
                ));
            }
            _ => {}
        }
    }
    if !config_lines.is_empty() {
        sections.push(format!(
            "\n## Configuration and deployment inputs\n{}",
            config_lines
                .into_iter()
                .take(MAX_RELATION_LINES)
                .collect::<Vec<_>>()
                .join("\n")
        ));
    }

    let config_relations: Vec<String> = graph
        .relations
        .iter()
        .filter(|relation| {
            matches!(
                relation.kind,
                crate::graph::RelationKind::ReadsEnv
                    | crate::graph::RelationKind::RunsCommand
                    | crate::graph::RelationKind::UsesImage
                    | crate::graph::RelationKind::BuildsImage
                    | crate::graph::RelationKind::DependsOnPackage
            )
        })
        .map(|relation| {
            format!(
                "- {} -[{:?}]-> {} ({:?})",
                node_label(&relation.source, node_by_id),
                relation.kind,
                node_label(&relation.target, node_by_id),
                relation.confidence
            )
        })
        .take(MAX_RELATION_LINES)
        .collect();
    if !config_relations.is_empty() {
        sections.push(format!(
            "\n## Configuration relations\n{}",
            config_relations.join("\n")
        ));
    }
}

fn summary_evidence(
    graph: &Graph,
    artifact_by_path: &BTreeMap<&str, &Artifact>,
    limit: usize,
) -> (Vec<String>, Vec<ContextExcerpt>) {
    let mut paths = BTreeSet::new();
    for node in &graph.nodes {
        match node {
            GraphNode::Artifact(artifact) => {
                paths.insert(artifact.path.as_str());
            }
            GraphNode::Symbol(symbol) => {
                paths.insert(symbol.evidence.path.as_str());
            }
            GraphNode::Config(config) => {
                paths.insert(config.evidence.path.as_str());
            }
            GraphNode::Documentation(doc) => {
                paths.insert(doc.evidence.path.as_str());
            }
            GraphNode::Command(command) => {
                paths.insert(command.evidence.path.as_str());
            }
            GraphNode::Module(module) => {
                paths.insert(module.evidence.path.as_str());
            }
            _ => {}
        }
    }

    let mut lines = Vec::new();
    let mut excerpts = Vec::new();
    for path in paths.into_iter().take(limit) {
        let policy = artifact_by_path
            .get(path)
            .map_or(ModelExposurePolicy::Allowed, |artifact| {
                artifact.model_policy
            });
        if policy == ModelExposurePolicy::Never {
            continue;
        }
        lines.push(format!("- EVIDENCE: {path}"));
        excerpts.push(ContextExcerpt {
            artifact_path: path.to_owned(),
            policy,
            included_lines: 0,
            truncated: true,
        });
    }
    if lines.len() == limit {
        lines.push("- ... (truncated)".to_owned());
    }
    (lines, excerpts)
}

fn module_relation_diagram(
    modules: &[DocumentationModule],
    graph: &Graph,
    owner_by_member: &BTreeMap<&GraphNodeId, &str>,
) -> Option<String> {
    let mut module_by_id: BTreeMap<&str, &DocumentationModule> = BTreeMap::new();
    for module in modules {
        module_by_id.insert(module.id.as_str(), module);
    }
    let mut lines = vec!["```mermaid".to_owned(), "flowchart TD".to_owned()];
    let mut ids: BTreeMap<&str, String> = BTreeMap::new();
    for (index, module) in modules.iter().enumerate().take(12) {
        let id = format!("M{index}");
        lines.push(format!("    {id}[\"{}\"]", mermaid_escape(&module.name)));
        ids.insert(module.id.as_str(), id);
    }

    let mut edges = BTreeSet::new();
    for relation in &graph.relations {
        let Some(source_owner) = owner_by_member.get(&relation.source) else {
            continue;
        };
        let Some(target_owner) = owner_by_member.get(&relation.target) else {
            continue;
        };
        if source_owner == target_owner {
            continue;
        }
        if module_by_id.contains_key(source_owner) && module_by_id.contains_key(target_owner) {
            edges.insert((*source_owner, *target_owner, relation.kind));
        }
    }
    for (source, target, kind) in edges.into_iter().take(20) {
        let Some(source_id) = ids.get(source) else {
            continue;
        };
        let Some(target_id) = ids.get(target) else {
            continue;
        };
        lines.push(format!("    {source_id} -- {:?} --> {target_id}", kind));
    }
    if lines.len() <= 2 {
        return None;
    }
    lines.push("```".to_owned());
    Some(lines.join("\n"))
}

fn workflow_diagram(
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
) -> Option<String> {
    relation_diagram(
        graph,
        node_by_id,
        |kind| {
            matches!(
                kind,
                crate::graph::RelationKind::RunsCommand
                    | crate::graph::RelationKind::UsesImage
                    | crate::graph::RelationKind::BuildsImage
                    | crate::graph::RelationKind::PublishesImage
            )
        },
        "flowchart LR",
    )
}

fn boundary_diagram(
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
) -> Option<String> {
    relation_diagram(
        graph,
        node_by_id,
        |kind| {
            matches!(
                kind,
                crate::graph::RelationKind::DependsOnPackage
                    | crate::graph::RelationKind::ReadsEnv
                    | crate::graph::RelationKind::References
            )
        },
        "flowchart LR",
    )
}

fn relation_diagram(
    graph: &Graph,
    node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>,
    include: impl Fn(crate::graph::RelationKind) -> bool,
    direction: &str,
) -> Option<String> {
    let mut lines = vec!["```mermaid".to_owned(), direction.to_owned()];
    let mut node_ids: BTreeMap<GraphNodeId, String> = BTreeMap::new();
    let mut next_index = 0usize;
    for relation in graph
        .relations
        .iter()
        .filter(|relation| include(relation.kind))
        .take(20)
    {
        for graph_id in [&relation.source, &relation.target] {
            if !node_ids.contains_key(graph_id) {
                let id = format!("N{next_index}");
                next_index += 1;
                lines.push(format!(
                    "    {id}[\"{}\"]",
                    mermaid_escape(&node_label(graph_id, node_by_id))
                ));
                node_ids.insert(graph_id.clone(), id);
            }
        }
        let Some(source_id) = node_ids.get(&relation.source) else {
            continue;
        };
        let Some(target_id) = node_ids.get(&relation.target) else {
            continue;
        };
        lines.push(format!(
            "    {source_id} -- {:?} --> {target_id}",
            relation.kind
        ));
    }
    if lines.len() <= 2 {
        return None;
    }
    lines.push("```".to_owned());
    Some(lines.join("\n"))
}

fn node_label(id: &GraphNodeId, node_by_id: &BTreeMap<&GraphNodeId, &GraphNode>) -> String {
    match node_by_id.get(id) {
        Some(GraphNode::Artifact(artifact)) => artifact.path.clone(),
        Some(GraphNode::Symbol(symbol)) => symbol.qualified_name.clone(),
        Some(GraphNode::Config(config)) => config.name.clone(),
        Some(GraphNode::Documentation(doc)) => doc.title.clone(),
        Some(GraphNode::Container(container)) => container.reference.clone(),
        Some(GraphNode::Command(command)) => command.text.clone(),
        Some(GraphNode::EnvVar(env)) => env.name.clone(),
        Some(GraphNode::Module(module)) => module.path.clone(),
        Some(GraphNode::Package(package)) => package.name.clone(),
        Some(GraphNode::Unresolved(unresolved)) => unresolved.value.clone(),
        Some(GraphNode::Rationale(rationale)) => rationale.text.clone(),
        None => display_id(id).to_owned(),
    }
}

fn mermaid_escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
}

fn summary_input_hash(modules: &[DocumentationModule]) -> String {
    let mut hashes: Vec<&str> = modules
        .iter()
        .map(|module| module.input_hash.as_str())
        .collect();
    hashes.sort_unstable();
    blake3::hash(hashes.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

fn system_prompt(task_kind: TaskKind) -> String {
    let role = match task_kind {
        TaskKind::Overview => {
            "You write the Lithograph repository overview page from graph-derived summaries below."
        }
        TaskKind::ModulePage => {
            "You write one Lithograph module documentation page from the module context below."
        }
        TaskKind::Architecture => {
            "You write the Lithograph repository architecture page from graph-derived summaries and deterministic diagrams below."
        }
        TaskKind::Quickstart => {
            "You write the Lithograph repository quickstart page from the module summaries below."
        }
        TaskKind::Workflows => {
            "You write the Lithograph repository workflow page from graph-derived commands, runtime nodes, relations, and deterministic diagrams below."
        }
        TaskKind::Boundaries => {
            "You write the Lithograph repository boundary and interface page from graph-derived external packages, env vars, unresolved references, and deterministic diagrams below."
        }
        TaskKind::Configuration => {
            "You write the Lithograph repository configuration and deployment page from graph-derived manifests, packages, env vars, and runtime configuration below."
        }
        TaskKind::Database => {
            "You write the Lithograph repository database overview page from graph-derived schema, migration, and SQL evidence below."
        }
        TaskKind::KeyModules => {
            "You write the Lithograph repository key-modules page from the largest and most connected modules below."
        }
        TaskKind::AdrDrift => {
            "You write the Lithograph repository architecture-decisions-and-drift page from recorded ADRs and documentation drift findings below."
        }
    };
    format!(
        "{role}\n\n\
         Respond with a single JSON object matching this schema exactly:\n\
         {{\"title\": string, \"summary\": string, \"evidence_refs\": string[], \
         \"unresolved_questions\": string[], \"body\": string (Markdown)}}\n\n\
         Rules:\n\
         - Only list a path in `evidence_refs` if it appears after an `EVIDENCE:` \
         line in the context below.\n\
         - For repository-wide summary pages, cite bare paths only; do not cite line \
         numbers unless source lines are shown in a fenced excerpt.\n\
         - Include a `## Source Evidence` section in `body` when evidence_refs is non-empty.\n\
         - Preserve deterministic Mermaid diagrams when they are present in the context.\n\
         - Never invent file paths, line numbers, or facts not present in the context.\n\
         - If something is unclear from the context, add it to `unresolved_questions` \
         instead of guessing.\n\
         - `body` must be self-contained Markdown."
    )
}

#[cfg(test)]
mod tests {
    use super::ContextBuilder;
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use crate::graph::GraphBuilder;
    use crate::graph::{
        ArtifactNode, EnvVarNode, Graph, GraphNode, GraphNodeId, Relation, RelationKind,
        UnresolvedNode,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::manifest::TaskKind;
    use crate::plan::{DocumentationModule, ModuleKind, ModulePlanner};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    #[test]
    fn module_context_includes_excerpts_symbols_relations_and_unresolved()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let python_module = modules
            .iter()
            .find(|module| module.kind == ModuleKind::PythonPackage)
            .ok_or("python module")?;

        let context = ContextBuilder.build_module_context(python_module, &graph, &artifacts, &root);

        assert!(
            context
                .user_prompt
                .contains("EVIDENCE: src/python_app/service.py")
        );
        assert!(context.user_prompt.contains("class RouteService"));
        assert!(context.user_prompt.contains("### Symbols"));
        assert!(context.user_prompt.contains("### Relations"));
        assert!(!context.excerpts.is_empty());
        assert_eq!(context.input_hash, python_module.input_hash);
        assert_eq!(context.task_kind, TaskKind::ModulePage);

        Ok(())
    }

    #[test]
    fn module_context_excludes_never_policy_artifacts_and_records_excerpts()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path().to_path_buf();
        std::fs::write(root.join("safe.txt"), "hello world\n")?;
        std::fs::write(
            root.join("README.md"),
            "sk-super-secret-token-do-not-leak\n",
        )?;
        let safe = Artifact::new(
            RepoPath::new("safe.txt")?,
            ArtifactCategory::Documentation,
            SupportTier::GenericText,
            ContentHash::new("abcdef")?,
            5,
        )
        .with_text_status(TextStatus::Text, Some(1));
        let secret = Artifact::new(
            RepoPath::new("README.md")?,
            ArtifactCategory::Configuration,
            SupportTier::GenericText,
            ContentHash::new("abcdef")?,
            5,
        )
        .with_text_status(TextStatus::UnsafeText, None)
        .with_model_policy(ModelExposurePolicy::Never);
        let artifacts = vec![safe.clone(), secret.clone()];

        let safe_id = GraphNodeId::new("artifact:safe.txt");
        let secret_id = GraphNodeId::new("artifact:README.md");
        let env_id = GraphNodeId::new("env:SECRET_TOKEN");
        let graph = Graph {
            nodes: vec![
                GraphNode::Artifact(ArtifactNode {
                    id: safe_id.clone(),
                    path: "safe.txt".to_owned(),
                    category: ArtifactCategory::Documentation,
                    evidence: file_evidence(&safe),
                }),
                GraphNode::Artifact(ArtifactNode {
                    id: secret_id.clone(),
                    path: "README.md".to_owned(),
                    category: ArtifactCategory::Configuration,
                    evidence: file_evidence(&secret),
                }),
                GraphNode::EnvVar(EnvVarNode {
                    id: env_id.clone(),
                    name: "SECRET_TOKEN".to_owned(),
                }),
                GraphNode::Unresolved(UnresolvedNode {
                    id: GraphNodeId::new("unresolved:mystery"),
                    value: "mystery".to_owned(),
                }),
            ],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source: secret_id.clone(),
                target: env_id.clone(),
                kind: RelationKind::ReadsEnv,
                confidence: crate::domain::Confidence::High,
                evidence: vec![file_evidence(&secret)],
                provenance: None,
            }],
        };
        let module = DocumentationModule {
            id: "module-plan:directory:root".to_owned(),
            name: "root".to_owned(),
            kind: ModuleKind::Directory,
            members: vec![
                safe_id,
                secret_id,
                env_id,
                GraphNodeId::new("unresolved:mystery"),
            ],
            input_hash: "hash".to_owned(),
            estimated_tokens: 10,
        };

        let context = ContextBuilder.build_module_context(&module, &graph, &artifacts, &root);

        assert!(context.user_prompt.contains("hello world"));
        assert!(
            !context
                .user_prompt
                .contains("sk-super-secret-token-do-not-leak")
        );
        assert!(
            context
                .excerpts
                .iter()
                .all(|excerpt| excerpt.artifact_path != "README.md")
        );
        assert!(context.user_prompt.contains("### Unresolved references"));
        assert!(context.user_prompt.contains("mystery"));

        Ok(())
    }

    #[test]
    fn summary_context_lists_modules_and_cross_module_relations_without_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let context = ContextBuilder.build_summary_context(
            TaskKind::Quickstart,
            &modules,
            &graph,
            &artifacts,
            None,
            None,
        );

        assert!(context.user_prompt.contains("## Repository modules"));
        assert!(
            context
                .user_prompt
                .contains("## Source evidence candidates")
        );
        assert!(context.user_prompt.contains("EVIDENCE:"));
        assert!(!context.excerpts.is_empty());
        assert!(!context.user_prompt.contains("```"));
        assert_eq!(context.task_kind, TaskKind::Quickstart);

        Ok(())
    }

    #[test]
    fn repository_contexts_include_page_specific_sections_and_diagrams()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let overview = ContextBuilder.build_summary_context(
            TaskKind::Overview,
            &modules,
            &graph,
            &artifacts,
            None,
            None,
        );
        let architecture = ContextBuilder.build_summary_context(
            TaskKind::Architecture,
            &modules,
            &graph,
            &artifacts,
            None,
            None,
        );
        let workflows = ContextBuilder.build_summary_context(
            TaskKind::Workflows,
            &modules,
            &graph,
            &artifacts,
            None,
            None,
        );
        let boundaries = ContextBuilder.build_summary_context(
            TaskKind::Boundaries,
            &modules,
            &graph,
            &artifacts,
            None,
            None,
        );
        let configuration = ContextBuilder.build_summary_context(
            TaskKind::Configuration,
            &modules,
            &graph,
            &artifacts,
            None,
            None,
        );

        assert!(overview.user_prompt.contains("## Overview guidance"));
        assert!(
            architecture
                .user_prompt
                .contains("## Deterministic architecture diagram")
        );
        assert!(architecture.user_prompt.contains("```mermaid"));
        assert!(
            workflows
                .user_prompt
                .contains("## Commands and execution entry points")
        );
        assert!(boundaries.user_prompt.contains("## External packages"));
        assert!(
            configuration
                .user_prompt
                .contains("## Configuration and deployment inputs")
        );
        assert!(!configuration.excerpts.is_empty());

        Ok(())
    }

    fn file_evidence(artifact: &Artifact) -> crate::domain::EvidenceRef {
        crate::domain::EvidenceRef::file(
            crate::domain::ArtifactId::from_path(&artifact.path),
            artifact.path.clone(),
        )
    }

    /// LIT-22.7.1: the architecture page's context (the prompt a real
    /// model actually receives) is explicitly C4-oriented (AC1), its
    /// seed diagram is graph-derived and passes the existing Mermaid
    /// validator (AC2), and existing ADRs/drift findings surface as
    /// explicit content grounded in real evidence (AC3).
    #[test]
    fn architecture_context_is_c4_oriented_with_validated_mermaid_and_adr_drift_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::adr::{AdrStatus, AdrStore};
        use crate::drift::{DriftDetector, DriftFinding, DriftKind, DriftReport};
        use crate::research::ResearchBuilder;

        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        let adr_temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(adr_temp.path());
        store.create(
            "Use blake3 for content hashing",
            "Need a fast, stable content hash for caching.",
            "Adopt blake3 across the analysis cache and run metadata.",
            None,
        )?;
        let mut adr_summaries = store.list();
        assert_eq!(adr_summaries.len(), 1);
        assert_eq!(adr_summaries[0].status, AdrStatus::Proposed);

        let real_drift = DriftDetector.scan(&artifacts, &graph, &root);
        let synthetic_finding = DriftFinding {
            kind: DriftKind::BrokenLink,
            artifact_path: "docs/lithograph/overview.md".to_owned(),
            detail: "docs/missing.md".to_owned(),
            evidence: file_evidence(&artifacts[0]),
            graph_node: None,
        };
        let mut findings = real_drift.findings;
        findings.push(synthetic_finding);
        let drift = DriftReport { findings };
        assert!(!drift.findings.is_empty());

        let architecture_context = super::ArchitectureViewContext {
            drift,
            adr_summaries: std::mem::take(&mut adr_summaries),
        };

        let context = ContextBuilder.build_summary_context(
            TaskKind::Architecture,
            &modules,
            &graph,
            &artifacts,
            Some(&brief),
            Some(&architecture_context),
        );

        for heading in [
            "## System Context",
            "## Container View",
            "## Component View",
            "## Deployment / Runtime View",
            "## Workflows",
            "## Risks",
            "## Drift",
        ] {
            assert!(
                context.user_prompt.contains(heading),
                "missing section: {heading}"
            );
        }
        assert!(context.user_prompt.contains("documentation drift finding"));
        assert!(
            context
                .user_prompt
                .contains("Use blake3 for content hashing")
        );
        assert!(context.user_prompt.contains("```mermaid"));

        let diagram_dir = tempfile::TempDir::new()?;
        let diagram = context
            .user_prompt
            .split("```mermaid")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .ok_or("expected a mermaid fence in the composed context")?;
        std::fs::write(
            diagram_dir.path().join("architecture.md"),
            format!("# Architecture\n\n```mermaid{diagram}```\n"),
        )?;
        let report = crate::mermaid::validate(diagram_dir.path(), None)?;
        assert!(
            report.is_clean(),
            "seed diagram failed Mermaid validation: {:?}",
            report.issues
        );

        Ok(())
    }

    /// LIT-22.7.3 AC1: the three new repository-wide pages (database,
    /// key modules, ADR/drift) each produce real, evidence-grounded
    /// context content -- the polyglot fixture has no SQL/database
    /// evidence, so the database page explicitly says so rather than
    /// fabricating facts.
    #[test]
    fn database_key_modules_and_adr_drift_contexts_are_evidence_grounded()
    -> Result<(), Box<dyn std::error::Error>> {
        use crate::adr::{AdrStatus, AdrStore};
        use crate::drift::DriftReport;
        use crate::research::ResearchBuilder;

        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);
        assert!(brief.agent_memory.database.is_none());

        let database_context = ContextBuilder.build_summary_context(
            TaskKind::Database,
            &modules,
            &graph,
            &artifacts,
            Some(&brief),
            None,
        );
        assert!(database_context.user_prompt.contains("## Database"));
        assert!(
            database_context
                .user_prompt
                .contains("no database schema, migration, or SQL evidence")
        );

        let key_modules_context = ContextBuilder.build_summary_context(
            TaskKind::KeyModules,
            &modules,
            &graph,
            &artifacts,
            Some(&brief),
            None,
        );
        assert!(key_modules_context.user_prompt.contains("## Key Modules"));

        let adr_temp = tempfile::TempDir::new()?;
        let store = AdrStore::new(adr_temp.path());
        store.create(
            "Use blake3 for content hashing",
            "Need a fast, stable content hash.",
            "Adopt blake3.",
            None,
        )?;
        let adr_summaries = store.list();
        assert_eq!(adr_summaries[0].status, AdrStatus::Proposed);
        let architecture_context = super::ArchitectureViewContext {
            drift: DriftReport::default(),
            adr_summaries,
        };
        let adr_drift_context = ContextBuilder.build_summary_context(
            TaskKind::AdrDrift,
            &modules,
            &graph,
            &artifacts,
            Some(&brief),
            Some(&architecture_context),
        );
        assert!(
            adr_drift_context
                .user_prompt
                .contains("## Architecture Decisions")
        );
        assert!(
            adr_drift_context
                .user_prompt
                .contains("Use blake3 for content hashing")
        );
        assert!(adr_drift_context.user_prompt.contains("## Drift"));
        assert!(adr_drift_context.user_prompt.contains("- none observed"));

        Ok(())
    }
}
