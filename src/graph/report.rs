//! Deterministic, offline graph report derivation and Markdown rendering.

use crate::domain::Confidence;
use crate::graph::analytics::{BetweennessPolicy, betweenness, degree_metrics};
use crate::graph::index::node_name;
use crate::graph::{Graph, GraphNode, GraphNodeId, KnowledgeIndex, RelationKind};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};

const LEADER_LIMIT: usize = 10;
const GAP_LIMIT: usize = 20;
const CYCLE_LIMIT: usize = 10;
const QUESTION_LIMIT: usize = 10;

/// High-level graph and resolution counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphReportSummary {
    /// Total graph nodes.
    pub node_count: usize,
    /// Total graph relations.
    pub relation_count: usize,
    /// Relations whose target is not unresolved.
    pub resolved_relation_count: usize,
    /// Relations whose target is unresolved.
    pub unresolved_relation_count: usize,
    /// Low-confidence relations.
    pub low_confidence_relation_count: usize,
}

/// Ranked node metric rendered in the report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphReportNodeRank {
    /// Stable graph node id.
    pub id: GraphNodeId,
    /// Human-readable node name.
    pub name: String,
    /// Inbound relation count.
    pub in_degree: usize,
    /// Outbound relation count.
    pub out_degree: usize,
}

/// Cross-cluster node ranked by deterministic betweenness.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphReportBridge {
    /// Stable graph node id.
    pub id: GraphNodeId,
    /// Human-readable node name.
    pub name: String,
    /// Betweenness scaled by 1,000,000 and rounded to an integer.
    pub score_millionths: u64,
}

/// Unresolved node and the number of relations that point to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphReportUnresolvedGap {
    /// Stable unresolved node id.
    pub id: GraphNodeId,
    /// Unresolved literal value.
    pub value: String,
    /// Relations targeting this node.
    pub inbound_relations: usize,
}

/// One bounded low-confidence relation requiring audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphReportLowConfidenceGap {
    /// Stable relation id.
    pub relation_id: String,
    /// Source node id.
    pub source: GraphNodeId,
    /// Target node id.
    pub target: GraphNodeId,
    /// Relation kind.
    pub kind: RelationKind,
}

/// Canonical graph digest derived only from a persisted graph snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GraphReport {
    /// Summary and resolution counts.
    pub summary: GraphReportSummary,
    /// Highest total-degree nodes.
    pub degree_leaders: Vec<GraphReportNodeRank>,
    /// Cross-cluster nodes with positive betweenness.
    pub betweenness_bridges: Vec<GraphReportBridge>,
    /// Bounded module dependency cycles.
    pub cycles: Vec<Vec<GraphNodeId>>,
    /// Nodes with no incident relations.
    pub isolated_nodes: Vec<GraphReportNodeRank>,
    /// Unresolved references ranked by inbound relation count.
    pub unresolved_gaps: Vec<GraphReportUnresolvedGap>,
    /// Low-confidence relations in stable order.
    pub low_confidence_gaps: Vec<GraphReportLowConfidenceGap>,
    /// Metric-derived next-step questions.
    pub suggested_questions: Vec<String>,
}

impl GraphReport {
    /// Derives a bounded report without clocks, randomness, network, or models.
    pub(crate) fn build(graph: &Graph) -> Self {
        let node_by_id = graph
            .nodes
            .iter()
            .map(|node| (node.id().clone(), node))
            .collect::<BTreeMap<_, _>>();
        let unresolved_ids = graph
            .nodes
            .iter()
            .filter(|node| matches!(node, GraphNode::Unresolved(_)))
            .map(|node| node.id().clone())
            .collect::<BTreeSet<_>>();
        let unresolved_relation_count = graph
            .relations
            .iter()
            .filter(|relation| unresolved_ids.contains(&relation.target))
            .count();
        let low_confidence_relation_count = graph
            .relations
            .iter()
            .filter(|relation| relation.confidence == Confidence::Low)
            .count();
        let summary = GraphReportSummary {
            node_count: graph.nodes.len(),
            relation_count: graph.relations.len(),
            resolved_relation_count: graph
                .relations
                .len()
                .saturating_sub(unresolved_relation_count),
            unresolved_relation_count,
            low_confidence_relation_count,
        };

        let mut degrees = degree_metrics(graph);
        degrees.sort_by(|left, right| {
            (right.1 + right.2)
                .cmp(&(left.1 + left.2))
                .then_with(|| left.0.cmp(&right.0))
        });
        let degree_rank =
            |(id, incoming, outgoing): &(GraphNodeId, usize, usize)| GraphReportNodeRank {
                id: id.clone(),
                name: node_by_id
                    .get(id)
                    .map(|node| node_name(node))
                    .unwrap_or_else(|| id.as_str().to_owned()),
                in_degree: *incoming,
                out_degree: *outgoing,
            };
        let degree_leaders = degrees
            .iter()
            .filter(|(_, incoming, outgoing)| incoming + outgoing > 0)
            .take(LEADER_LIMIT)
            .map(degree_rank)
            .collect::<Vec<_>>();
        let isolated_nodes = degrees
            .iter()
            .filter(|(_, incoming, outgoing)| incoming + outgoing == 0)
            .take(GAP_LIMIT)
            .map(degree_rank)
            .collect::<Vec<_>>();

        let index = KnowledgeIndex::new(graph);
        let clusters = index.clusters();
        let cluster_by_node = clusters
            .iter()
            .flat_map(|cluster| {
                cluster
                    .members
                    .iter()
                    .map(move |member| (member.clone(), cluster.id.as_str()))
            })
            .collect::<BTreeMap<_, _>>();
        let mut boundary_nodes = BTreeSet::new();
        for relation in &graph.relations {
            let (Some(source_cluster), Some(target_cluster)) = (
                cluster_by_node.get(&relation.source),
                cluster_by_node.get(&relation.target),
            ) else {
                continue;
            };
            if source_cluster != target_cluster {
                boundary_nodes.insert(relation.source.clone());
                boundary_nodes.insert(relation.target.clone());
            }
        }
        let mut bridge_scores = betweenness(graph, BetweennessPolicy::default());
        bridge_scores.retain(|(id, score)| boundary_nodes.contains(id) && *score > 0.0);
        bridge_scores.sort_by(|left, right| {
            right
                .1
                .total_cmp(&left.1)
                .then_with(|| left.0.cmp(&right.0))
        });
        let betweenness_bridges = bridge_scores
            .into_iter()
            .take(LEADER_LIMIT)
            .map(|(id, score)| GraphReportBridge {
                name: node_by_id
                    .get(&id)
                    .map(|node| node_name(node))
                    .unwrap_or_else(|| id.as_str().to_owned()),
                id,
                score_millionths: (score * 1_000_000.0).round().max(0.0) as u64,
            })
            .collect::<Vec<_>>();

        let cycles = index
            .dependency_matrix()
            .cycles
            .into_iter()
            .take(CYCLE_LIMIT)
            .collect::<Vec<_>>();
        let inbound = graph.relations.iter().fold(
            BTreeMap::<GraphNodeId, usize>::new(),
            |mut counts, relation| {
                *counts.entry(relation.target.clone()).or_default() += 1;
                counts
            },
        );
        let mut unresolved_gaps = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Unresolved(unresolved) => Some(GraphReportUnresolvedGap {
                    id: unresolved.id.clone(),
                    value: unresolved.value.clone(),
                    inbound_relations: inbound.get(&unresolved.id).copied().unwrap_or_default(),
                }),
                _ => None,
            })
            .collect::<Vec<_>>();
        unresolved_gaps.sort_by(|left, right| {
            right
                .inbound_relations
                .cmp(&left.inbound_relations)
                .then_with(|| left.id.cmp(&right.id))
        });
        unresolved_gaps.truncate(GAP_LIMIT);

        let mut low_confidence_gaps = graph
            .relations
            .iter()
            .filter(|relation| relation.confidence == Confidence::Low)
            .map(|relation| GraphReportLowConfidenceGap {
                relation_id: relation.id.clone(),
                source: relation.source.clone(),
                target: relation.target.clone(),
                kind: relation.kind,
            })
            .collect::<Vec<_>>();
        low_confidence_gaps.sort_by(|left, right| {
            left.source
                .cmp(&right.source)
                .then_with(|| left.kind.cmp(&right.kind))
                .then_with(|| left.target.cmp(&right.target))
                .then_with(|| left.relation_id.cmp(&right.relation_id))
        });
        low_confidence_gaps.truncate(GAP_LIMIT);

        let suggested_questions = suggested_questions(
            &degree_leaders,
            &betweenness_bridges,
            &unresolved_gaps,
            summary.low_confidence_relation_count,
        );
        Self {
            summary,
            degree_leaders,
            betweenness_bridges,
            cycles,
            isolated_nodes,
            unresolved_gaps,
            low_confidence_gaps,
            suggested_questions,
        }
    }

    /// Renders canonical Markdown with stable ordering and bounded sections.
    pub(crate) fn render_markdown(&self) -> String {
        let mut output = String::from("# Lithograph Graph Report\n\n");
        output.push_str("## Summary and resolution\n\n");
        output.push_str(&format!(
            "- Nodes: {}\n- Relations: {}\n- Resolved relations: {} of {} ({}%)\n- Unresolved relations: {}\n- Low-confidence relations: {}\n\n",
            self.summary.node_count,
            self.summary.relation_count,
            self.summary.resolved_relation_count,
            self.summary.relation_count,
            resolution_percent(&self.summary),
            self.summary.unresolved_relation_count,
            self.summary.low_confidence_relation_count,
        ));
        render_node_ranks(&mut output, "God nodes by degree", &self.degree_leaders);
        output.push_str("## Cross-cluster bridges by betweenness\n\n");
        if self.betweenness_bridges.is_empty() {
            output.push_str("No positive-betweenness cross-cluster bridges detected.\n\n");
        } else {
            for bridge in &self.betweenness_bridges {
                output.push_str(&format!(
                    "- `{}` — {} (score {})\n",
                    inline(bridge.id.as_str()),
                    inline(&bridge.name),
                    bridge.score_millionths
                ));
            }
            output.push('\n');
        }
        output.push_str("## Import and dependency cycles\n\n");
        if self.cycles.is_empty() {
            output.push_str("No module dependency cycles detected.\n\n");
        } else {
            for cycle in &self.cycles {
                let mut ids = cycle
                    .iter()
                    .map(|id| inline(id.as_str()))
                    .collect::<Vec<_>>();
                if let Some(first) = ids.first().cloned() {
                    ids.push(first);
                }
                output.push_str(&format!("- `{}`\n", ids.join("` → `")));
            }
            output.push('\n');
        }
        output.push_str("## Knowledge gaps\n\n### Isolated nodes\n\n");
        render_rank_items(
            &mut output,
            &self.isolated_nodes,
            "No isolated nodes detected.",
        );
        output.push_str("### Unresolved hotspots\n\n");
        if self.unresolved_gaps.is_empty() {
            output.push_str("No unresolved nodes detected.\n\n");
        } else {
            for gap in &self.unresolved_gaps {
                output.push_str(&format!(
                    "- `{}` — {} ({} inbound relations)\n",
                    inline(gap.id.as_str()),
                    inline(&gap.value),
                    gap.inbound_relations
                ));
            }
            output.push('\n');
        }
        output.push_str("### Low-confidence relations to audit\n\n");
        if self.low_confidence_gaps.is_empty() {
            output.push_str("No low-confidence relations detected.\n\n");
        } else {
            for gap in &self.low_confidence_gaps {
                output.push_str(&format!(
                    "- `{}` → `{}` ({:?}, relation `{}`)\n",
                    inline(gap.source.as_str()),
                    inline(gap.target.as_str()),
                    gap.kind,
                    inline(&gap.relation_id),
                ));
            }
            output.push('\n');
        }
        output.push_str("## Suggested audit questions\n\n");
        for question in &self.suggested_questions {
            output.push_str(&format!("- {}\n", question));
        }
        output.push('\n');
        output
    }
}

/// Canonical path for the persisted report.
pub(crate) fn graph_report_path(repo_root: &Path) -> PathBuf {
    repo_root.join(".lithograph/GRAPH_REPORT.md")
}

/// Writes a newly derived report only when its canonical bytes changed.
pub(crate) fn persist_graph_report(repo_root: &Path, graph: &Graph) -> io::Result<bool> {
    let path = graph_report_path(repo_root);
    let markdown = GraphReport::build(graph).render_markdown();
    if std::fs::read_to_string(&path).ok().as_deref() == Some(markdown.as_str()) {
        return Ok(false);
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, markdown)?;
    Ok(true)
}

fn resolution_percent(summary: &GraphReportSummary) -> String {
    if summary.relation_count == 0 {
        return "100.0".to_owned();
    }
    let tenths = summary.resolved_relation_count * 1_000 / summary.relation_count;
    format!("{}.{:01}", tenths / 10, tenths % 10)
}

fn render_node_ranks(output: &mut String, title: &str, ranks: &[GraphReportNodeRank]) {
    output.push_str(&format!("## {title}\n\n"));
    render_rank_items(output, ranks, "No connected nodes detected.");
}

fn render_rank_items(output: &mut String, ranks: &[GraphReportNodeRank], empty: &str) {
    if ranks.is_empty() {
        output.push_str(empty);
        output.push_str("\n\n");
        return;
    }
    for rank in ranks {
        output.push_str(&format!(
            "- `{}` — {} (in {}, out {}, total {})\n",
            inline(rank.id.as_str()),
            inline(&rank.name),
            rank.in_degree,
            rank.out_degree,
            rank.in_degree + rank.out_degree,
        ));
    }
    output.push('\n');
}

fn suggested_questions(
    degree_leaders: &[GraphReportNodeRank],
    bridges: &[GraphReportBridge],
    unresolved: &[GraphReportUnresolvedGap],
    low_confidence_count: usize,
) -> Vec<String> {
    let mut questions = Vec::new();
    for leader in degree_leaders.iter().take(3) {
        questions.push(format!(
            "What responsibilities make `{}` highly connected, and should any be separated?",
            inline(leader.id.as_str())
        ));
    }
    for bridge in bridges.iter().take(3) {
        questions.push(format!(
            "What contract does `{}` enforce across functional clusters?",
            inline(bridge.id.as_str())
        ));
    }
    for gap in unresolved.iter().take(3) {
        questions.push(format!(
            "What repository evidence can resolve `{}` ({})?",
            inline(gap.id.as_str()),
            inline(&gap.value)
        ));
    }
    questions.push(format!(
        "Which of the graph's {low_confidence_count} low-confidence relations are justified by source evidence, and which should be corrected?"
    ));
    questions.truncate(QUESTION_LIMIT);
    questions
}

fn inline(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('`', "'")
}

#[cfg(test)]
mod tests {
    use super::GraphReport;
    use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath};
    use crate::graph::{
        Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, Relation, RelationKind,
        UnresolvedNode,
    };

    fn module(id: &str) -> Result<GraphNode, Box<dyn std::error::Error>> {
        let path = RepoPath::new(format!("src/{id}.rs"))?;
        Ok(GraphNode::Module(ModuleNode {
            id: GraphNodeId::new(format!("module:{id}")),
            path: id.to_owned(),
            language: ModuleLanguage::Rust,
            evidence: EvidenceRef::file(ArtifactId::from_path(&path), path),
        }))
    }

    fn relation(
        id: &str,
        source: &str,
        target: &str,
        kind: RelationKind,
        confidence: Confidence,
    ) -> Relation {
        Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence,
            evidence: vec![],
            provenance: None,
        }
    }

    #[test]
    fn report_is_stable_bounded_and_surfaces_cycles_and_gaps()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = Graph {
            nodes: vec![
                module("a")?,
                module("b")?,
                module("c")?,
                module("d")?,
                module("isolated")?,
                GraphNode::Unresolved(UnresolvedNode {
                    id: GraphNodeId::new("unresolved:missing"),
                    value: "missing.module".to_owned(),
                }),
            ],
            relations: vec![
                relation(
                    "1",
                    "module:a",
                    "module:b",
                    RelationKind::Calls,
                    Confidence::High,
                ),
                relation(
                    "2",
                    "module:b",
                    "module:a",
                    RelationKind::Imports,
                    Confidence::High,
                ),
                relation(
                    "3",
                    "module:c",
                    "module:d",
                    RelationKind::Calls,
                    Confidence::High,
                ),
                relation(
                    "4",
                    "module:b",
                    "module:c",
                    RelationKind::References,
                    Confidence::High,
                ),
                relation(
                    "5",
                    "module:a",
                    "unresolved:missing",
                    RelationKind::Imports,
                    Confidence::Low,
                ),
            ],
        };

        let report = GraphReport::build(&graph);
        let markdown = report.render_markdown();

        assert_eq!(markdown, GraphReport::build(&graph).render_markdown());
        assert_eq!(report.summary.unresolved_relation_count, 1);
        assert_eq!(report.summary.low_confidence_relation_count, 1);
        assert!(!report.betweenness_bridges.is_empty());
        assert_eq!(
            report.cycles,
            vec![vec![
                GraphNodeId::new("module:a"),
                GraphNodeId::new("module:b")
            ]]
        );
        assert_eq!(report.isolated_nodes[0].id.as_str(), "module:isolated");
        assert!(markdown.contains("## Cross-cluster bridges by betweenness"));
        assert!(markdown.contains("## Knowledge gaps"));
        assert!(markdown.contains("unresolved:missing"));
        assert!(markdown.contains("low-confidence relations are justified"));
        assert!(!markdown.contains("/Users/"));
        Ok(())
    }
}
