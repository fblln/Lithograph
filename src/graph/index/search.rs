//! Structured graph search.

use super::KnowledgeIndex;
use super::common::{node_label, node_search_text, search_result};
use crate::graph::GraphNodeId;
use serde::{Deserialize, Serialize};

/// Structured graph search parameters.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SearchParams {
    /// Optional node label filter, e.g. `Symbol`, `Artifact`, or `Package`.
    pub label: Option<String>,
    /// Optional case-insensitive substring matched against node names, ids, and paths.
    pub query: Option<String>,
    /// Maximum result count. Defaults to 10 when zero.
    pub limit: usize,
}

/// One graph search result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResult {
    /// Graph node id.
    pub id: GraphNodeId,
    /// Node label.
    pub label: String,
    /// Human-readable name.
    pub name: String,
    /// Repository-relative file path when the node has one.
    pub file_path: Option<String>,
    /// Inbound relation count.
    pub in_degree: usize,
    /// Outbound relation count.
    pub out_degree: usize,
}

impl<'a> KnowledgeIndex<'a> {
    /// Searches nodes by label and substring query.
    pub fn search(&self, params: &SearchParams) -> Vec<SearchResult> {
        let degree = self.degree_index();
        let query = params.query.as_ref().map(|query| query.to_lowercase());
        let label = params.label.as_ref().map(|label| label.to_lowercase());
        let limit = default_limit(params.limit);

        let mut results: Vec<SearchResult> = self
            .graph
            .nodes
            .iter()
            .filter(|node| {
                label
                    .as_ref()
                    .is_none_or(|wanted| node_label(node).to_lowercase() == *wanted)
            })
            .filter(|node| {
                query
                    .as_ref()
                    .is_none_or(|wanted| node_search_text(node).contains(wanted))
            })
            .map(|node| search_result(node, &degree))
            .collect();
        results.sort_by(|a, b| {
            (b.in_degree + b.out_degree)
                .cmp(&(a.in_degree + a.out_degree))
                .then(a.label.cmp(&b.label))
                .then(a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        results.truncate(limit);
        results
    }
}

fn default_limit(limit: usize) -> usize {
    if limit == 0 { 10 } else { limit.min(100) }
}
