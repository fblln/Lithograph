//! Typed semantic graph: the merged, evidence-backed source of truth built
//! from repository artifacts and their analyzer output.

pub mod builder;
pub mod model;
pub mod validate;

pub use builder::GraphBuilder;
pub use model::{
    ArtifactNode, CommandNode, ConfigNode, ConfigNodeKind, ContainerImageNode, DocumentationNode,
    EnvVarNode, Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, PackageNode, Relation,
    RelationKind, SymbolKind, SymbolNode, UnresolvedNode,
};
pub use validate::{GraphIssue, GraphIssueKind, GraphValidator};
