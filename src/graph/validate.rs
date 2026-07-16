//! Graph invariant validation.

use crate::domain::Artifact;
use crate::graph::model::{Graph, GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

/// Category of a detected graph invariant violation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphIssueKind {
    /// A relation's source node ID does not exist in the graph.
    DanglingRelationSource,
    /// A relation's target node ID does not exist in the graph.
    DanglingRelationTarget,
    /// A relation's target node kind is not valid for the relation kind.
    InvalidRelationTarget,
    /// Evidence references an artifact ID absent from the known artifact set.
    MissingEvidenceArtifact,
    /// Evidence references a line span past the end of its artifact.
    InvalidSourceSpan,
}

/// One detected graph invariant violation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphIssue {
    /// Issue category.
    pub kind: GraphIssueKind,
    /// Human-readable, actionable description.
    pub message: String,
}

impl Display for GraphIssue {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

/// Validates graph invariants: dangling references, mistyped relation
/// targets, evidence pointing at unknown artifacts, and evidence spans past
/// the end of their artifact.
#[derive(Debug, Clone, Copy, Default)]
pub struct GraphValidator;

impl GraphValidator {
    /// Validates `graph` against the artifacts it was built from. An empty
    /// result means the graph is valid.
    pub fn validate(&self, graph: &Graph, artifacts: &[Artifact]) -> Vec<GraphIssue> {
        let node_ids: BTreeSet<&GraphNodeId> = graph.nodes.iter().map(GraphNode::id).collect();
        let node_kinds: BTreeMap<&GraphNodeId, NodeKindTag> = graph
            .nodes
            .iter()
            .map(|node| (node.id(), node_kind_tag(node)))
            .collect();
        let known_artifacts: BTreeMap<&str, Option<u32>> = artifacts
            .iter()
            .map(|artifact| (artifact.path.as_str(), artifact.line_count))
            .collect();

        let mut issues = Vec::new();

        for relation in &graph.relations {
            if !node_ids.contains(&relation.source) {
                issues.push(GraphIssue {
                    kind: GraphIssueKind::DanglingRelationSource,
                    message: format!(
                        "relation {} has source {} which is not a graph node",
                        relation.id, relation.source
                    ),
                });
            }
            match node_kinds.get(&relation.target) {
                None => issues.push(GraphIssue {
                    kind: GraphIssueKind::DanglingRelationTarget,
                    message: format!(
                        "relation {} has target {} which is not a graph node",
                        relation.id, relation.target
                    ),
                }),
                Some(kind) => {
                    if !target_kind_allowed(relation.kind, *kind) {
                        issues.push(GraphIssue {
                            kind: GraphIssueKind::InvalidRelationTarget,
                            message: format!(
                                "relation {} of kind {:?} targets {} ({:?}), which is not a valid target for this relation kind",
                                relation.id, relation.kind, relation.target, kind
                            ),
                        });
                    }
                }
            }
        }

        for node in &graph.nodes {
            for evidence in node_evidence(node) {
                validate_evidence(evidence, &known_artifacts, &mut issues);
            }
        }
        for relation in &graph.relations {
            for evidence in &relation.evidence {
                validate_evidence(evidence, &known_artifacts, &mut issues);
            }
        }

        issues
    }
}

fn validate_evidence(
    evidence: &crate::domain::EvidenceRef,
    known_artifacts: &BTreeMap<&str, Option<u32>>,
    issues: &mut Vec<GraphIssue>,
) {
    let Some(line_count) = known_artifacts.get(evidence.path.as_str()) else {
        issues.push(GraphIssue {
            kind: GraphIssueKind::MissingEvidenceArtifact,
            message: format!(
                "evidence references artifact {} ({}) which is not a known artifact",
                evidence.artifact_id, evidence.path
            ),
        });
        return;
    };
    if let (Some(span), Some(line_count)) = (&evidence.span, line_count)
        && span.end_line > *line_count
    {
        issues.push(GraphIssue {
            kind: GraphIssueKind::InvalidSourceSpan,
            message: format!(
                "evidence for {} spans lines {}-{} but the artifact has only {} lines",
                evidence.path, span.start_line, span.end_line, line_count
            ),
        });
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum NodeKindTag {
    Artifact,
    Symbol,
    Config,
    Documentation,
    Container,
    Command,
    EnvVar,
    Module,
    Package,
    Unresolved,
    Rationale,
}

pub(crate) fn node_kind_tag(node: &GraphNode) -> NodeKindTag {
    match node {
        GraphNode::Artifact(_) => NodeKindTag::Artifact,
        GraphNode::Symbol(_) => NodeKindTag::Symbol,
        GraphNode::Config(_) => NodeKindTag::Config,
        GraphNode::Documentation(_) => NodeKindTag::Documentation,
        GraphNode::Container(_) => NodeKindTag::Container,
        GraphNode::Command(_) => NodeKindTag::Command,
        GraphNode::EnvVar(_) => NodeKindTag::EnvVar,
        GraphNode::Module(_) => NodeKindTag::Module,
        GraphNode::Package(_) => NodeKindTag::Package,
        GraphNode::Unresolved(_) => NodeKindTag::Unresolved,
        GraphNode::Rationale(_) => NodeKindTag::Rationale,
    }
}

fn node_evidence(node: &GraphNode) -> Vec<&crate::domain::EvidenceRef> {
    match node {
        GraphNode::Artifact(node) => vec![&node.evidence],
        GraphNode::Symbol(node) => vec![&node.evidence],
        GraphNode::Config(node) => vec![&node.evidence],
        GraphNode::Documentation(node) => vec![&node.evidence],
        GraphNode::Command(node) => vec![&node.evidence],
        GraphNode::Module(node) => vec![&node.evidence],
        GraphNode::Rationale(node) => vec![&node.evidence],
        GraphNode::Container(_)
        | GraphNode::EnvVar(_)
        | GraphNode::Package(_)
        | GraphNode::Unresolved(_) => Vec::new(),
    }
}

// ponytail: allow-list is intentionally permissive (Unresolved is always
// accepted, and structural kinds like Contains/References/Calls accept many
// target kinds) so this catches clear mismatches (e.g. ReadsEnv -> Symbol)
// without becoming a second copy of the builder's own dispatch logic.
pub(crate) fn target_kind_allowed(kind: RelationKind, target: NodeKindTag) -> bool {
    if target == NodeKindTag::Unresolved {
        return true;
    }
    match kind {
        RelationKind::Contains => true,
        // A note explains the code it sits inside: the enclosing symbol
        // when one can be determined, else the artifact holding it.
        RelationKind::RationaleFor => {
            matches!(target, NodeKindTag::Symbol | NodeKindTag::Artifact)
        }
        RelationKind::BelongsToModule => target == NodeKindTag::Module,
        RelationKind::BelongsToPackage | RelationKind::DependsOnPackage => {
            target == NodeKindTag::Package
        }
        // LIT-23.1: the generic hybrid resolver pipeline (src/resolve/mod.rs)
        // upgrades any SyntaxOnly/Fallback relation -- regardless of kind --
        // whose Unresolved value matches a known package name or local
        // artifact path, so an Imports or generic reference relation can
        // legitimately end up targeting an Artifact or Package node, not
        // just a Module/Symbol as when only the specialized Python/Rust
        // analyzers produced hybrid-resolved relations.
        RelationKind::Imports => matches!(
            target,
            NodeKindTag::Module | NodeKindTag::Package | NodeKindTag::Artifact
        ),
        RelationKind::Calls => matches!(target, NodeKindTag::Symbol),
        RelationKind::ReadsEnv | RelationKind::DefinesEnv => target == NodeKindTag::EnvVar,
        RelationKind::BindsConfig | RelationKind::ReferencesConfig => target == NodeKindTag::Config,
        RelationKind::RunsCommand => target == NodeKindTag::Command,
        RelationKind::UsesImage | RelationKind::BuildsImage | RelationKind::PublishesImage => {
            target == NodeKindTag::Container
        }
        RelationKind::TypeRefs | RelationKind::Usages => matches!(
            target,
            NodeKindTag::Symbol
                | NodeKindTag::Module
                | NodeKindTag::Package
                | NodeKindTag::Artifact
        ),
        RelationKind::Implements
        | RelationKind::Inherits
        | RelationKind::Decorates
        | RelationKind::HasMethod
        | RelationKind::MemberOf
        | RelationKind::UsesType
        | RelationKind::Ffi
        | RelationKind::DataFlows
        | RelationKind::SimilarTo => matches!(target, NodeKindTag::Symbol),
        RelationKind::HandlesRoute => target == NodeKindTag::Symbol,
        RelationKind::Tests | RelationKind::FileChangesWith | RelationKind::DocumentsSource => {
            matches!(target, NodeKindTag::Artifact | NodeKindTag::Symbol)
        }
        RelationKind::Reads
        | RelationKind::Writes
        | RelationKind::CrossesServiceBoundary
        | RelationKind::References
        | RelationKind::Emits
        | RelationKind::ListensOn => true,
    }
}

#[cfg(test)]
mod tests {
    use super::{GraphIssueKind, GraphValidator};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, EvidenceRef, RepoPath, SourceSpan, SupportTier,
        TextStatus,
    };
    use crate::graph::model::{
        ArtifactNode, EnvVarNode, Graph, GraphNode, GraphNodeId, Relation, RelationKind,
    };
    use crate::graph::{ContainerImageNode, GraphBuilder};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn artifact(
        path: &str,
        line_count: Option<u32>,
    ) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::GenericText,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_text_status(TextStatus::Text, line_count))
    }

    #[test]
    fn fixture_graph_is_valid() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let issues = GraphValidator.validate(&graph, &artifacts);

        assert!(issues.is_empty(), "unexpected issues: {issues:?}");

        Ok(())
    }

    #[test]
    fn detects_dangling_relation_endpoints() -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("src/lib.rs", Some(5))?];
        let graph = Graph {
            nodes: vec![GraphNode::Artifact(ArtifactNode {
                id: GraphNodeId::new("artifact:src/lib.rs"),
                path: "src/lib.rs".to_owned(),
                category: ArtifactCategory::SourceCode,
                evidence: file_evidence(&artifacts[0]),
            })],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source: GraphNodeId::new("artifact:src/lib.rs"),
                target: GraphNodeId::new("symbol:does-not-exist"),
                kind: RelationKind::Contains,
                confidence: crate::domain::Confidence::High,
                evidence: vec![file_evidence(&artifacts[0])],
                provenance: None,
            }],
        };

        let issues = GraphValidator.validate(&graph, &artifacts);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, GraphIssueKind::DanglingRelationTarget);

        Ok(())
    }

    #[test]
    fn detects_invalid_relation_target_kind() -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("src/lib.rs", Some(5))?];
        let graph = Graph {
            nodes: vec![
                GraphNode::Artifact(ArtifactNode {
                    id: GraphNodeId::new("artifact:src/lib.rs"),
                    path: "src/lib.rs".to_owned(),
                    category: ArtifactCategory::SourceCode,
                    evidence: file_evidence(&artifacts[0]),
                }),
                GraphNode::EnvVar(EnvVarNode {
                    id: GraphNodeId::new("env:X"),
                    name: "X".to_owned(),
                }),
            ],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source: GraphNodeId::new("artifact:src/lib.rs"),
                target: GraphNodeId::new("env:X"),
                kind: RelationKind::UsesImage,
                confidence: crate::domain::Confidence::High,
                evidence: vec![file_evidence(&artifacts[0])],
                provenance: None,
            }],
        };

        let issues = GraphValidator.validate(&graph, &artifacts);

        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].kind, GraphIssueKind::InvalidRelationTarget);

        Ok(())
    }

    /// LIT-22.3.3 AC2/AC3: `Inherits`/`TypeRefs`/`Usages`/`Ffi` accept a
    /// `Symbol` target or an `Unresolved` one (never fabricated), but
    /// reject a structurally wrong target kind like `EnvVar`.
    #[test]
    fn new_semantic_relation_kinds_enforce_symbol_or_unresolved_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("src/lib.rs", Some(5))?];
        let base_nodes = vec![
            GraphNode::Artifact(ArtifactNode {
                id: GraphNodeId::new("artifact:src/lib.rs"),
                path: "src/lib.rs".to_owned(),
                category: ArtifactCategory::SourceCode,
                evidence: file_evidence(&artifacts[0]),
            }),
            GraphNode::Symbol(crate::graph::model::SymbolNode {
                id: GraphNodeId::new("symbol:Base"),
                kind: crate::graph::model::SymbolKind::Class,
                qualified_name: "Base".to_owned(),
                doc: None,
                evidence: file_evidence(&artifacts[0]),
            }),
            GraphNode::Unresolved(crate::graph::model::UnresolvedNode {
                id: GraphNodeId::new("unresolved:mystery"),
                value: "mystery".to_owned(),
            }),
            GraphNode::EnvVar(EnvVarNode {
                id: GraphNodeId::new("env:X"),
                name: "X".to_owned(),
            }),
        ];

        for kind in [
            RelationKind::Inherits,
            RelationKind::Decorates,
            RelationKind::HasMethod,
            RelationKind::MemberOf,
            RelationKind::UsesType,
            RelationKind::TypeRefs,
            RelationKind::Usages,
            RelationKind::Ffi,
        ] {
            let relation = |target: &str, confidence: crate::domain::Confidence| Relation {
                id: "relation:1".to_owned(),
                source: GraphNodeId::new("artifact:src/lib.rs"),
                target: GraphNodeId::new(target),
                kind,
                confidence,
                evidence: vec![file_evidence(&artifacts[0])],
                provenance: None,
            };

            let valid_symbol_target = Graph {
                nodes: base_nodes.clone(),
                relations: vec![relation("symbol:Base", crate::domain::Confidence::Low)],
            };
            assert_eq!(
                GraphValidator.validate(&valid_symbol_target, &artifacts),
                Vec::new(),
                "{kind:?} -> Symbol should be valid"
            );

            let valid_unresolved_target = Graph {
                nodes: base_nodes.clone(),
                relations: vec![relation(
                    "unresolved:mystery",
                    crate::domain::Confidence::Low,
                )],
            };
            assert_eq!(
                GraphValidator.validate(&valid_unresolved_target, &artifacts),
                Vec::new(),
                "{kind:?} -> Unresolved should be valid"
            );

            let invalid_target = Graph {
                nodes: base_nodes.clone(),
                relations: vec![relation("env:X", crate::domain::Confidence::High)],
            };
            let issues = GraphValidator.validate(&invalid_target, &artifacts);
            assert_eq!(
                issues.len(),
                1,
                "{kind:?} -> EnvVar should be exactly one issue"
            );
            assert_eq!(issues[0].kind, GraphIssueKind::InvalidRelationTarget);
        }

        Ok(())
    }

    #[test]
    fn detects_missing_evidence_artifact_and_oversized_span()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifacts = vec![artifact("src/lib.rs", Some(3))?];
        let oversized_span = EvidenceRef::file(
            crate::domain::ArtifactId::from_path(&artifacts[0].path),
            artifacts[0].path.clone(),
        )
        .with_span(SourceSpan::new(1, 100)?);
        let missing_artifact = EvidenceRef::file(
            crate::domain::ArtifactId::from_path(&RepoPath::new("does/not/exist.rs")?),
            RepoPath::new("does/not/exist.rs")?,
        );
        let graph = Graph {
            nodes: vec![GraphNode::Container(ContainerImageNode {
                id: GraphNodeId::new("image:example"),
                reference: "example".to_owned(),
                is_dynamic: false,
            })],
            relations: vec![
                Relation {
                    id: "relation:1".to_owned(),
                    source: GraphNodeId::new("artifact:src/lib.rs"),
                    target: GraphNodeId::new("image:example"),
                    kind: RelationKind::UsesImage,
                    confidence: crate::domain::Confidence::High,
                    evidence: vec![oversized_span],
                    provenance: None,
                },
                Relation {
                    id: "relation:2".to_owned(),
                    source: GraphNodeId::new("artifact:src/lib.rs"),
                    target: GraphNodeId::new("image:example"),
                    kind: RelationKind::UsesImage,
                    confidence: crate::domain::Confidence::High,
                    evidence: vec![missing_artifact],
                    provenance: None,
                },
            ],
        };

        let issues = GraphValidator.validate(&graph, &artifacts);

        assert!(
            issues
                .iter()
                .any(|issue| issue.kind == GraphIssueKind::InvalidSourceSpan)
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.kind == GraphIssueKind::MissingEvidenceArtifact)
        );
        assert!(
            issues
                .iter()
                .any(|issue| issue.kind == GraphIssueKind::DanglingRelationSource)
        );

        Ok(())
    }

    fn file_evidence(artifact: &Artifact) -> EvidenceRef {
        EvidenceRef::file(
            crate::domain::ArtifactId::from_path(&artifact.path),
            artifact.path.clone(),
        )
    }
}
