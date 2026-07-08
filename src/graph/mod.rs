//! Typed semantic graph: the merged, evidence-backed source of truth built
//! from repository artifacts and their analyzer output.

pub mod builder;
pub mod index;
pub mod model;
pub mod store;
pub mod validate;

pub use builder::GraphBuilder;
pub use index::{
    ArchitectureAspect, ArchitectureCluster, ArchitectureSummary, FileTreeNode, GraphSchema,
    KnowledgeIndex, LabelCount, LanguageSummary, PackageSummary, SearchParams, SearchResult,
    TraceDirection, TraceParams, TraceRelation, TraceResult, TypeCount,
};
pub use model::{
    ArtifactNode, CommandNode, ConfigNode, ConfigNodeKind, ContainerImageNode, DocumentationNode,
    EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, PackageNode, Relation,
    RelationKind, RelationProvenance, RelationResolution, SymbolKind, SymbolNode, UnresolvedNode,
};
pub use store::{
    GRAPH_ARTIFACT_FORMAT_VERSION, GRAPH_MODEL_VERSION, GRAPH_STORE_SCHEMA_VERSION,
    GraphArtifactMetadata, GraphArtifactReport, GraphSnapshot, GraphStore, GraphStoreMetadata,
    GraphStoreWriteOutcome,
};
pub use validate::{GraphIssue, GraphIssueKind, GraphValidator};
