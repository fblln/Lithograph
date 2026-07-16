//! Single-node explanation: what a node is, what proves it, and what it
//! connects to.
//!
//! Answers "what is this thing" against a built graph (LIT-47). It lives
//! beside the other index queries rather than in the CLI so the MCP server
//! and the explorer can serve the same answer.

use super::KnowledgeIndex;
use super::common::search_result;
use super::search::SearchResult;
use crate::domain::EvidenceRef;
use crate::graph::{GraphNode, RelationKind, RelationResolution};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// One node, its evidence, and its neighbors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeExplanation {
    /// The explained node.
    pub node: SearchResult,
    /// Source spans backing the node's existence.
    pub evidence: Vec<EvidenceRef>,
    /// Relations pointing away from the node, grouped by kind.
    pub outbound: BTreeMap<String, Vec<Neighbor>>,
    /// Relations pointing at the node, grouped by kind.
    pub inbound: BTreeMap<String, Vec<Neighbor>>,
}

/// One neighbor reached by a relation of a given kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Neighbor {
    /// The neighbor node.
    pub node: SearchResult,
    /// How the connecting relation was resolved, when it records
    /// provenance: the difference between a proven edge and a guess.
    pub resolution: Option<RelationResolution>,
}

impl<'a> KnowledgeIndex<'a> {
    /// Explains the first node matching `query`, or `None` when nothing
    /// matches.
    ///
    /// Neighbors are grouped by relation kind because the nodes worth asking
    /// about are the well-connected ones: an ungrouped dump of a hub's edges
    /// is the question restated, not an answer.
    pub fn explain(&self, query: &str) -> Option<NodeExplanation> {
        let node = self.find_root(query)?;
        let degree = self.degree_index();
        let mut outbound: BTreeMap<String, Vec<Neighbor>> = BTreeMap::new();
        let mut inbound: BTreeMap<String, Vec<Neighbor>> = BTreeMap::new();

        for relation in &self.graph.relations {
            let (group, counterpart) = if &relation.source == node.id() {
                (&mut outbound, &relation.target)
            } else if &relation.target == node.id() {
                (&mut inbound, &relation.source)
            } else {
                continue;
            };
            let Some(other) = self
                .graph
                .nodes
                .iter()
                .find(|node| node.id() == counterpart)
            else {
                continue;
            };
            group
                .entry(relation_kind_name(relation.kind))
                .or_default()
                .push(Neighbor {
                    node: search_result(other, &degree),
                    resolution: relation
                        .provenance
                        .as_ref()
                        .map(|provenance| provenance.resolution),
                });
        }
        for group in outbound.values_mut().chain(inbound.values_mut()) {
            group.sort_by(|a, b| a.node.id.cmp(&b.node.id));
            group.dedup_by(|a, b| a.node.id == b.node.id);
        }

        Some(NodeExplanation {
            node: search_result(node, &degree),
            evidence: node_evidence(node),
            outbound,
            inbound,
        })
    }
}

fn relation_kind_name(kind: RelationKind) -> String {
    format!("{kind:?}")
}

/// Every source span recorded for a node. Node kinds that are graph-internal
/// groupings rather than source facts (packages, unresolved references) carry
/// none, and report none rather than inventing a location.
fn node_evidence(node: &GraphNode) -> Vec<EvidenceRef> {
    match node {
        GraphNode::Artifact(node) => vec![node.evidence.clone()],
        GraphNode::Symbol(node) => vec![node.evidence.clone()],
        GraphNode::Config(node) => vec![node.evidence.clone()],
        GraphNode::Documentation(node) => vec![node.evidence.clone()],
        GraphNode::Command(node) => vec![node.evidence.clone()],
        GraphNode::Module(node) => vec![node.evidence.clone()],
        GraphNode::Container(_)
        | GraphNode::EnvVar(_)
        | GraphNode::Package(_)
        | GraphNode::Unresolved(_) => Vec::new(),
    }
}
