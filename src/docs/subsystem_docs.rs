//! Typed, graph-only subsystem documentation agent substrate for MCP.

use crate::domain::Confidence;
use crate::graph::{Graph, GraphNodeId, GraphTag, RepositoryTension, TagSource};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};

/// Deterministic response used by MCP generation, refinement, and validation tools.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub(crate) struct SubsystemDocument {
    pub subsystem_id: String,
    pub graph_snapshot_id: String,
    pub prompt_version: String,
    pub confidence: Confidence,
    pub cited_nodes: Vec<GraphNodeId>,
    pub cited_edges: Vec<String>,
    pub source_spans: Vec<String>,
    pub unresolved_assumptions: Vec<String>,
    pub markdown: String,
    pub resolved_tags: Vec<GraphTag>,
    pub tag_expression: Option<String>,
}
/// Snapshot-bound persistence for generated and refined subsystem documents.
#[derive(Debug, Clone)]
pub(crate) struct SubsystemDocumentStore {
    root: std::path::PathBuf,
}
impl SubsystemDocumentStore {
    /// Creates a store rooted at `.lithograph/subsystem-docs`.
    pub(crate) fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
    /// Writes only changed document payloads.
    pub(crate) fn save(&self, document: &SubsystemDocument) -> std::io::Result<bool> {
        let payload = serde_json::to_string(document).map_err(std::io::Error::other)?;
        let path = self.path(document);
        if JsonStore.read::<String>(&path)?.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }
    /// Loads a saved document only when it matches the requested graph snapshot.
    #[cfg(test)]
    pub(crate) fn load_current(
        &self,
        subsystem: &str,
        graph_snapshot_id: &str,
    ) -> std::io::Result<Option<SubsystemDocument>> {
        let path = self.root.join(format!(
            "{}.json",
            blake3::hash(subsystem.as_bytes()).to_hex()
        ));
        let Some(payload): Option<String> = JsonStore.read(&path)? else {
            return Ok(None);
        };
        let document: SubsystemDocument =
            serde_json::from_str(&payload).map_err(std::io::Error::other)?;
        Ok((document.graph_snapshot_id == graph_snapshot_id).then_some(document))
    }
    fn path(&self, document: &SubsystemDocument) -> std::path::PathBuf {
        self.root.join(format!(
            "{}.json",
            blake3::hash(document.subsystem_id.as_bytes()).to_hex()
        ))
    }
}
/// Lists documentable graph subsystems from stable node-id prefixes.
pub(crate) fn list_subsystems(graph: &Graph) -> Vec<String> {
    let mut values: Vec<_> = graph
        .nodes
        .iter()
        .map(|node| {
            node.id()
                .as_str()
                .split(':')
                .next()
                .unwrap_or("graph")
                .to_owned()
        })
        .collect();
    values.sort();
    values.dedup();
    values
}
/// Generates deterministic, evidence-limited subsystem documentation from graph facts only.
pub(crate) fn generate_subsystem_doc(
    graph: &Graph,
    subsystem: &str,
    snapshot: &str,
    tensions: &[RepositoryTension],
    instruction: Option<&str>,
) -> SubsystemDocument {
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| node.id().as_str().starts_with(subsystem))
        .map(|node| node.id().clone())
        .collect();
    generate_subsystem_doc_for_nodes(graph, subsystem, snapshot, tensions, instruction, &nodes)
}

/// Generates a document constrained to a caller-resolved tag scope.
pub(crate) fn generate_subsystem_doc_for_nodes(
    graph: &Graph,
    subsystem: &str,
    snapshot: &str,
    tensions: &[RepositoryTension],
    instruction: Option<&str>,
    selected_nodes: &[GraphNodeId],
) -> SubsystemDocument {
    let nodes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| selected_nodes.contains(node.id()))
        .map(|node| node.id().clone())
        .collect();
    let edges: Vec<_> = graph
        .relations
        .iter()
        .filter(|edge| nodes.contains(&edge.source) || nodes.contains(&edge.target))
        .map(|edge| edge.id.clone())
        .collect();
    let risks = tensions
        .iter()
        .filter(|t| t.affected_nodes.iter().any(|node| nodes.contains(node)))
        .count();
    let suffix = instruction
        .map(|value| format!("\n\nRefinement: {value}"))
        .unwrap_or_default();
    let resolved_tags = if nodes.is_empty() {
        Vec::new()
    } else {
        vec![GraphTag::new(
            subsystem,
            "subsystem",
            subsystem,
            TagSource::Architecture,
            snapshot,
        )]
    };
    SubsystemDocument {
        subsystem_id: subsystem.into(),
        graph_snapshot_id: snapshot.into(),
        prompt_version: "subsystem-doc-v1".into(),
        confidence: Confidence::High,
        cited_nodes: nodes.clone(),
        cited_edges: edges.clone(),
        source_spans: vec![],
        unresolved_assumptions: if nodes.is_empty() {
            vec!["No matching graph nodes.".into()]
        } else {
            vec![]
        },
        markdown: format!(
            "# {subsystem} subsystem\n\n## Architecture summary\n{} graph nodes and {} relations.\n\n## Responsibilities, boundaries, dependencies, workflows, operations, risks, and open questions\nGraph-backed risk count: {risks}.{suffix}",
            nodes.len(),
            edges.len()
        ),
        resolved_tags,
        tag_expression: None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{Graph, GraphNode, UnresolvedNode};
    #[test]
    fn deterministic_graph_only_doc_handles_missing_subsystem() {
        let graph = Graph {
            nodes: vec![],
            relations: vec![],
        };
        let doc = generate_subsystem_doc(&graph, "symbol", "g1", &[], None);
        assert!(!doc.unresolved_assumptions.is_empty());
        assert_eq!(
            doc,
            generate_subsystem_doc(&graph, "symbol", "g1", &[], None)
        );
    }
    #[test]
    fn store_round_trips_refinement_and_rejects_stale_snapshots()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = Graph {
            nodes: vec![],
            relations: vec![],
        };
        let document = generate_subsystem_doc(&graph, "symbol", "g1", &[], Some("add operations"));
        let temp = tempfile::TempDir::new()?;
        let store = SubsystemDocumentStore::new(temp.path());
        assert!(store.save(&document)?);
        assert!(!store.save(&document)?);
        assert_eq!(store.load_current("symbol", "g1")?, Some(document));
        assert_eq!(store.load_current("symbol", "g2")?, None);
        Ok(())
    }
    #[test]
    fn tagged_scope_emits_a_searchable_subsystem_tag() {
        let node = GraphNode::Unresolved(UnresolvedNode {
            id: GraphNodeId::new("symbol:payment"),
            value: "payment".into(),
        });
        let graph = Graph {
            nodes: vec![node],
            relations: vec![],
        };
        let document = generate_subsystem_doc(&graph, "symbol", "g1", &[], None);
        assert_eq!(document.resolved_tags[0].value, "symbol");
        assert!(document.tag_expression.is_none());
    }
}
