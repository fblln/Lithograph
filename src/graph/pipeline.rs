//! Typed, observable contracts for deterministic graph construction passes.

use crate::graph::Graph;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Instant;

/// Bump when graph-pass ordering or semantics change.
pub(crate) const GRAPH_BUILD_PIPELINE_VERSION: u32 = 2;

/// Version of the machine-readable diagnostic trace contract.
pub(crate) const GRAPH_BUILD_TRACE_VERSION: u32 = 1;

/// A deterministic phase in graph construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphBuildPass {
    /// Create artifact and module structure.
    #[serde(rename = "structure", alias = "Structure")]
    Structure,
    /// Extract and materialize definitions and imports.
    #[serde(rename = "definitions_and_imports", alias = "DefinitionsAndImports")]
    DefinitionsAndImports,
    /// Apply semantic enrichment such as clone detection.
    #[serde(rename = "enrichment", alias = "Enrichment")]
    Enrichment,
    /// Resolve calls, types, imports, and usages against project indexes.
    #[serde(rename = "resolution", alias = "Resolution")]
    Resolution,
    /// Compute graph-derived analytics without changing semantic topology.
    #[serde(rename = "analytics", alias = "Analytics")]
    Analytics,
    /// Prepare the normalized graph for durable storage.
    #[serde(rename = "persistence", alias = "Persistence")]
    Persistence,
    /// Normalize graph invariants and prepare the persisted graph payload.
    #[serde(rename = "finalize", alias = "Finalize")]
    Finalize,
}

impl GraphBuildPass {
    /// Stable serialized/file name independent of Rust debug formatting.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Structure => "structure",
            Self::DefinitionsAndImports => "definitions_and_imports",
            Self::Enrichment => "enrichment",
            Self::Resolution => "resolution",
            Self::Analytics => "analytics",
            Self::Persistence => "persistence",
            Self::Finalize => "finalize",
        }
    }
}

/// One explainable graph decision captured for focused diagnostics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphDecisionTrace {
    /// Stable decision category.
    pub kind: String,
    /// Source node or artifact.
    pub source: String,
    /// Selected or considered target.
    pub target: String,
    /// Stable strategy name.
    pub strategy: String,
    /// Selected/emitted/rejected outcome.
    pub outcome: String,
    /// Integer score in millionths to avoid floating-point trace instability.
    pub score_millionths: u32,
    /// Evidence paths relevant to the decision.
    pub evidence_paths: Vec<String>,
    /// Candidate/rejection explanation.
    pub reason: String,
}

/// Stable execution order for every graph build. Test-only: asserts the
/// build trace covers every pass; production drives passes directly.
#[cfg(test)]
pub(crate) const GRAPH_BUILD_PASS_ORDER: &[GraphBuildPass] = &[
    GraphBuildPass::Structure,
    GraphBuildPass::DefinitionsAndImports,
    GraphBuildPass::Enrichment,
    GraphBuildPass::Resolution,
    GraphBuildPass::Analytics,
    GraphBuildPass::Persistence,
    GraphBuildPass::Finalize,
];

/// Observable output of one completed graph-build pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GraphBuildPassResult {
    /// Completed pass.
    pub pass: GraphBuildPass,
    /// Node count after the pass.
    pub node_count: usize,
    /// Relation count after the pass.
    pub relation_count: usize,
}

/// Controls how much graph state an opt-in diagnostic build retains.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) enum GraphBuildTraceDetail {
    /// Retain deterministic counts and hashes only.
    #[default]
    Summary,
    /// Retain the complete canonical graph after every pass.
    Full,
}

/// Configuration for an opt-in graph-build trace.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct GraphBuildTraceConfig {
    /// Amount of graph state retained for each pass.
    pub detail: GraphBuildTraceDetail,
    /// Optional stable node/path fragments used by diagnostic consumers.
    /// The builder records these selectors verbatim so a replay can prove
    /// which focused investigation was requested.
    #[serde(default)]
    pub selectors: Vec<String>,
}

/// Inspectable state captured immediately after one graph-build pass.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphBuildStageTrace {
    /// Completed pass.
    pub pass: GraphBuildPass,
    /// Node count after the pass.
    pub node_count: usize,
    /// Relation count after the pass.
    pub relation_count: usize,
    /// BLAKE3 hash of the canonical graph JSON after the pass.
    pub graph_hash: String,
    /// Wall-clock duration excluded from correctness hashes and diffs.
    #[serde(skip)]
    pub duration_us: u64,
    /// Non-canonical component timings retained only in memory for lab observations.
    #[serde(skip)]
    pub component_durations_us: BTreeMap<String, u64>,
    /// Pass-specific diagnostic counters.
    #[serde(default)]
    pub counters: BTreeMap<String, u64>,
    /// Focused explainable decisions. Summary traces may omit these.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<GraphDecisionTrace>,
    /// Complete graph when full tracing is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub graph: Option<Graph>,
}

/// Complete deterministic graph-build trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GraphBuildTrace {
    /// Trace schema version.
    pub trace_version: u32,
    /// Graph-pipeline semantics version.
    pub pipeline_version: u32,
    /// Requested trace selectors.
    pub selectors: Vec<String>,
    /// Ordered pass states.
    pub stages: Vec<GraphBuildStageTrace>,
}

impl GraphBuildTrace {
    fn new(config: &GraphBuildTraceConfig) -> Self {
        let mut selectors = config.selectors.clone();
        selectors.sort();
        selectors.dedup();
        Self {
            trace_version: GRAPH_BUILD_TRACE_VERSION,
            pipeline_version: GRAPH_BUILD_PIPELINE_VERSION,
            selectors,
            stages: Vec::new(),
        }
    }

    fn record(
        &mut self,
        pass: GraphBuildPass,
        graph: &Graph,
        detail: &GraphBuildTraceDetail,
        started: Instant,
        counters: BTreeMap<String, u64>,
        decisions: Vec<GraphDecisionTrace>,
    ) {
        let payload = graph.to_json().unwrap_or_default();
        self.stages.push(GraphBuildStageTrace {
            pass,
            node_count: graph.nodes.len(),
            relation_count: graph.relations.len(),
            graph_hash: blake3::hash(payload.as_bytes()).to_hex().to_string(),
            duration_us: started.elapsed().as_micros().try_into().unwrap_or(u64::MAX),
            component_durations_us: BTreeMap::new(),
            counters,
            decisions,
            graph: (*detail == GraphBuildTraceDetail::Full).then(|| graph.clone()),
        });
    }
}

/// Complete typed output of graph construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphBuildOutput {
    /// Final normalized graph.
    pub graph: Graph,
    /// Pipeline semantics version used to create it.
    pub pipeline_version: u32,
    /// Ordered, observable pass outputs.
    pub passes: Vec<GraphBuildPassResult>,
    /// Optional diagnostic trace. Normal production builds leave this absent.
    pub trace: Option<GraphBuildTrace>,
}

impl GraphBuildOutput {
    /// Records a pass result from the current graph state.
    pub(crate) fn record(&mut self, pass: GraphBuildPass) {
        self.passes.push(GraphBuildPassResult {
            pass,
            node_count: self.graph.nodes.len(),
            relation_count: self.graph.relations.len(),
        });
    }

    pub(crate) fn enable_trace(&mut self, config: &GraphBuildTraceConfig) {
        self.trace = Some(GraphBuildTrace::new(config));
    }

    pub(crate) fn record_trace(
        &mut self,
        pass: GraphBuildPass,
        detail: &GraphBuildTraceDetail,
        started: Instant,
        counters: BTreeMap<String, u64>,
        decisions: Vec<GraphDecisionTrace>,
    ) {
        if let Some(trace) = &mut self.trace {
            trace.record(pass, &self.graph, detail, started, counters, decisions);
        }
    }

    pub(crate) fn record_component_duration(
        &mut self,
        pass: GraphBuildPass,
        name: &str,
        duration_us: u64,
    ) {
        if let Some(stage) = self
            .trace
            .as_mut()
            .and_then(|trace| trace.stages.iter_mut().find(|stage| stage.pass == pass))
        {
            stage
                .component_durations_us
                .insert(name.to_owned(), duration_us);
        }
    }
}
