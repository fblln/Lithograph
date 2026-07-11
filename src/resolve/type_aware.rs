//! Optional, deterministic type-aware resolution over baseline graph facts.

use crate::graph::{Graph, RelationKind, RelationProvenance, RelationResolution};
use crate::resolve::{ImportLookup, ImportMap, ResolverContext};

/// Capability level exposed by a language-specific type-aware resolver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeAwareCapability {
    /// No optional type-aware resolver is available; baseline syntax remains.
    Unavailable,
    /// Resolves uniquely identifiable type names using graph declaration facts.
    UniqueName,
}

/// High-value language capability metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeAwareLanguage {
    /// Stable language registry id.
    pub language: &'static str,
    /// Optional resolver capability.
    pub capability: TypeAwareCapability,
    /// Conservative known limitation.
    pub limitation: &'static str,
}

/// Supported type-aware resolver capabilities and limitations.
pub const TYPE_AWARE_LANGUAGES: &[TypeAwareLanguage] = &[
    TypeAwareLanguage {
        language: "python",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No runtime receiver inference.",
    },
    TypeAwareLanguage {
        language: "rust",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No trait/generic monomorphization.",
    },
    TypeAwareLanguage {
        language: "typescript",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No compiler-program type checking.",
    },
    TypeAwareLanguage {
        language: "javascript",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No dynamic receiver inference.",
    },
    TypeAwareLanguage {
        language: "java",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No classpath or overload inference.",
    },
    TypeAwareLanguage {
        language: "c_sharp",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No assembly or overload inference.",
    },
    TypeAwareLanguage {
        language: "go",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No package type checking.",
    },
    TypeAwareLanguage {
        language: "c",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No preprocessor or pointer inference.",
    },
    TypeAwareLanguage {
        language: "cpp",
        capability: TypeAwareCapability::UniqueName,
        limitation: "No template or overload inference.",
    },
];

/// Runs optional type-aware upgrades when explicitly enabled.
pub fn resolve_type_aware(graph: &mut Graph, enabled: bool) -> usize {
    if !enabled {
        return 0;
    }
    let context = ResolverContext::build(graph);
    let unresolved = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            crate::graph::GraphNode::Unresolved(value) => {
                Some((node.id().clone(), value.value.clone()))
            }
            _ => None,
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut upgrades = Vec::new();
    for (index, relation) in graph.relations.iter().enumerate() {
        if !matches!(
            relation.kind,
            RelationKind::TypeRefs | RelationKind::UsesType
        ) {
            continue;
        }
        let Some(language) = relation
            .provenance
            .as_ref()
            .and_then(|value| value.language.as_deref())
        else {
            continue;
        };
        if !TYPE_AWARE_LANGUAGES.iter().any(|entry| {
            entry.language == language && entry.capability == TypeAwareCapability::UniqueName
        }) {
            continue;
        }
        let Some(value) = unresolved.get(&relation.target) else {
            continue;
        };
        if let ImportLookup::Suffix { target, confidence }
        | ImportLookup::UniqueName { target, confidence } =
            ImportMap::new(&context.symbols).lookup(None, None, value)
        {
            upgrades.push((index, target, confidence));
        }
    }
    let count = upgrades.len();
    for (index, target, confidence) in upgrades {
        let relation = &mut graph.relations[index];
        relation.target = target;
        relation.confidence = confidence;
        let language = relation
            .provenance
            .as_ref()
            .and_then(|value| value.language.clone());
        relation.provenance = Some(RelationProvenance {
            language,
            resolver_strategy: "optional-type-aware-unique-name".to_owned(),
            resolution: RelationResolution::HybridResolved,
            confidence,
        });
    }
    count
}

#[cfg(test)]
mod tests {
    use super::{TYPE_AWARE_LANGUAGES, TypeAwareCapability, resolve_type_aware};
    use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath};
    use crate::graph::{
        Graph, GraphNode, GraphNodeId, Relation, RelationKind, RelationProvenance,
        RelationResolution, SymbolKind, SymbolNode, UnresolvedNode,
    };
    #[test]
    fn documents_every_high_value_language() {
        assert_eq!(TYPE_AWARE_LANGUAGES.len(), 9);
        assert!(
            TYPE_AWARE_LANGUAGES
                .iter()
                .all(|entry| entry.capability != TypeAwareCapability::Unavailable)
        );
    }

    #[test]
    fn disabled_mode_is_a_no_op_and_enabled_mode_upgrades_a_unique_type() {
        let path = RepoPath::new("src/a.py").unwrap_or_else(|_| unreachable!());
        let evidence = EvidenceRef::file(ArtifactId::from_path(&path), path);
        let mut graph = Graph {
            nodes: vec![
                GraphNode::Symbol(SymbolNode {
                    id: GraphNodeId::new("symbol:Base"),
                    kind: SymbolKind::Class,
                    qualified_name: "app::Base".to_owned(),
                    doc: None,
                    evidence: evidence.clone(),
                }),
                GraphNode::Unresolved(UnresolvedNode {
                    id: GraphNodeId::new("unresolved:Base"),
                    value: "Base".to_owned(),
                }),
            ],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source: GraphNodeId::new("symbol:Base"),
                target: GraphNodeId::new("unresolved:Base"),
                kind: RelationKind::UsesType,
                confidence: Confidence::Low,
                evidence: vec![evidence],
                provenance: Some(RelationProvenance {
                    language: Some("python".to_owned()),
                    resolver_strategy: "syntax-extraction".to_owned(),
                    resolution: RelationResolution::SyntaxOnly,
                    confidence: Confidence::Low,
                }),
            }],
        };
        let baseline = graph.clone();
        assert_eq!(resolve_type_aware(&mut graph, false), 0);
        assert_eq!(graph, baseline);
        assert_eq!(resolve_type_aware(&mut graph, true), 1);
        assert_eq!(graph.relations[0].target, GraphNodeId::new("symbol:Base"));
        assert_eq!(graph.relations[0].confidence, Confidence::Low);
        assert_eq!(
            graph.relations[0]
                .provenance
                .as_ref()
                .map(|value| value.resolver_strategy.as_str()),
            Some("optional-type-aware-unique-name")
        );
    }
}
