//! Deterministic architecture and operations Markdown derived from graph facts.

use crate::docs_model::{DocumentSectionKind, GraphDocument};
use crate::graph::{Graph, RepositoryTension};

/// Generates an evidence-linked architecture/operations document with safe no-data fallbacks.
pub fn generate_graph_docs(
    graph: &Graph,
    tensions: &[RepositoryTension],
    snapshot_id: &str,
) -> (GraphDocument, String) {
    let mut document = GraphDocument::new("architecture-ops", snapshot_id, 1);
    let overview = document.add_section(
        DocumentSectionKind::SystemOverview,
        "System Overview",
        vec!["graph:overview".into()],
        vec![],
        graph
            .nodes
            .iter()
            .map(|node| node.id().clone())
            .take(10)
            .collect(),
        vec![],
        crate::domain::Confidence::High,
    );
    let architecture = document.add_section(
        DocumentSectionKind::C4Context,
        "Architecture",
        vec!["graph:architecture".into()],
        vec![],
        vec![],
        vec![],
        crate::domain::Confidence::High,
    );
    let workflow = document.add_section(
        DocumentSectionKind::Workflow,
        "Workflows",
        vec!["graph:workflows".into()],
        vec![],
        vec![],
        vec![],
        crate::domain::Confidence::Low,
    );
    let boundaries = document.add_section(
        DocumentSectionKind::BoundaryInterface,
        "Boundary Interfaces",
        vec!["graph:boundaries".into()],
        vec![],
        vec![],
        vec![],
        crate::domain::Confidence::Low,
    );
    let data = document.add_section(
        DocumentSectionKind::DataStore,
        "Data / Database Overview",
        vec!["graph:data".into()],
        vec![],
        vec![],
        vec![],
        crate::domain::Confidence::Low,
    );
    let ops = document.add_section(
        DocumentSectionKind::OperationalRunbook,
        "Operations Runbook",
        vec!["graph:operations".into()],
        vec![],
        vec![],
        vec![],
        crate::domain::Confidence::High,
    );
    let risks = document.add_section(
        DocumentSectionKind::Risk,
        "Risk / Tension Summary",
        vec!["graph:tensions".into()],
        tensions
            .iter()
            .flat_map(|t| t.evidence_references.clone())
            .collect(),
        tensions
            .iter()
            .flat_map(|t| t.affected_nodes.clone())
            .collect(),
        tensions
            .iter()
            .flat_map(|t| t.affected_edges.clone())
            .collect(),
        crate::domain::Confidence::High,
    );
    let drift = document.add_section(
        DocumentSectionKind::Drift,
        "Drift",
        vec!["graph:drift".into()],
        vec![],
        vec![],
        vec![],
        crate::domain::Confidence::Low,
    );
    let lines = [overview, architecture, workflow, boundaries, data, ops, risks, drift].into_iter().filter_map(|id| document.section_index().get(&id).copied()).map(|section| format!("## {}\n\n<!-- graph-section:{} {} -->\n\n{}\n", section.title, section.id, section.deep_link_target, if section.kind == DocumentSectionKind::OperationalRunbook { "Run `lithograph init`, `lithograph update`, and `make check-all`. Indexing is local/offline; inspect configuration, dependencies, failures, and troubleshooting through graph evidence." } else if section.kind == DocumentSectionKind::Risk && tensions.is_empty() { "No graph-backed tensions were detected." } else { "Graph facts are available through the linked focused view." })).collect::<Vec<_>>();
    let mermaid = "```mermaid\nflowchart LR\n  graph[\"Graph Snapshot\"] --> docs[\"Architecture Docs\"]\n```\n";
    (
        document,
        format!(
            "# Architecture and Operations\n\n{mermaid}\n{}",
            lines.join("\n")
        ),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    #[test]
    fn generated_docs_are_stable_and_cover_no_data_fallbacks() {
        let (doc, markdown) = generate_graph_docs(
            &Graph {
                nodes: vec![],
                relations: vec![],
            },
            &[],
            "g1",
        );
        assert!(markdown.contains("No graph-backed tensions"));
        assert!(markdown.contains("```mermaid"));
        assert_eq!(doc.sections.len(), 8);
        assert_eq!(
            generate_graph_docs(
                &Graph {
                    nodes: vec![],
                    relations: vec![]
                },
                &[],
                "g1"
            )
            .1,
            markdown
        );
    }
}
