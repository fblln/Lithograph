//! Typed semantic graph: the merged, evidence-backed source of truth built
//! from repository artifacts and their analyzer output.

pub(crate) mod analytics;
pub(crate) mod builder;
pub(crate) mod communities;
pub(crate) mod health;
pub(crate) mod index;
pub(crate) mod layout;
pub(crate) mod model;
pub(crate) mod pipeline;
pub(crate) mod report;
pub(crate) mod semantic;
pub(crate) mod store;
pub(crate) mod tags;
pub(crate) mod tensions;
pub(crate) mod validate;

pub(crate) use builder::GraphBuilder;
pub(crate) use communities::{
    CommunityScope, CommunitySnapshotStore, CommunitySummary, LEIDEN_ALGORITHM_VERSION,
    analyze_communities, architecture_aware_scope, environment_aware_scope, leiden_communities,
    leiden_communities_with_diagnostics,
};
pub(crate) use health::{
    HealthFinding, HealthRule, HealthSeverity, HealthThresholds, detect_health,
};
pub(crate) use index::{
    ArchitectureAspect, ArchitectureCluster, ArchitectureSummary, DependencyMatrix, GraphSchema,
    KnowledgeIndex, NodeExplanation, PathResult, SearchParams, TraceDirection, TraceParams,
    TraceResult,
};
pub(crate) use layout::{
    LayoutRequest, LayoutSnapshotStore, compute_layout, compute_layout_cached,
};
pub(crate) use model::{
    ArtifactNode, CommandProvenance, ConfigNodeKind, Graph, GraphNode, GraphNodeId, ModuleLanguage,
    ModuleNode, Relation, RelationKind, RelationProvenance, RelationResolution, SymbolKind,
};
// Node-variant types other modules only reference from their own tests.
#[cfg(test)]
pub(crate) use model::{
    CommandNode, ConfigNode, ContainerImageNode, EnvVarNode, PackageNode, SymbolNode,
    UnresolvedNode,
};
#[cfg(test)]
pub(crate) use pipeline::GRAPH_BUILD_PASS_ORDER;
pub(crate) use pipeline::{
    GRAPH_BUILD_PIPELINE_VERSION, GraphBuildOutput, GraphBuildPass, GraphBuildStageTrace,
    GraphBuildTraceConfig, GraphBuildTraceDetail, GraphDecisionTrace,
};
pub(crate) use report::{GraphReport, persist_graph_report};
pub(crate) use semantic::filter_classes;
pub(crate) use store::{
    GRAPH_MODEL_VERSION, GRAPH_STORE_SCHEMA_VERSION, GraphArtifactReport, GraphStore,
};
pub(crate) use tags::{
    GraphTag, TagIndex, TagSource, cluster_display_tags, derive_tags, relation_display_tags,
    resolve_expression, tension_display_tags,
};
#[cfg(test)]
pub(crate) use tensions::TensionCategory;
pub(crate) use tensions::{RepositoryTension, score_tensions};
#[cfg(test)]
pub(crate) use validate::GraphIssueKind;
pub(crate) use validate::{GraphIssue, GraphValidator};
pub(crate) use validate::{NodeKindTag, node_kind_tag, target_kind_allowed};
