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

pub(crate) use analytics::{MetricSnapshot, MetricSnapshotStore, NodeMetric};
pub(crate) use builder::GraphBuilder;
pub(crate) use communities::{
    CommunityAnalysis, CommunityDiagnostics, CommunityScope, CommunitySnapshot,
    CommunitySnapshotStore, CommunitySummary, CommunityTopic, LEIDEN_ALGORITHM_VERSION,
    TOPIC_ALGORITHM_VERSION, TopicSnapshot, TopicSnapshotStore, analyze_communities,
    architecture_aware_scope, environment_aware_scope, label_topic_snapshot, leiden_communities,
    leiden_communities_with_diagnostics,
};
pub(crate) use health::{HealthFinding, HealthRule, HealthSeverity, HealthThresholds, detect_health};
pub(crate) use index::{
    ArchitectureAspect, ArchitectureCluster, ArchitectureClusterLink, ArchitectureClusterLinkKind,
    ArchitectureClusterRelation, ArchitectureSummary, DependencyMatrix, FileTreeNode, GraphSchema,
    KnowledgeIndex, LabelCount, LanguageSummary, Neighbor, NodeExplanation, PackageSummary,
    PathHop, PathResult, SearchParams, SearchResult, TraceDirection, TraceParams, TraceRelation,
    TraceResult, TypeCount,
};
pub(crate) use layout::{
    LAYOUT_ALGORITHM_VERSION, LayoutBudget, LayoutEdge, LayoutRequest, LayoutResult,
    LayoutSnapshot, LayoutSnapshotStore, PositionedNode, compute_layout, compute_layout_cached,
};
pub(crate) use model::{
    ArtifactNode, CommandNode, CommandProvenance, ConfigNode, ConfigNodeKind, ContainerImageNode,
    DocumentationNode, EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode,
    PackageNode, Relation, RelationKind, RelationProvenance, RelationResolution, SymbolKind,
    SymbolNode, UnresolvedNode,
};
pub(crate) use pipeline::{
    GRAPH_BUILD_PASS_ORDER, GRAPH_BUILD_PIPELINE_VERSION, GRAPH_BUILD_TRACE_VERSION,
    GraphBuildOutput, GraphBuildPass, GraphBuildPassResult, GraphBuildStageTrace, GraphBuildTrace,
    GraphBuildTraceConfig, GraphBuildTraceDetail, GraphDecisionTrace,
};
pub(crate) use report::{GraphReport, graph_report_path, persist_graph_report};
pub(crate) use semantic::{
    SemanticClassMatch, SemanticClassProfile, SemanticScore, class_profiles, filter_classes,
};
pub(crate) use store::{
    GRAPH_ARTIFACT_FORMAT_VERSION, GRAPH_MODEL_VERSION, GRAPH_STORE_SCHEMA_VERSION,
    GraphArtifactMetadata, GraphArtifactReport, GraphSnapshot, GraphStore, GraphStoreMetadata,
    GraphStoreWriteOutcome,
};
pub(crate) use tags::{
    GraphTag, TagIndex, TagSource, cluster_display_tags, derive_tags, inherit_tag,
    relation_display_tags, resolve_expression, tension_display_tags,
};
pub(crate) use tensions::{
    RepositoryTension, TENSION_ALGORITHM_VERSION, TensionCategory, TensionSnapshot,
    TensionSnapshotStore, score_tensions,
};
pub(crate) use validate::{GraphIssue, GraphIssueKind, GraphValidator};
pub(crate) use validate::{NodeKindTag, node_kind_tag, target_kind_allowed};
