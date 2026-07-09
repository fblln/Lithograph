//! Typed semantic graph: nodes, relations, and the exported graph shape.

use crate::domain::{ArtifactCategory, Confidence, EvidenceRef};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// Stable, kind-prefixed graph node identifier (e.g. `artifact:src/lib.rs`,
/// `symbol:src/lib.rs#RouteBaker`, `env:RIDGELINE_WORKER`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphNodeId(String);

impl GraphNodeId {
    /// Wraps an already-formatted, kind-prefixed identifier.
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Returns the identifier as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for GraphNodeId {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// One typed semantic graph node.
///
/// Tagged `node_type` rather than `kind`: several variants (`Symbol`,
/// `Config`) already have their own inner `kind` field (`SymbolKind`,
/// `ConfigNodeKind`), and an internally-tagged enum inserts its tag as a
/// sibling field in the same JSON object, so reusing `kind` here collided
/// with those and made the JSON undeserializable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "node_type")]
pub enum GraphNode {
    /// A repository artifact from inventory.
    Artifact(ArtifactNode),
    /// A code symbol (class, function, method, struct, enum, trait).
    Symbol(SymbolNode),
    /// A configuration entity (service, job, port, or other named value).
    Config(ConfigNode),
    /// A documentation entity (heading).
    Documentation(DocumentationNode),
    /// A container image reference.
    Container(ContainerImageNode),
    /// A shell command invocation.
    Command(CommandNode),
    /// An environment variable, deduplicated by name across the repository.
    EnvVar(EnvVarNode),
    /// A source module (Python dotted path or Rust `::` path).
    Module(ModuleNode),
    /// A package or crate, local or external.
    Package(PackageNode),
    /// A reference Lithograph could not resolve to another node.
    Unresolved(UnresolvedNode),
}

impl GraphNode {
    /// Returns this node's identifier.
    pub fn id(&self) -> &GraphNodeId {
        match self {
            Self::Artifact(node) => &node.id,
            Self::Symbol(node) => &node.id,
            Self::Config(node) => &node.id,
            Self::Documentation(node) => &node.id,
            Self::Container(node) => &node.id,
            Self::Command(node) => &node.id,
            Self::EnvVar(node) => &node.id,
            Self::Module(node) => &node.id,
            Self::Package(node) => &node.id,
            Self::Unresolved(node) => &node.id,
        }
    }
}

/// Repository artifact node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Repository-relative path.
    pub path: String,
    /// Coarse artifact category.
    pub category: ArtifactCategory,
    /// Evidence for this artifact.
    pub evidence: EvidenceRef,
}

/// Code symbol category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SymbolKind {
    /// Python class.
    Class,
    /// Python or Rust function.
    Function,
    /// Method declared on a class.
    Method,
    /// Rust struct.
    Struct,
    /// Rust enum.
    Enum,
    /// Rust trait.
    Trait,
    /// Definition-like syntax fact from a generic syntax-indexed language
    /// (see LIT-22.2.3), e.g. a class, function, or struct in a language
    /// with a tree-sitter adapter but no specialized deep analyzer.
    Definition,
}

/// Code symbol node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Symbol category.
    pub kind: SymbolKind,
    /// Fully qualified name (module/class-scoped).
    pub qualified_name: String,
    /// Doc comment or docstring, when present.
    pub doc: Option<String>,
    /// Evidence for this symbol.
    pub evidence: EvidenceRef,
}

/// Configuration entity category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigNodeKind {
    /// Docker Compose service.
    Service,
    /// GitHub Actions job.
    Job,
    /// Network port.
    Port,
    /// An HTTP route, gRPC/protobuf RPC, or GraphQL Query/Mutation field
    /// (LIT-22.3.4). `ConfigNode::name` is `"METHOD path"` for HTTP (e.g.
    /// `"GET /users/{id}"`), `"service.rpc"` for gRPC, or the GraphQL field
    /// name.
    Route,
    /// Other named configuration value.
    Value,
}

/// Configuration entity node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Entity category.
    pub kind: ConfigNodeKind,
    /// Entity name.
    pub name: String,
    /// Evidence for this entity.
    pub evidence: EvidenceRef,
}

/// Documentation entity node (a heading).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentationNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Heading text.
    pub title: String,
    /// Evidence for this heading.
    pub evidence: EvidenceRef,
}

/// Container image reference node, deduplicated by reference string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContainerImageNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Image reference as written.
    pub reference: String,
    /// True when the reference contains an unresolved template expression.
    pub is_dynamic: bool,
}

/// Shell command node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Command text.
    pub text: String,
    /// Evidence for this command.
    pub evidence: EvidenceRef,
}

/// Environment variable node, deduplicated by name across the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVarNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Variable name.
    pub name: String,
}

/// Source language for a `Module` node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModuleLanguage {
    /// Python module.
    Python,
    /// Rust module.
    Rust,
    /// A generic syntax-indexed language (see LIT-22.2.3), backed by a
    /// tree-sitter adapter but not yet a specialized deep analyzer.
    SyntaxIndexed(crate::analysis::SyntaxIndexedLanguage),
}

/// Source module node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Dotted (Python) or `::`-joined (Rust) module path.
    pub path: String,
    /// Source language.
    pub language: ModuleLanguage,
    /// Evidence for the defining artifact.
    pub evidence: EvidenceRef,
}

/// Package or crate node, deduplicated by name across the repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackageNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Package or crate name.
    pub name: String,
    /// True when this package is an external dependency, not built in-repo.
    pub is_external: bool,
}

/// Unresolved reference node, deduplicated by literal value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnresolvedNode {
    /// Node identifier.
    pub id: GraphNodeId,
    /// Literal, unresolved value.
    pub value: String,
}

/// Relation category between two graph nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RelationKind {
    /// The source node contains or owns the target node.
    Contains,
    /// An artifact belongs to a module.
    BelongsToModule,
    /// An artifact or module belongs to a package.
    BelongsToPackage,
    /// A package depends on another (possibly external) package.
    DependsOnPackage,
    /// An import/use statement.
    Imports,
    /// A same-file call to another symbol.
    Calls,
    /// An environment variable read.
    ReadsEnv,
    /// A command invocation.
    RunsCommand,
    /// A container image is used as a base or runtime image.
    UsesImage,
    /// A container image is built.
    BuildsImage,
    /// A container image is published.
    PublishesImage,
    /// A type implements a trait.
    Implements,
    /// A class/type inherits from (extends) a base class/type.
    Inherits,
    /// A definition references another type by name (field type, parameter
    /// type, return type, generic argument).
    TypeRefs,
    /// A file-scoped reference to a symbol name that is neither a
    /// definition nor an import (a use site).
    Usages,
    /// A foreign-function-interface boundary: a function or static
    /// declared `extern` for another ABI.
    Ffi,
    /// A generic reference (path, URL, dynamic import, ctypes, service dependency).
    References,
    /// Publishes/emits an event or message onto a channel (LIT-22.3.5).
    Emits,
    /// Subscribes/listens for an event or message on a channel (LIT-22.3.5).
    ListensOn,
    /// Data flows from the source symbol into the target symbol via a call
    /// argument or a `self.field` assignment (LIT-22.3.6 AC1).
    DataFlows,
    /// Two symbols are near-clones by deterministic lexical/structural
    /// similarity (LIT-22.3.6 AC2) -- never live embeddings.
    SimilarTo,
}

/// How a relation was extracted or resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelationResolution {
    /// Parser or structured syntax identified the relation, with no cross-file refinement.
    SyntaxOnly,
    /// Generic text or heuristic fallback identified the relation.
    Fallback,
    /// Syntax facts were refined against package/module/reference indexes.
    HybridResolved,
}

/// Provenance for a graph relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationProvenance {
    /// Detected language or format responsible for the relation, when known.
    pub language: Option<String>,
    /// Stable resolver strategy label.
    pub resolver_strategy: String,
    /// Relation extraction/resolution level.
    pub resolution: RelationResolution,
    /// Confidence assigned by that resolver.
    pub confidence: Confidence,
}

/// One relation between two graph nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Relation {
    /// Relation identifier, unique within one graph.
    pub id: String,
    /// Source node.
    pub source: GraphNodeId,
    /// Target node (may be an `Unresolved` node when resolution failed).
    pub target: GraphNodeId,
    /// Relation category.
    pub kind: RelationKind,
    /// Confidence in this relation.
    pub confidence: Confidence,
    /// Evidence supporting this relation.
    pub evidence: Vec<EvidenceRef>,
    /// Resolver provenance for this relation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<RelationProvenance>,
}

/// The complete typed semantic graph for one repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Graph {
    /// All graph nodes, sorted by id for deterministic export.
    pub nodes: Vec<GraphNode>,
    /// All relations, sorted by (source, kind, target) for deterministic export.
    pub relations: Vec<Relation>,
}

impl Graph {
    /// Renders the graph as deterministic pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }
}
