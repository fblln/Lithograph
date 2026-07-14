//! Deterministic graph schema summary.

use super::KnowledgeIndex;
use super::common::node_label;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Deterministic graph schema summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSchema {
    /// Counts by graph node label.
    pub node_labels: Vec<LabelCount>,
    /// Counts by relation type.
    pub edge_types: Vec<TypeCount>,
    /// Observed source/edge/target patterns.
    pub relationship_patterns: Vec<String>,
}

/// Count for one node label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelCount {
    /// Node label.
    pub label: String,
    /// Number of nodes with this label.
    pub count: usize,
}

/// Count for one edge type.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeCount {
    /// Relation type.
    pub edge_type: String,
    /// Number of relations with this type.
    pub count: usize,
}

impl<'a> KnowledgeIndex<'a> {
    /// Returns deterministic graph schema counts.
    pub fn schema(&self) -> GraphSchema {
        let node_by_id = self.node_by_id();
        let mut node_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut edge_counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut patterns: BTreeMap<String, usize> = BTreeMap::new();

        for node in &self.graph.nodes {
            *node_counts.entry(node_label(node).to_owned()).or_default() += 1;
        }
        for relation in &self.graph.relations {
            let edge = format!("{:?}", relation.kind);
            *edge_counts.entry(edge.clone()).or_default() += 1;
            let source = node_by_id
                .get(&relation.source)
                .map_or("Unknown", |node| node_label(node));
            let target = node_by_id
                .get(&relation.target)
                .map_or("Unknown", |node| node_label(node));
            *patterns
                .entry(format!("({source})-[{edge}]->({target})"))
                .or_default() += 1;
        }

        GraphSchema {
            node_labels: node_counts
                .into_iter()
                .map(|(label, count)| LabelCount { label, count })
                .collect(),
            edge_types: edge_counts
                .into_iter()
                .map(|(edge_type, count)| TypeCount { edge_type, count })
                .collect(),
            relationship_patterns: patterns
                .into_iter()
                .map(|(pattern, count)| format!("{pattern} [{count}x]"))
                .collect(),
        }
    }
}
