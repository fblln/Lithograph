//! Dead-code heuristic: symbol nodes with no inbound relation.

use super::KnowledgeIndex;
use super::common::search_result;
use super::search::SearchResult;
use crate::graph::{GraphNode, GraphNodeId, RelationKind};
use std::collections::BTreeSet;

impl<'a> KnowledgeIndex<'a> {
    /// Symbol nodes with no inbound relation anywhere in the graph -- never
    /// called, implemented, referenced, or used. A heuristic (it can't see
    /// true entry points like a `main` function or reflection-based
    /// dynamic dispatch), not a certainty; callers should treat the result
    /// as candidates to review, not a definite deletion list.
    pub fn find_dead_code(&self) -> Vec<SearchResult> {
        // `Contains` (an artifact/class defining this symbol) is structural,
        // not a use -- every symbol has exactly one, so counting it would
        // make every symbol look "referenced" and this method useless.
        let mut referenced: BTreeSet<&GraphNodeId> = BTreeSet::new();
        for relation in &self.graph.relations {
            if relation.kind != RelationKind::Contains {
                referenced.insert(&relation.target);
            }
        }
        let degree = self.degree_index();
        let mut dead: Vec<SearchResult> = self
            .graph
            .nodes
            .iter()
            .filter(|node| matches!(node, GraphNode::Symbol(_)) && !referenced.contains(node.id()))
            .map(|node| search_result(node, &degree))
            .collect();
        dead.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        dead
    }
}
