//! Deterministic parity-benchmark metrics for regression fixtures.

use crate::graph::Graph;

/// Fixture benchmark record; costs are deterministic operation counts rather
/// than wall-clock times so CI comparisons remain stable across machines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParityBenchmark {
    /// Indexed graph node count.
    pub node_count: usize,
    /// Indexed graph relation count.
    pub relation_count: usize,
    /// Query work estimate (nodes plus relations inspected by schema/search).
    pub query_work_units: usize,
    /// Layout/analytics work estimate (pairwise/community candidate cost).
    pub analytics_work_units: usize,
}

/// Measures a fixture graph's deterministic parity costs.
pub fn measure(graph: &Graph) -> ParityBenchmark {
    let node_count = graph.nodes.len();
    let relation_count = graph.relations.len();
    ParityBenchmark {
        node_count,
        relation_count,
        query_work_units: node_count + relation_count,
        analytics_work_units: node_count.saturating_mul(node_count.saturating_sub(1)) / 2
            + relation_count,
    }
}

#[cfg(test)]
mod tests {
    use super::measure;
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;
    #[test]
    fn polyglot_benchmark_is_deterministic_and_nonzero() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let first = measure(&graph);
        assert_eq!(first, measure(&graph));
        assert!(first.node_count > 0 && first.relation_count > 0 && first.query_work_units > 0);
        Ok(())
    }
}
