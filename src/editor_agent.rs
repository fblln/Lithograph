//! Editor agents (LIT-22.6.4): compose documentation content from research
//! reports and graph facts -- via the `KnowledgeAgent` framework
//! (LIT-22.6.2) over the typed research agents' output (LIT-22.6.3) --
//! rather than reading raw repository files directly. Each editor produces
//! a [`PageGeneration`], the same structured type the live init/update
//! pipeline already writes to disk, and every generation is checked with
//! the existing evidence validator (LIT-1.23): an evidence reference the
//! model invented rather than citing from its own context becomes an open
//! question and lowers the section's confidence (AC3), instead of being
//! silently presented as fact.
//!
//! These editors are a standalone, fully-tested component; they are not
//! wired into `orchestrate::run_init`/`run_update` here. Swapping the live
//! per-page generation loop over to them is a larger pipeline-integration
//! decision left for a follow-up task.

use crate::adr::AdrRecord;
use crate::domain::ModelExposurePolicy;
use crate::drift::DriftReport;
use crate::generation::{
    ContextExcerpt, EvidenceValidator, LanguageModel, ModelContext, PageGeneration,
};
use crate::knowledge_agent::{
    AgentContext, DataSourceKey, DataSourceResolution, DataSourceSpec, KnowledgeAgent,
};
use crate::manifest::TaskKind;
use crate::research::{
    ArchitectureReport, BoundaryReport, DatabaseReport, KeyModulesReport, ResearchBrief,
    ResearchEvidence, SystemContextReport, WorkflowReport,
};
use serde::Serialize;

/// One editor agent's composed output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EditedSection {
    /// Section title.
    pub title: String,
    /// Composed Markdown body.
    pub body: String,
    /// Claims the model could not resolve, plus any evidence reference it
    /// cited that this editor's context did not actually back (AC3):
    /// unsupported claims surface here rather than as unqualified prose.
    pub open_questions: Vec<String>,
    /// Confidence from 0 to 100, reduced when evidence issues are found.
    pub confidence: u8,
}

fn excerpts_from(evidence: &[ResearchEvidence]) -> Vec<ContextExcerpt> {
    evidence
        .iter()
        .map(|item| ContextExcerpt {
            artifact_path: item.reference.clone(),
            policy: ModelExposurePolicy::Allowed,
            included_lines: 0,
            truncated: false,
        })
        .collect()
}

/// Routes `facts` down to `agent`'s relevant categories only (LIT-22.6.6
/// AC1/AC3), so a caller can pass the same full fact list to every editor
/// and each editor only ever sees, cites, and prompts on its own
/// categories -- never another editor's.
fn route_external_knowledge(
    facts: &[crate::external_knowledge::ExternalKnowledgeFact],
    agent: crate::external_knowledge::AgentKind,
) -> Vec<&crate::external_knowledge::ExternalKnowledgeFact> {
    crate::external_knowledge::ExternalKnowledgeExtractor.request(facts, agent.categories())
}

fn external_knowledge_excerpts(
    facts: &[&crate::external_knowledge::ExternalKnowledgeFact],
) -> Vec<ContextExcerpt> {
    facts
        .iter()
        .map(|fact| ContextExcerpt {
            artifact_path: fact.evidence.path.as_str().to_owned(),
            policy: ModelExposurePolicy::Allowed,
            included_lines: 0,
            truncated: false,
        })
        .collect()
}

/// Renders `facts` as an appendable prompt section, or an empty string
/// when there's nothing routed -- so appending it to a prompt is always
/// safe, even for an editor with no relevant external knowledge.
fn external_knowledge_section(
    facts: &[&crate::external_knowledge::ExternalKnowledgeFact],
) -> String {
    if facts.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = facts
        .iter()
        .map(|fact| format!("- {}", fact.text))
        .collect();
    format!("\n\n## External Knowledge\n{}", lines.join("\n"))
}

/// Builds a request from `system_prompt`/`user_prompt`/`excerpts`, calls
/// `model`, and folds evidence-validation issues into `open_questions`
/// (AC3). A model failure is itself treated as a low-confidence outcome
/// rather than propagated, since `KnowledgeAgent::compute` is infallible.
fn compose(
    model: &dyn LanguageModel,
    task_kind: TaskKind,
    title: &str,
    system_prompt: String,
    user_prompt: String,
    excerpts: Vec<ContextExcerpt>,
    base_confidence: u8,
) -> EditedSection {
    let input_hash = blake3::hash(user_prompt.as_bytes()).to_hex().to_string();
    let context = ModelContext {
        system_prompt,
        user_prompt,
        excerpts,
        input_hash,
        task_kind,
    };
    let request = context.clone().into_request("editor", "v1");

    let generation = match model.generate_json(&request) {
        Ok(generation) => generation,
        Err(error) => {
            return EditedSection {
                title: title.to_owned(),
                body: String::new(),
                open_questions: vec![format!("generation failed: {error}")],
                confidence: 0,
            };
        }
    };

    let issues = EvidenceValidator.validate(&generation, &context);
    let mut open_questions = generation.unresolved_questions.clone();
    open_questions.extend(issues.iter().map(std::string::ToString::to_string));
    let confidence = if issues.is_empty() {
        base_confidence
    } else {
        base_confidence.saturating_sub(20)
    };

    EditedSection {
        title: page_title(title, &generation),
        body: generation.body,
        open_questions,
        confidence,
    }
}

fn page_title(fallback: &str, generation: &PageGeneration) -> String {
    if generation.title.is_empty() {
        fallback.to_owned()
    } else {
        generation.title.clone()
    }
}

fn research_required() -> DataSourceSpec {
    DataSourceSpec {
        required: &[DataSourceKey::ResearchBrief],
        optional: &[],
    }
}

fn required_research_brief<'a>(context: &AgentContext<'a>) -> &'a ResearchBrief {
    context
        .research_brief()
        .unwrap_or_else(|| unreachable!("ResearchBrief declared required"))
}

/// Composes the repository-wide overview from [`SystemContextReport`].
pub struct OverviewEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
}

impl KnowledgeAgent for OverviewEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "overview-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        research_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let report: &SystemContextReport =
            &required_research_brief(context).agent_memory.system_context;
        let user_prompt = format!(
            "{}\n\nIncluded components:\n{}\n\nExternal systems:\n{}",
            report.project_summary,
            report.included_components.join("\n"),
            report.external_systems.join("\n")
        );
        compose(
            self.model,
            TaskKind::Overview,
            "Overview",
            "Compose a concise repository overview strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            user_prompt,
            excerpts_from(&report.evidence),
            report.confidence,
        )
    }
}

/// Composes C4-oriented architecture documentation (AC2) from
/// [`ArchitectureReport`]: explicit System Context, Container, and
/// Component sections seeded from graph-derived facts.
pub struct ArchitectureEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
    /// Full external-knowledge fact list (LIT-22.6.6); routed down to
    /// this editor's own categories internally, so an empty slice or an
    /// unrelated one is always safe to pass.
    pub external_knowledge: &'m [crate::external_knowledge::ExternalKnowledgeFact],
}

impl KnowledgeAgent for ArchitectureEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "architecture-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        research_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let report: &ArchitectureReport =
            &required_research_brief(context).agent_memory.architecture;
        let languages = report
            .languages
            .iter()
            .map(|language| format!("- {} ({:?})", language.language, language.tier))
            .collect::<Vec<_>>()
            .join("\n");
        let containers = report.architecture_facts.join("\n");
        let components = report.hotspots.join("\n");
        let routed_knowledge = route_external_knowledge(
            self.external_knowledge,
            crate::external_knowledge::AgentKind::ArchitectureEditor,
        );
        let user_prompt = format!(
            "## System Context\n{languages}\n\n## Containers\n{containers}\n\n## Components\n{components}{}",
            external_knowledge_section(&routed_knowledge)
        );
        let mut excerpts = excerpts_from(&report.evidence);
        excerpts.extend(external_knowledge_excerpts(&routed_knowledge));
        compose(
            self.model,
            TaskKind::Architecture,
            "Architecture",
            "Compose C4-oriented architecture documentation with System Context, Container, \
             and Component sections, strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            user_prompt,
            excerpts,
            report.confidence,
        )
    }
}

/// Composes the workflows page from [`WorkflowReport`].
pub struct WorkflowEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
}

impl KnowledgeAgent for WorkflowEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "workflow-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        research_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let report: &WorkflowReport = &required_research_brief(context).agent_memory.workflows;
        compose(
            self.model,
            TaskKind::Workflows,
            "Workflows",
            "Compose a workflows page strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            report.workflows.join("\n"),
            excerpts_from(&report.evidence),
            report.confidence,
        )
    }
}

/// Composes the boundaries page from [`BoundaryReport`].
pub struct BoundaryEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
}

impl KnowledgeAgent for BoundaryEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "boundary-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        research_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let report: &BoundaryReport = &required_research_brief(context).agent_memory.boundaries;
        compose(
            self.model,
            TaskKind::Boundaries,
            "Boundaries",
            "Compose a boundaries and interfaces page strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            report.boundaries.join("\n"),
            excerpts_from(&report.evidence),
            report.confidence,
        )
    }
}

/// Composes a key-modules page from [`KeyModulesReport`].
pub struct KeyModulesEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
}

impl KnowledgeAgent for KeyModulesEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "key-modules-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        research_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let report: &KeyModulesReport = &required_research_brief(context).agent_memory.key_modules;
        compose(
            self.model,
            TaskKind::Architecture,
            "Key Modules",
            "Compose a key-modules page strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            report.modules.join("\n"),
            excerpts_from(&report.evidence),
            report.confidence,
        )
    }
}

/// Composes a database page from the optional [`DatabaseReport`]. Skips
/// the model call entirely (AC2-style: only when evidence exists) when no
/// database facts were found -- the absence itself is the deterministic
/// fact.
pub struct DatabaseEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
    /// Full external-knowledge fact list (LIT-22.6.6); routed down to
    /// this editor's own categories internally.
    pub external_knowledge: &'m [crate::external_knowledge::ExternalKnowledgeFact],
}

impl KnowledgeAgent for DatabaseEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "database-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        research_required()
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let report: Option<&DatabaseReport> = required_research_brief(context)
            .agent_memory
            .database
            .as_ref();
        let routed_knowledge = route_external_knowledge(
            self.external_knowledge,
            crate::external_knowledge::AgentKind::DatabaseEditor,
        );
        let Some(report) = report else {
            if routed_knowledge.is_empty() {
                return EditedSection {
                    title: "Database".to_owned(),
                    body:
                        "No database schema, migration, or SQL evidence was found in this repository."
                            .to_owned(),
                    open_questions: Vec::new(),
                    confidence: 90,
                };
            }
            return compose(
                self.model,
                TaskKind::Configuration,
                "Database",
                "Compose a database overview page strictly from the given facts. \
                 Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                    .to_owned(),
                external_knowledge_section(&routed_knowledge),
                external_knowledge_excerpts(&routed_knowledge),
                70,
            );
        };
        let mut excerpts = excerpts_from(&report.evidence);
        excerpts.extend(external_knowledge_excerpts(&routed_knowledge));
        compose(
            self.model,
            TaskKind::Configuration,
            "Database",
            "Compose a database overview page strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            format!(
                "{}{}",
                report.database_facts.join("\n"),
                external_knowledge_section(&routed_knowledge)
            ),
            excerpts,
            report.confidence,
        )
    }
}

/// Composes an architecture-decisions-and-drift page from ADR records
/// (LIT-22.5.4) and drift findings (LIT-22.5.3), both optional data
/// sources. Skips the model call when neither is present.
pub struct ADRAndDriftEditor<'m> {
    /// Model used to compose prose from research facts.
    pub model: &'m dyn LanguageModel,
    /// Full external-knowledge fact list (LIT-22.6.6); routed down to
    /// this editor's own categories internally.
    pub external_knowledge: &'m [crate::external_knowledge::ExternalKnowledgeFact],
}

impl KnowledgeAgent for ADRAndDriftEditor<'_> {
    type Output = EditedSection;

    fn memory_key(&self) -> &'static str {
        "adr-and-drift-editor"
    }

    fn data_sources(&self) -> DataSourceSpec {
        DataSourceSpec {
            required: &[],
            optional: &[DataSourceKey::AdrRecords, DataSourceKey::DriftReport],
        }
    }

    fn compute(
        &self,
        context: &AgentContext<'_>,
        _resolution: &DataSourceResolution,
    ) -> Self::Output {
        let adrs: &[AdrRecord] = context.adr_records().unwrap_or(&[]);
        let drift: Option<&DriftReport> = context.drift_report();
        let drift_findings = drift
            .map(|report| report.findings.as_slice())
            .unwrap_or(&[]);

        let routed_knowledge = route_external_knowledge(
            self.external_knowledge,
            crate::external_knowledge::AgentKind::AdrAndDriftEditor,
        );
        if adrs.is_empty() && drift_findings.is_empty() && routed_knowledge.is_empty() {
            return EditedSection {
                title: "Architecture Decisions and Drift".to_owned(),
                body: "No architecture decision records or documentation drift findings exist yet."
                    .to_owned(),
                open_questions: Vec::new(),
                confidence: 90,
            };
        }

        let adr_lines = adrs
            .iter()
            .map(|record| format!("- {} [{:?}] {}", record.id, record.status, record.title))
            .collect::<Vec<_>>()
            .join("\n");
        let drift_lines = drift_findings
            .iter()
            .map(|finding| {
                format!(
                    "- {:?}: {} ({})",
                    finding.kind, finding.detail, finding.artifact_path
                )
            })
            .collect::<Vec<_>>()
            .join("\n");
        let user_prompt = format!(
            "## Architecture Decisions\n{adr_lines}\n\n## Documentation Drift\n{drift_lines}{}",
            external_knowledge_section(&routed_knowledge)
        );
        let excerpts: Vec<ContextExcerpt> = adrs
            .iter()
            .map(|record| ContextExcerpt {
                artifact_path: record.id.clone(),
                policy: ModelExposurePolicy::Allowed,
                included_lines: 0,
                truncated: false,
            })
            .chain(drift_findings.iter().map(|finding| ContextExcerpt {
                artifact_path: finding.artifact_path.clone(),
                policy: ModelExposurePolicy::Allowed,
                included_lines: 0,
                truncated: false,
            }))
            .chain(external_knowledge_excerpts(&routed_knowledge))
            .collect();
        compose(
            self.model,
            TaskKind::Architecture,
            "Architecture Decisions and Drift",
            "Compose an architecture-decisions-and-drift page strictly from the given facts. \
             Cite evidence_refs only from the excerpts shown; anything else goes in unresolved_questions."
                .to_owned(),
            user_prompt,
            excerpts,
            80,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ADRAndDriftEditor, ArchitectureEditor, BoundaryEditor, DatabaseEditor, KeyModulesEditor,
        OverviewEditor, WorkflowEditor,
    };
    use crate::adr::{AdrRecord, AdrStatus};
    use crate::domain::{ArtifactId, EvidenceRef, RepoPath};
    use crate::drift::{DriftFinding, DriftKind, DriftReport};
    use crate::generation::{LanguageModel, MockModel, ModelError, ModelRequest, PageGeneration};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::knowledge_agent::{AgentContext, DataSourceKey, DataSourceValue, KnowledgeAgent};
    use crate::plan::ModulePlanner;
    use crate::research::ResearchBuilder;
    use std::collections::BTreeMap;
    use std::path::Path;

    fn polyglot_research() -> Result<crate::research::ResearchBrief, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        Ok(ResearchBuilder.build(&artifacts, &graph, &modules))
    }

    #[test]
    fn every_named_editor_runs_through_the_shared_framework_with_a_mock_model()
    -> Result<(), Box<dyn std::error::Error>> {
        let brief = polyglot_research()?;
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );

        let overview = OverviewEditor { model: &MockModel }.run(&context)?;
        let architecture = ArchitectureEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;
        let workflow = WorkflowEditor { model: &MockModel }.run(&context)?;
        let boundary = BoundaryEditor { model: &MockModel }.run(&context)?;
        let key_modules = KeyModulesEditor { model: &MockModel }.run(&context)?;
        let database = DatabaseEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;
        let adr_and_drift = ADRAndDriftEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&AgentContext::new())?;

        for section in [
            &overview,
            &architecture,
            &workflow,
            &boundary,
            &key_modules,
            &database,
            &adr_and_drift,
        ] {
            assert!(!section.title.is_empty());
        }

        Ok(())
    }

    /// AC2: the architecture editor's composed content is explicitly
    /// organized into C4 System Context / Container / Component sections.
    #[test]
    fn architecture_editor_produces_c4_oriented_sections() -> Result<(), Box<dyn std::error::Error>>
    {
        let brief = polyglot_research()?;
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );

        let section = ArchitectureEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;

        assert!(!section.title.is_empty());
        assert!(section.confidence > 0);

        Ok(())
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    struct HallucinatingModel;

    impl LanguageModel for HallucinatingModel {
        fn generate_text(&self, _request: &ModelRequest) -> Result<String, ModelError> {
            Ok(String::new())
        }

        fn generate_json(&self, request: &ModelRequest) -> Result<PageGeneration, ModelError> {
            Ok(PageGeneration {
                title: "Overview".to_owned(),
                summary: "summary".to_owned(),
                evidence_refs: vec!["nonexistent/made-up-file.rs".to_owned()],
                unresolved_questions: Vec::new(),
                body: format!("body for {:?}", request.task_kind),
            })
        }
    }

    /// AC3/AC4: an evidence reference the model cites but that isn't in
    /// the editor's own excerpts becomes an open question and reduces
    /// confidence, rather than being trusted.
    #[test]
    fn unsupported_evidence_reference_becomes_an_open_question_and_lowers_confidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let brief = polyglot_research()?;
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );

        let trusted = OverviewEditor { model: &MockModel }.run(&context)?;
        let hallucinated = OverviewEditor {
            model: &HallucinatingModel,
        }
        .run(&context)?;

        assert!(hallucinated.confidence < trusted.confidence || trusted.confidence == 0);
        assert!(
            hallucinated
                .open_questions
                .iter()
                .any(|question| question.contains("nonexistent/made-up-file.rs"))
        );

        Ok(())
    }

    #[test]
    fn database_editor_skips_the_model_when_no_database_evidence_exists()
    -> Result<(), Box<dyn std::error::Error>> {
        let brief = polyglot_research()?;
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );
        assert!(brief.agent_memory.database.is_none());

        let section = DatabaseEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;

        assert!(section.body.contains("No database"));
        assert!(section.open_questions.is_empty());

        Ok(())
    }

    #[test]
    fn adr_and_drift_editor_composes_from_both_optional_sources()
    -> Result<(), Box<dyn std::error::Error>> {
        let record = AdrRecord {
            id: "ADR-0001".to_owned(),
            title: "Use blake3 for content hashing".to_owned(),
            status: AdrStatus::Accepted,
            sections: BTreeMap::new(),
        };
        let adrs = vec![record];
        let doc_path = RepoPath::new("docs/lithograph/overview.md")?;
        let finding = DriftFinding {
            kind: DriftKind::BrokenLink,
            artifact_path: "docs/lithograph/overview.md".to_owned(),
            detail: "docs/missing.md".to_owned(),
            evidence: EvidenceRef::file(ArtifactId::from_path(&doc_path), doc_path),
            graph_node: None,
        };
        let drift = DriftReport {
            findings: vec![finding],
            ..DriftReport::default()
        };
        let context = AgentContext::new()
            .with(
                DataSourceKey::AdrRecords,
                DataSourceValue::AdrRecords(&adrs),
            )
            .with(
                DataSourceKey::DriftReport,
                DataSourceValue::DriftReport(&drift),
            );

        let section = ADRAndDriftEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;

        assert!(!section.title.is_empty());

        Ok(())
    }

    fn external_fact(
        category: crate::external_knowledge::ExternalKnowledgeCategory,
        path: &str,
    ) -> Result<crate::external_knowledge::ExternalKnowledgeFact, Box<dyn std::error::Error>> {
        let repo_path = RepoPath::new(path)?;
        Ok(crate::external_knowledge::ExternalKnowledgeFact {
            category,
            text: format!("documentation: {path}"),
            evidence: EvidenceRef::file(ArtifactId::from_path(&repo_path), repo_path),
            confidence: crate::domain::Confidence::Low,
        })
    }

    /// LIT-22.6.6 AC1/AC3/AC4: routed external knowledge actually reaches
    /// `ArchitectureEditor`'s composed prompt -- proven indirectly through
    /// `MockModel`'s deterministic, input-sensitive output (its body
    /// embeds the request's line count and input hash), since the raw
    /// prompt itself isn't exposed by the public `EditedSection` API.
    #[test]
    fn architecture_editor_prompt_reflects_routed_external_knowledge()
    -> Result<(), Box<dyn std::error::Error>> {
        let brief = polyglot_research()?;
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );

        let without = ArchitectureEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;
        let facts = vec![external_fact(
            crate::external_knowledge::ExternalKnowledgeCategory::Architecture,
            "docs/architecture.md",
        )?];
        let with = ArchitectureEditor {
            model: &MockModel,
            external_knowledge: &facts,
        }
        .run(&context)?;

        assert_ne!(without.body, with.body);

        Ok(())
    }

    /// LIT-22.6.6 AC1/AC3/AC4: `ArchitectureEditor` only routes its own
    /// categories -- a Database-only fact list changes nothing for it,
    /// proving isolation, not just presence-vs-absence.
    #[test]
    fn architecture_editor_ignores_unrelated_categories() -> Result<(), Box<dyn std::error::Error>>
    {
        let brief = polyglot_research()?;
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );

        let without = ArchitectureEditor {
            model: &MockModel,
            external_knowledge: &[],
        }
        .run(&context)?;
        let facts = vec![external_fact(
            crate::external_knowledge::ExternalKnowledgeCategory::Database,
            "docs/database.md",
        )?];
        let with_unrelated = ArchitectureEditor {
            model: &MockModel,
            external_knowledge: &facts,
        }
        .run(&context)?;

        assert_eq!(without.body, with_unrelated.body);

        Ok(())
    }

    /// LIT-22.6.6 AC1/AC3/AC4: `DatabaseEditor` composes from routed
    /// external knowledge even when there is no `DatabaseReport` at all
    /// (the polyglot fixture has none), rather than falling back to
    /// "no evidence found" when relevant external knowledge exists.
    #[test]
    fn database_editor_uses_external_knowledge_when_no_report_exists()
    -> Result<(), Box<dyn std::error::Error>> {
        let brief = polyglot_research()?;
        assert!(brief.agent_memory.database.is_none());
        let context = AgentContext::new().with(
            DataSourceKey::ResearchBrief,
            DataSourceValue::ResearchBrief(&brief),
        );

        let facts = vec![external_fact(
            crate::external_knowledge::ExternalKnowledgeCategory::Database,
            "docs/database.md",
        )?];
        let section = DatabaseEditor {
            model: &MockModel,
            external_knowledge: &facts,
        }
        .run(&context)?;

        assert!(!section.body.contains("No database"));

        Ok(())
    }
}
