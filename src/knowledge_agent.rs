//! Reusable `KnowledgeAgent` framework (LIT-22.6.2): promotes the ad-hoc
//! deterministic agent shape already used internally by `research.rs` into
//! a general framework both research agents (LIT-22.6.3) and editor agents
//! (LIT-22.6.4) build on. An agent declares which named data sources it
//! needs (required vs. optional), an optional prompt template id for
//! agents that call a model, and post-processing/validation hooks; the
//! framework resolves data sources and runs the agent, failing clearly on
//! a missing *required* source and recording (without failing) a missing
//! *optional* one.

use crate::adr::AdrRecord;
use crate::domain::Artifact;
use crate::drift::DriftReport;
use crate::graph::Graph;
use crate::plan::DocumentationModule;
use crate::research::ResearchBrief;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

/// A named fact an agent can declare a dependency on. Concrete and
/// enum-based (not a type-erased registry) since the set of facts flowing
/// through this pipeline is small and known; a downcasting `Any`-based
/// registry would be more general but less type-safe for no real benefit
/// at this scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DataSourceKey {
    /// Repository artifact inventory.
    Artifacts,
    /// Built and validated semantic graph.
    Graph,
    /// Planned documentation modules.
    Modules,
    /// Already-computed research brief (available to agents that run
    /// after the deterministic research pass, e.g. editor agents).
    ResearchBrief,
    /// Persisted architecture decision records (LIT-22.5.4).
    AdrRecords,
    /// Deterministic documentation/intent drift findings (LIT-22.5.3).
    DriftReport,
}

impl Display for DataSourceKey {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Artifacts => "artifacts",
            Self::Graph => "graph",
            Self::Modules => "modules",
            Self::ResearchBrief => "research_brief",
            Self::AdrRecords => "adr_records",
            Self::DriftReport => "drift_report",
        };
        formatter.write_str(name)
    }
}

/// One resolved data source value.
pub(crate) enum DataSourceValue<'a> {
    /// See [`DataSourceKey::Artifacts`].
    Artifacts(&'a [Artifact]),
    /// See [`DataSourceKey::Graph`].
    Graph(&'a Graph),
    /// See [`DataSourceKey::Modules`].
    Modules(&'a [DocumentationModule]),
    /// See [`DataSourceKey::ResearchBrief`].
    ResearchBrief(&'a ResearchBrief),
    /// See [`DataSourceKey::AdrRecords`].
    AdrRecords(&'a [AdrRecord]),
    /// See [`DataSourceKey::DriftReport`].
    DriftReport(&'a DriftReport),
}

/// Registry of data sources available for one agent run. Built once per
/// pipeline run and shared read-only across every agent that runs against
/// it (agents never mutate the context; each returns its own typed output).
#[derive(Default)]
pub(crate) struct AgentContext<'a> {
    values: BTreeMap<DataSourceKey, DataSourceValue<'a>>,
}

impl<'a> AgentContext<'a> {
    /// Builds an empty context; call [`Self::with`] to populate it.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Inserts one data source, replacing any existing value for the same
    /// key.
    #[must_use]
    pub(crate) fn with(mut self, key: DataSourceKey, value: DataSourceValue<'a>) -> Self {
        self.values.insert(key, value);
        self
    }

    /// Looks up one data source.
    pub(crate) fn get(&self, key: DataSourceKey) -> Option<&DataSourceValue<'a>> {
        self.values.get(&key)
    }

    /// Looks up the artifact inventory, when present.
    pub(crate) fn artifacts(&self) -> Option<&'a [Artifact]> {
        match self.get(DataSourceKey::Artifacts) {
            Some(DataSourceValue::Artifacts(artifacts)) => Some(artifacts),
            _ => None,
        }
    }

    /// Looks up the graph, when present.
    pub(crate) fn graph(&self) -> Option<&'a Graph> {
        match self.get(DataSourceKey::Graph) {
            Some(DataSourceValue::Graph(graph)) => Some(graph),
            _ => None,
        }
    }

    /// Looks up the planned modules, when present.
    pub(crate) fn modules(&self) -> Option<&'a [DocumentationModule]> {
        match self.get(DataSourceKey::Modules) {
            Some(DataSourceValue::Modules(modules)) => Some(modules),
            _ => None,
        }
    }

    /// Looks up the research brief, when present.
    pub(crate) fn research_brief(&self) -> Option<&'a ResearchBrief> {
        match self.get(DataSourceKey::ResearchBrief) {
            Some(DataSourceValue::ResearchBrief(brief)) => Some(brief),
            _ => None,
        }
    }

    /// Looks up the ADR records, when present.
    pub(crate) fn adr_records(&self) -> Option<&'a [AdrRecord]> {
        match self.get(DataSourceKey::AdrRecords) {
            Some(DataSourceValue::AdrRecords(records)) => Some(records),
            _ => None,
        }
    }

    /// Looks up the drift report, when present.
    pub(crate) fn drift_report(&self) -> Option<&'a DriftReport> {
        match self.get(DataSourceKey::DriftReport) {
            Some(DataSourceValue::DriftReport(report)) => Some(report),
            _ => None,
        }
    }
}

/// Which data sources one agent needs (AC1).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DataSourceSpec {
    /// Sources that must be present, or [`AgentError::MissingRequiredDataSource`]
    /// is returned before the agent ever runs.
    pub required: &'static [DataSourceKey],
    /// Sources the agent can use if present, but runs without.
    pub optional: &'static [DataSourceKey],
}

/// Which declared-optional data sources were actually missing this run,
/// so an agent can adjust its output (or a caller can report reduced
/// confidence) without treating absence as an error (AC2).
#[derive(Debug, Clone, Default)]
pub(crate) struct DataSourceResolution {
    /// Optional keys declared by the agent that had no value in the context.
    pub missing_optional: Vec<DataSourceKey>,
}

impl DataSourceResolution {
    /// True when `key` was declared optional and had no value.
    pub(crate) fn is_missing(&self, key: DataSourceKey) -> bool {
        self.missing_optional.contains(&key)
    }
}

/// A framework-level agent failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AgentError {
    /// A data source the agent declared `required` had no value in the context.
    MissingRequiredDataSource(DataSourceKey),
    /// The agent's own [`KnowledgeAgent::validate`] rejected its output.
    Validation(String),
}

impl Display for AgentError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRequiredDataSource(key) => {
                write!(formatter, "missing required data source `{key}`")
            }
            Self::Validation(message) => {
                write!(formatter, "agent output validation failed: {message}")
            }
        }
    }
}

impl std::error::Error for AgentError {}

/// One deterministic-under-mock-input knowledge agent (AC1/AC3).
///
/// `run()` is the framework entry point: it resolves declared data
/// sources, computes the output, post-processes it, then validates it,
/// short-circuiting with a clear [`AgentError`] on a missing required
/// source or a failed validation. Implementors only need [`Self::compute`]
/// (and [`Self::data_sources`]); [`Self::post_process`] and
/// [`Self::validate`] default to no-ops.
pub(crate) trait KnowledgeAgent {
    /// Output report type. The type itself is this agent's output schema
    /// (AC1) -- Rust's type system already gives a precise, checked schema,
    /// so the framework doesn't duplicate it as a separate description.
    type Output: Serialize + Clone + PartialEq;

    /// Stable agent name and memory/report key.
    fn memory_key(&self) -> &'static str;

    /// Declares which data sources this agent needs. Defaults to none.
    fn data_sources(&self) -> DataSourceSpec {
        DataSourceSpec::default()
    }

    /// Stable prompt template id for agents that call a model (LIT-22.6.3/
    /// LIT-22.6.4). Deterministic, template-free agents leave this `None`.
    fn prompt_template(&self) -> Option<&'static str> {
        None
    }

    /// Computes this agent's output from the resolved context. Called only
    /// after every `required` data source is confirmed present.
    fn compute(
        &self,
        context: &AgentContext<'_>,
        resolution: &DataSourceResolution,
    ) -> Self::Output;

    /// Adjusts computed output before validation (e.g. sorting, capping
    /// list length). Defaults to identity.
    fn post_process(&self, output: Self::Output) -> Self::Output {
        output
    }

    /// Rejects an output that violates an agent-specific invariant.
    /// Defaults to always accepting.
    fn validate(&self, _output: &Self::Output) -> Result<(), String> {
        Ok(())
    }

    /// Runs this agent against `context`: resolves data sources, computes,
    /// post-processes, and validates (AC2).
    fn run(&self, context: &AgentContext<'_>) -> Result<Self::Output, AgentError> {
        let spec = self.data_sources();
        for key in spec.required {
            if context.get(*key).is_none() {
                return Err(AgentError::MissingRequiredDataSource(*key));
            }
        }
        let missing_optional: Vec<DataSourceKey> = spec
            .optional
            .iter()
            .copied()
            .filter(|key| context.get(*key).is_none())
            .collect();
        let resolution = DataSourceResolution { missing_optional };

        let output = self.post_process(self.compute(context, &resolution));
        self.validate(&output).map_err(AgentError::Validation)?;
        Ok(output)
    }
}

/// Distinct data-source keys declared across `required`/`optional` in a
/// [`DataSourceSpec`], useful for reporting which facts an agent touches
/// without needing to run it.
pub(crate) fn declared_keys(spec: &DataSourceSpec) -> BTreeSet<DataSourceKey> {
    spec.required
        .iter()
        .chain(spec.optional.iter())
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        AgentContext, AgentError, DataSourceKey, DataSourceResolution, DataSourceSpec,
        DataSourceValue, KnowledgeAgent, declared_keys,
    };
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, RepoPath, SupportTier, TextStatus,
    };
    use crate::generation::{LanguageModel, MockModel, ModelRequest};
    use serde::Serialize;

    fn artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::DeepLanguage,
            ContentHash::new("aaaaaaaa")?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    struct CountReport {
        artifact_count: usize,
        used_optional_modules: bool,
    }

    /// Requires `Artifacts`, optionally uses `Modules`.
    struct CountAgent;

    impl KnowledgeAgent for CountAgent {
        type Output = CountReport;

        fn memory_key(&self) -> &'static str {
            "count-report"
        }

        fn data_sources(&self) -> DataSourceSpec {
            DataSourceSpec {
                required: &[DataSourceKey::Artifacts],
                optional: &[DataSourceKey::Modules],
            }
        }

        fn compute(
            &self,
            context: &AgentContext<'_>,
            resolution: &DataSourceResolution,
        ) -> Self::Output {
            let artifact_count = context.artifacts().map(<[_]>::len).unwrap_or(0);
            CountReport {
                artifact_count,
                used_optional_modules: !resolution.is_missing(DataSourceKey::Modules),
            }
        }
    }

    #[test]
    fn missing_required_data_source_fails_clearly() {
        let context = AgentContext::new();

        let error = CountAgent.run(&context);

        assert_eq!(
            error,
            Err(AgentError::MissingRequiredDataSource(
                DataSourceKey::Artifacts
            ))
        );
    }

    #[test]
    fn missing_optional_data_source_is_recorded_but_non_fatal()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("a.py")?, artifact("b.py")?];
        let context = AgentContext::new().with(
            DataSourceKey::Artifacts,
            DataSourceValue::Artifacts(&artifacts),
        );

        let report = CountAgent.run(&context)?;

        assert_eq!(
            report,
            CountReport {
                artifact_count: 2,
                used_optional_modules: false,
            }
        );

        Ok(())
    }

    #[test]
    fn present_optional_data_source_is_used() -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("a.py")?];
        let modules: Vec<crate::plan::DocumentationModule> = Vec::new();
        let context = AgentContext::new()
            .with(
                DataSourceKey::Artifacts,
                DataSourceValue::Artifacts(&artifacts),
            )
            .with(DataSourceKey::Modules, DataSourceValue::Modules(&modules));

        let report = CountAgent.run(&context)?;

        assert!(report.used_optional_modules);

        Ok(())
    }

    #[derive(Debug, Clone, PartialEq, Serialize)]
    struct AlwaysInvalidReport;

    struct InvalidatingAgent;

    impl KnowledgeAgent for InvalidatingAgent {
        type Output = AlwaysInvalidReport;

        fn memory_key(&self) -> &'static str {
            "invalidating-report"
        }

        fn compute(
            &self,
            _context: &AgentContext<'_>,
            _resolution: &DataSourceResolution,
        ) -> Self::Output {
            AlwaysInvalidReport
        }

        fn validate(&self, _output: &Self::Output) -> Result<(), String> {
            Err("always invalid".to_owned())
        }
    }

    #[test]
    fn validation_failure_surfaces_as_agent_error() {
        let context = AgentContext::new();

        let error = InvalidatingAgent.run(&context);

        assert_eq!(
            error,
            Err(AgentError::Validation("always invalid".to_owned()))
        );
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize)]
    struct ModelBackedReport {
        text: String,
    }

    /// Proves determinism under a mock model (AC3): the agent holds its
    /// own model reference (not part of the trait signature, since not
    /// every agent needs one) and calls it inside `compute`.
    struct ModelBackedAgent<'m> {
        model: &'m dyn LanguageModel,
    }

    impl KnowledgeAgent for ModelBackedAgent<'_> {
        type Output = ModelBackedReport;

        fn memory_key(&self) -> &'static str {
            "model-backed-report"
        }

        fn compute(
            &self,
            _context: &AgentContext<'_>,
            _resolution: &DataSourceResolution,
        ) -> Self::Output {
            let request = ModelRequest {
                model: "mock".to_owned(),
                prompt_version: "v1".to_owned(),
                task_kind: crate::manifest::TaskKind::Overview,
                input_hash: "hash".to_owned(),
                system_prompt: String::new(),
                user_prompt: "describe the repository".to_owned(),
            };
            let text = self
                .model
                .generate_text(&request)
                .unwrap_or_else(|_| String::new());
            ModelBackedReport { text }
        }
    }

    #[test]
    fn agent_execution_is_deterministic_under_mock_model_outputs() {
        let agent = ModelBackedAgent { model: &MockModel };
        let context = AgentContext::new();

        let first = agent.run(&context);
        let second = agent.run(&context);

        assert_eq!(first, second);
        assert!(first.is_ok());
    }

    #[test]
    fn declared_keys_merges_required_and_optional() {
        let spec = DataSourceSpec {
            required: &[DataSourceKey::Artifacts],
            optional: &[DataSourceKey::Modules, DataSourceKey::Artifacts],
        };

        let keys = declared_keys(&spec);

        assert_eq!(
            keys,
            [DataSourceKey::Artifacts, DataSourceKey::Modules]
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn agent_error_implements_std_error() {
        fn assert_error<E: std::error::Error>() {}
        assert_error::<AgentError>();
    }
}
