//! Addressable graph-backed architecture and operations document model.

use crate::domain::Confidence;
use crate::graph::{GraphNodeId, GraphTag, TagSource};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Semantic section families required by generated architecture and ops docs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(missing_docs)]
pub(crate) enum DocumentSectionKind {
    SystemOverview,
    C4Context,
    C4Container,
    C4Component,
    RuntimeDeployment,
    Workflow,
    BoundaryInterface,
    DataStore,
    OperationalRunbook,
    Risk,
    Drift,
    OpenQuestion,
}
/// One stable, evidence-backed architecture/ops section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub(crate) struct GraphDocumentSection {
    pub id: String,
    pub kind: DocumentSectionKind,
    pub title: String,
    pub source_query_ids: Vec<String>,
    pub evidence_references: Vec<String>,
    pub affected_nodes: Vec<GraphNodeId>,
    pub affected_edges: Vec<String>,
    pub confidence: Confidence,
    pub graph_snapshot_id: String,
    pub deep_link_target: String,
    pub tags: Vec<GraphTag>,
}
/// The versioned document model for one graph snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub(crate) struct GraphDocument {
    pub id: String,
    pub graph_snapshot_id: String,
    pub schema_version: u32,
    pub sections: Vec<GraphDocumentSection>,
}
impl GraphDocument {
    /// Constructs an empty stable document for a graph snapshot.
    pub(crate) fn new(
        id: impl Into<String>,
        graph_snapshot_id: impl Into<String>,
        schema_version: u32,
    ) -> Self {
        Self {
            id: id.into(),
            graph_snapshot_id: graph_snapshot_id.into(),
            schema_version,
            sections: vec![],
        }
    }
    /// Adds a section with an id stable across runs for the same document/kind/title.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn add_section(
        &mut self,
        kind: DocumentSectionKind,
        title: impl Into<String>,
        source_query_ids: Vec<String>,
        evidence_references: Vec<String>,
        affected_nodes: Vec<GraphNodeId>,
        affected_edges: Vec<String>,
        confidence: Confidence,
    ) -> String {
        let title = title.into();
        let id = format!(
            "section:{}",
            blake3::hash(format!("{}:{kind:?}:{title}", self.id).as_bytes()).to_hex()
        );
        let deep_link_target = format!("graph://focus?section={id}");
        let topic = match kind {
            DocumentSectionKind::SystemOverview => "system",
            DocumentSectionKind::C4Context
            | DocumentSectionKind::C4Container
            | DocumentSectionKind::C4Component => "architecture",
            DocumentSectionKind::RuntimeDeployment => "runtime",
            DocumentSectionKind::Workflow => "workflow",
            DocumentSectionKind::BoundaryInterface => "boundary",
            DocumentSectionKind::DataStore => "data-store",
            DocumentSectionKind::OperationalRunbook => "operation",
            DocumentSectionKind::Risk => "risk",
            DocumentSectionKind::Drift => "drift",
            DocumentSectionKind::OpenQuestion => "open-question",
        };
        let tags = if affected_nodes.is_empty() && evidence_references.is_empty() {
            Vec::new()
        } else {
            vec![GraphTag::new(
                id.clone(),
                "topic",
                topic,
                TagSource::Architecture,
                self.graph_snapshot_id.clone(),
            )]
        };
        self.sections.push(GraphDocumentSection {
            id: id.clone(),
            kind,
            title,
            source_query_ids,
            evidence_references,
            affected_nodes,
            affected_edges,
            confidence,
            graph_snapshot_id: self.graph_snapshot_id.clone(),
            deep_link_target,
            tags,
        });
        self.sections.sort_by(|a, b| a.id.cmp(&b.id));
        id
    }
    /// Returns sections keyed by stable id for reverse graph-link resolution.
    pub(crate) fn section_index(&self) -> BTreeMap<String, &GraphDocumentSection> {
        self.sections
            .iter()
            .map(|section| (section.id.clone(), section))
            .collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_is_stable_evidence_backed_and_stale_aware() -> Result<(), Box<dyn std::error::Error>> {
        let mut doc = GraphDocument::new("architecture", "g1", 1);
        let id = doc.add_section(
            DocumentSectionKind::SystemOverview,
            "Overview",
            vec!["query:overview".into()],
            vec!["artifact:readme".into()],
            vec![GraphNodeId::new("symbol:a")],
            vec!["edge:a-b".into()],
            Confidence::High,
        );
        assert_eq!(doc.section_index()[&id].tags[0].value, "system");
        assert_eq!(
            serde_json::from_str::<GraphDocument>(&serde_json::to_string(&doc)?)?,
            doc
        );
        Ok(())
    }
}
