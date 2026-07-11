//! Typed query facade shared by Ladybug-backed and in-memory graph callers.

use crate::graph::{
    Graph, GraphNodeId, KnowledgeIndex, SearchParams, SearchResult, TraceParams, TraceResult,
};

/// Guard for raw graph queries; disabled by default because raw Cypher is not
/// portable across persisted graph versions and must never be user supplied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RawQueryAccess {
    enabled: bool,
}
impl RawQueryAccess {
    /// Creates an explicitly enabled trusted-only raw-query guard.
    pub fn trusted() -> Self {
        Self { enabled: true }
    }
    /// Returns whether trusted raw query execution is allowed.
    pub fn is_enabled(self) -> bool {
        self.enabled
    }
}

/// Typed focused-neighborhood request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NeighborhoodQuery {
    /// Root node for traversal.
    pub root: GraphNodeId,
    /// Maximum relation hops from the root.
    pub depth: usize,
}

/// Typed graph query facade.
pub struct LadybugQueryApi<'a> {
    graph: &'a Graph,
}
impl<'a> LadybugQueryApi<'a> {
    /// Creates a facade over a graph snapshot projected to Ladybug storage.
    pub fn new(graph: &'a Graph) -> Self {
        Self { graph }
    }
    /// Returns the graph schema.
    pub fn schema(&self) -> crate::graph::GraphSchema {
        KnowledgeIndex::new(self.graph).schema()
    }
    /// Runs typed full-text graph search.
    pub fn search(&self, params: &SearchParams) -> Vec<SearchResult> {
        KnowledgeIndex::new(self.graph).search(params)
    }
    /// Returns a focused neighborhood rooted at a typed node id.
    pub fn neighborhood(&self, query: NeighborhoodQuery) -> Option<TraceResult> {
        KnowledgeIndex::new(self.graph).trace(&TraceParams {
            query: query.root.as_str().to_owned(),
            depth: query.depth,
            direction: crate::graph::TraceDirection::Both,
        })
    }
    /// Rejects raw queries unless a trusted caller has explicitly enabled them.
    pub fn raw_query(&self, access: RawQueryAccess, _query: &str) -> Result<(), &'static str> {
        access.is_enabled().then_some(()).ok_or(
            "raw query access is disabled; use typed query APIs or explicitly trusted access",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::RawQueryAccess;
    #[test]
    fn raw_queries_are_disabled_by_default() {
        assert!(!RawQueryAccess::default().is_enabled());
        assert!(RawQueryAccess::trusted().is_enabled());
    }
}
