//! Optional deterministic graph-enrichment overlays.

use crate::domain::{Confidence, EvidenceRef};
use crate::graph::{
    Graph, GraphNode, GraphNodeId, Relation, RelationKind, RelationProvenance, RelationResolution,
};

/// Version for deterministic enrichment scoring semantics.
pub const ENRICHMENT_ALGORITHM_VERSION: u32 = 1;

/// Immutable optional enrichment output; applying it is caller-controlled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnrichmentOverlay {
    /// Algorithm version used to derive the relations.
    pub algorithm_version: u32,
    /// Derived relations, sorted deterministically.
    pub relations: Vec<Relation>,
}

/// Builds deterministic test and documentation/source relation overlays.
pub fn derive_enrichment(graph: &Graph, enabled: bool) -> EnrichmentOverlay {
    if !enabled {
        return EnrichmentOverlay {
            algorithm_version: ENRICHMENT_ALGORITHM_VERSION,
            relations: Vec::new(),
        };
    }
    let artifacts: Vec<_> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Artifact(node) => Some(node),
            _ => None,
        })
        .collect();
    let mut relations = Vec::new();
    for test in artifacts.iter().filter(|node| is_test_path(&node.path)) {
        let stem = test
            .path
            .rsplit('/')
            .next()
            .unwrap_or(&test.path)
            .trim_end_matches(".rs")
            .trim_end_matches(".py")
            .trim_start_matches("test_");
        for source in artifacts
            .iter()
            .filter(|node| !is_test_path(&node.path) && node.path.contains(stem))
        {
            relations.push(relation(
                "tests",
                &test.id,
                &source.id,
                RelationKind::Tests,
                test.evidence.clone(),
            ));
        }
    }
    for doc in graph.nodes.iter().filter_map(|node| match node {
        GraphNode::Documentation(node) => Some(node),
        _ => None,
    }) {
        for artifact in artifacts
            .iter()
            .filter(|artifact| artifact.path == doc.evidence.path.as_str())
        {
            relations.push(relation(
                "documents-source",
                &doc.id,
                &artifact.id,
                RelationKind::DocumentsSource,
                doc.evidence.clone(),
            ));
        }
    }
    relations.sort_by(|a, b| (&a.source, a.kind, &a.target).cmp(&(&b.source, b.kind, &b.target)));
    EnrichmentOverlay {
        algorithm_version: ENRICHMENT_ALGORITHM_VERSION,
        relations,
    }
}

fn is_test_path(path: &str) -> bool {
    path.contains("/tests/")
        || path.starts_with("tests/")
        || path
            .rsplit('/')
            .next()
            .is_some_and(|name| name.starts_with("test_"))
}
fn relation(
    prefix: &str,
    source: &GraphNodeId,
    target: &GraphNodeId,
    kind: RelationKind,
    evidence: EvidenceRef,
) -> Relation {
    Relation {
        id: format!("{prefix}:{}:{}", source.as_str(), target.as_str()),
        source: source.clone(),
        target: target.clone(),
        kind,
        confidence: Confidence::Low,
        evidence: vec![evidence],
        provenance: Some(RelationProvenance {
            language: None,
            resolver_strategy: "deterministic-enrichment-v1".to_owned(),
            resolution: RelationResolution::HybridResolved,
            confidence: Confidence::Low,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{ENRICHMENT_ALGORITHM_VERSION, derive_enrichment};
    use crate::domain::{ArtifactCategory, ArtifactId, EvidenceRef, RepoPath};
    use crate::graph::{
        ArtifactNode, DocumentationNode, Graph, GraphNode, GraphNodeId, RelationKind,
    };

    fn evidence(path: &str) -> EvidenceRef {
        let path = RepoPath::new(path).unwrap_or_else(|_| unreachable!());
        EvidenceRef::file(ArtifactId::from_path(&path), path)
    }
    fn artifact(path: &str) -> GraphNode {
        GraphNode::Artifact(ArtifactNode {
            id: GraphNodeId::new(format!("artifact:{path}")),
            path: path.to_owned(),
            category: ArtifactCategory::SourceCode,
            evidence: evidence(path),
        })
    }

    #[test]
    fn optional_overlay_is_deterministic_and_does_not_mutate_the_graph() {
        let graph = Graph {
            nodes: vec![
                artifact("src/widget.py"),
                artifact("tests/test_widget.py"),
                GraphNode::Documentation(DocumentationNode {
                    id: GraphNodeId::new("doc:src/widget.py#1"),
                    title: "Widget".to_owned(),
                    evidence: evidence("src/widget.py"),
                }),
            ],
            relations: Vec::new(),
        };
        let baseline = graph.clone();
        assert!(derive_enrichment(&graph, false).relations.is_empty());
        let first = derive_enrichment(&graph, true);
        let second = derive_enrichment(&graph, true);
        assert_eq!(first, second);
        assert_eq!(first.algorithm_version, ENRICHMENT_ALGORITHM_VERSION);
        assert!(
            first
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Tests)
        );
        assert!(
            first
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::DocumentsSource)
        );
        assert_eq!(graph, baseline);
    }
}
