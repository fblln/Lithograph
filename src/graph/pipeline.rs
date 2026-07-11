//! Typed, observable contracts for deterministic graph construction passes.

use crate::graph::Graph;
use serde::{Deserialize, Serialize};

/// Bump when graph-pass ordering or semantics change.
pub const GRAPH_BUILD_PIPELINE_VERSION: u32 = 2;

/// A deterministic phase in graph construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphBuildPass {
    /// Create artifact and module structure.
    Structure,
    /// Extract and materialize definitions and imports.
    DefinitionsAndImports,
    /// Apply semantic enrichment such as clone detection.
    Enrichment,
    /// Resolve calls, types, imports, and usages against project indexes.
    Resolution,
    /// Compute graph-derived analytics without changing semantic topology.
    Analytics,
    /// Prepare the normalized graph for durable storage.
    Persistence,
    /// Normalize graph invariants and prepare the persisted graph payload.
    Finalize,
}

/// Stable execution order for every graph build.
pub const GRAPH_BUILD_PASS_ORDER: &[GraphBuildPass] = &[
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
pub struct GraphBuildPassResult {
    /// Completed pass.
    pub pass: GraphBuildPass,
    /// Node count after the pass.
    pub node_count: usize,
    /// Relation count after the pass.
    pub relation_count: usize,
}

/// Complete typed output of graph construction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphBuildOutput {
    /// Final normalized graph.
    pub graph: Graph,
    /// Pipeline semantics version used to create it.
    pub pipeline_version: u32,
    /// Ordered, observable pass outputs.
    pub passes: Vec<GraphBuildPassResult>,
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
}
