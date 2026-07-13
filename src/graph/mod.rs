//! Typed semantic graph: the merged, evidence-backed source of truth built
//! from repository artifacts and their analyzer output.

pub mod analytics;
pub mod builder;
pub mod communities;
pub mod enrichment;
pub mod health;
pub mod index;
pub mod ladybug_schema;
pub mod ladybug_store;
pub mod layout;
pub mod model;
pub mod parity_benchmark;
pub mod pipeline;
pub mod query_api;
pub mod semantic;
pub mod store;
pub mod tags;
pub mod tensions;
pub mod validate;

pub use analytics::{MetricSnapshot, MetricSnapshotStore, NodeMetric};
pub use builder::GraphBuilder;
pub use communities::{
    CommunityAnalysis, CommunityDiagnostics, CommunityScope, CommunitySnapshot,
    CommunitySnapshotStore, CommunitySummary, CommunityTopic, LEIDEN_ALGORITHM_VERSION,
    TOPIC_ALGORITHM_VERSION, TopicSnapshot, TopicSnapshotStore, analyze_communities,
    environment_aware_scope, label_topic_snapshot, leiden_communities,
    leiden_communities_with_diagnostics,
};
pub use enrichment::{ENRICHMENT_ALGORITHM_VERSION, EnrichmentOverlay, derive_enrichment};
pub use health::{HealthFinding, HealthRule, HealthSeverity, HealthThresholds, detect_health};
pub use index::{
    ArchitectureAspect, ArchitectureCluster, ArchitectureSummary, DependencyMatrix, FileTreeNode,
    GraphSchema, KnowledgeIndex, LabelCount, LanguageSummary, PackageSummary, SearchParams,
    SearchResult, TraceDirection, TraceParams, TraceRelation, TraceResult, TypeCount,
};
pub use ladybug_schema::{
    LADYBUG_ALGORITHM_VERSION, LADYBUG_SCHEMA_VERSION, LADYBUG_TABLES_V1, LadybugTable,
    creation_statements as ladybug_creation_statements, migration_id as ladybug_migration_id,
};
pub use ladybug_store::LadybugGraphStore;
pub use layout::{
    LAYOUT_ALGORITHM_VERSION, LayoutBudget, LayoutEdge, LayoutRequest, LayoutResult,
    LayoutSnapshot, LayoutSnapshotStore, PositionedNode, compute_layout, compute_layout_cached,
};
pub use model::{
    ArtifactNode, CommandNode, ConfigNode, ConfigNodeKind, ContainerImageNode, DocumentationNode,
    EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, PackageNode, Relation,
    RelationKind, RelationProvenance, RelationResolution, SymbolKind, SymbolNode, UnresolvedNode,
};
pub use parity_benchmark::{ParityBenchmark, measure as measure_parity_benchmark};
pub use pipeline::{
    GRAPH_BUILD_PASS_ORDER, GRAPH_BUILD_PIPELINE_VERSION, GRAPH_BUILD_TRACE_VERSION,
    GraphBuildOutput, GraphBuildPass, GraphBuildPassResult, GraphBuildStageTrace, GraphBuildTrace,
    GraphBuildTraceConfig, GraphBuildTraceDetail, GraphDecisionTrace,
};
pub use query_api::{LadybugQueryApi, NeighborhoodQuery, RawQueryAccess};
pub use semantic::{
    SemanticClassMatch, SemanticClassProfile, SemanticScore, class_profiles, filter_classes,
};
pub use store::{
    GRAPH_ARTIFACT_FORMAT_VERSION, GRAPH_MODEL_VERSION, GRAPH_STORE_SCHEMA_VERSION,
    GraphArtifactMetadata, GraphArtifactReport, GraphSnapshot, GraphStore, GraphStoreMetadata,
    GraphStoreWriteOutcome,
};
pub use tags::{GraphTag, TagIndex, TagSource, derive_tags, inherit_tag, resolve_expression};
pub use tensions::{
    RepositoryTension, TENSION_ALGORITHM_VERSION, TensionCategory, TensionSnapshot,
    TensionSnapshotStore, score_tensions,
};
pub use validate::{GraphIssue, GraphIssueKind, GraphValidator};
