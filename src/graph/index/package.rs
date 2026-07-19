//! Package summaries and declared-dependency lookups.

use super::KnowledgeIndex;
#[cfg(test)]
use crate::graph::{GraphNode, RelationKind};
use serde::{Deserialize, Serialize};

/// Package summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PackageSummary {
    /// Package name.
    pub name: String,
    /// True when external to the repository.
    pub is_external: bool,
    /// Inbound relation count.
    pub in_degree: usize,
    /// Outbound relation count.
    pub out_degree: usize,
}

impl<'a> KnowledgeIndex<'a> {
    /// Typed package-map lookup for import resolvers (LIT-22.2.4 AC3):
    /// returns every package `package_name` declares a `DependsOnPackage`
    /// edge to, local or external. `package_name` matches a `Package` node's
    /// name exactly (e.g. a registry id from a manifest analyzer), not a
    /// substring.
    #[cfg(test)]
    pub(crate) fn package_dependencies(&self, package_name: &str) -> Vec<PackageSummary> {
        let degree = self.degree_index();
        let node_by_id = self.node_by_id();
        let Some(source_id) = self.graph.nodes.iter().find_map(|node| match node {
            GraphNode::Package(package) if package.name == package_name => Some(node.id()),
            _ => None,
        }) else {
            return Vec::new();
        };

        let mut dependencies: Vec<PackageSummary> = self
            .graph
            .relations
            .iter()
            .filter(|relation| {
                relation.kind == RelationKind::DependsOnPackage && &relation.source == source_id
            })
            .filter_map(|relation| {
                let node = node_by_id.get(&relation.target)?;
                let GraphNode::Package(package) = node else {
                    return None;
                };
                let (in_degree, out_degree) = degree.get(node.id()).copied().unwrap_or((0, 0));
                Some(PackageSummary {
                    name: package.name.clone(),
                    is_external: package.is_external,
                    in_degree,
                    out_degree,
                })
            })
            .collect();
        dependencies.sort_by(|a, b| a.name.cmp(&b.name));
        dependencies.dedup_by(|a, b| a.name == b.name);
        dependencies
    }
}
