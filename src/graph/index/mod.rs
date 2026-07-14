//! Queryable knowledge index over the typed semantic graph.

mod architecture;
mod clusters;
mod common;
mod dead_code;
mod dependency_matrix;
mod package;
mod schema;
mod search;
mod trace;

pub use architecture::{
    ALL_ARCHITECTURE_ASPECTS, ArchitectureAspect, ArchitectureSummary, FileTreeNode,
    LanguageSummary,
};
pub use clusters::ArchitectureCluster;
pub(crate) use common::{node_file_path, node_label, node_name};
pub use dependency_matrix::DependencyMatrix;
pub use package::PackageSummary;
pub use schema::{GraphSchema, LabelCount, TypeCount};
pub use search::{SearchParams, SearchResult};
pub use trace::{NodeHop, TraceDirection, TraceParams, TraceRelation, TraceResult};

use crate::graph::{Graph, GraphNode, GraphNodeId};
use common::node_search_text;
use std::collections::BTreeMap;

/// Queryable knowledge index over one graph snapshot.
#[derive(Debug, Clone, Copy)]
pub struct KnowledgeIndex<'a> {
    graph: &'a Graph,
}

impl<'a> KnowledgeIndex<'a> {
    /// Creates an index over a graph snapshot.
    pub fn new(graph: &'a Graph) -> Self {
        Self { graph }
    }

    pub(crate) fn find_root(&self, query: &str) -> Option<&GraphNode> {
        let query_lower = query.to_lowercase();
        self.graph
            .nodes
            .iter()
            .find(|node| node.id().as_str() == query)
            .or_else(|| {
                self.graph
                    .nodes
                    .iter()
                    .find(|node| node_name(node) == query)
            })
            .or_else(|| {
                self.graph
                    .nodes
                    .iter()
                    .find(|node| node_search_text(node).contains(&query_lower))
            })
    }

    pub(crate) fn node_by_id(&self) -> BTreeMap<&GraphNodeId, &GraphNode> {
        self.graph
            .nodes
            .iter()
            .map(|node| (node.id(), node))
            .collect()
    }

    pub(crate) fn degree_index(&self) -> BTreeMap<&GraphNodeId, (usize, usize)> {
        let mut degree = BTreeMap::new();
        for node in &self.graph.nodes {
            degree.insert(node.id(), (0usize, 0usize));
        }
        for relation in &self.graph.relations {
            if let Some((_, out_degree)) = degree.get_mut(&relation.source) {
                *out_degree += 1;
            }
            if let Some((in_degree, _)) = degree.get_mut(&relation.target) {
                *in_degree += 1;
            }
        }
        degree
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ALL_ARCHITECTURE_ASPECTS, ArchitectureAspect, ArchitectureCluster, KnowledgeIndex,
        SearchParams, TraceDirection, TraceParams,
    };
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::collections::BTreeSet;
    use std::path::Path;

    fn fixture_graph() -> Result<crate::graph::Graph, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        Ok(GraphBuilder.build(&root, &artifacts))
    }

    #[test]
    fn schema_search_trace_and_architecture_are_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let index = KnowledgeIndex::new(&graph);

        let schema = index.schema();
        assert!(
            schema
                .node_labels
                .iter()
                .any(|label| label.label == "Artifact")
        );
        assert!(
            schema
                .edge_types
                .iter()
                .any(|edge| edge.edge_type == "Contains")
        );

        let search = index.search(&SearchParams {
            label: Some("Artifact".to_owned()),
            query: Some("python".to_owned()),
            limit: 5,
        });
        assert!(!search.is_empty());

        let trace = index
            .trace(&TraceParams {
                query: search[0].id.as_str().to_owned(),
                depth: 1,
                direction: TraceDirection::Both,
            })
            .ok_or("missing trace result")?;
        assert_eq!(trace.root.id, search[0].id);
        assert!(!trace.visited.is_empty());

        let architecture = index.architecture(None);
        assert!(!architecture.hotspots.is_empty());
        assert_eq!(architecture.schema, schema);

        Ok(())
    }

    #[test]
    fn dependency_matrix_tarjan_finds_a_module_cycle() -> Result<(), Box<dyn std::error::Error>> {
        use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath};
        use crate::graph::{
            Graph, GraphNode, GraphNodeId, ModuleLanguage, ModuleNode, Relation, RelationKind,
        };
        let path = RepoPath::new("src/lib.rs")?;
        let evidence = EvidenceRef::file(ArtifactId::from_path(&path), path);
        let module = |name: &str| {
            GraphNode::Module(ModuleNode {
                id: GraphNodeId::new(format!("module:{name}")),
                path: name.into(),
                language: ModuleLanguage::Rust,
                evidence: evidence.clone(),
            })
        };
        let relation = |id: &str, source: &str, target: &str| Relation {
            id: id.into(),
            source: GraphNodeId::new(format!("module:{source}")),
            target: GraphNodeId::new(format!("module:{target}")),
            kind: RelationKind::Imports,
            confidence: Confidence::High,
            evidence: vec![],
            provenance: None,
        };
        let graph = Graph {
            nodes: vec![module("a"), module("b"), module("c")],
            relations: vec![
                relation("ab", "a", "b"),
                relation("ba", "b", "a"),
                relation("bc", "b", "c"),
            ],
        };
        let matrix = KnowledgeIndex::new(&graph).dependency_matrix();
        assert_eq!(
            matrix.cycles,
            vec![vec![
                GraphNodeId::new("module:a"),
                GraphNodeId::new("module:b")
            ]]
        );
        assert_eq!(
            matrix.modules,
            vec![
                GraphNodeId::new("module:a"),
                GraphNodeId::new("module:b"),
                GraphNodeId::new("module:c")
            ]
        );
        assert_eq!(matrix.cells[0][1], 1);
        Ok(())
    }

    #[test]
    fn dependency_matrix_is_deterministic_and_exposes_cycles()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let index = KnowledgeIndex::new(&graph);
        let first = index.dependency_matrix();
        assert_eq!(first, index.dependency_matrix());
        assert_eq!(first.modules.len(), first.cells.len());
        assert!(
            first
                .cells
                .iter()
                .all(|row| row.len() == first.modules.len())
        );
        Ok(())
    }

    /// LIT-22.2.4 AC3: `package_dependencies` is the typed API an import
    /// resolver uses to look up a package's declared dependencies.
    #[test]
    fn package_dependencies_looks_up_declared_dependencies_by_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name": "acme-web", "dependencies": {"react": "^18.0.0", "lodash": "^4.0.0"}}"#,
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let dependencies = index.package_dependencies("acme-web");
        assert_eq!(dependencies.len(), 2);
        assert!(dependencies.iter().all(|dependency| dependency.is_external));
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.name == "react")
        );
        assert!(
            dependencies
                .iter()
                .any(|dependency| dependency.name == "lodash")
        );

        assert!(index.package_dependencies("does-not-exist").is_empty());

        Ok(())
    }

    /// LIT-22.3.4 AC3: HTTP routes and gRPC/GraphQL facts surface in
    /// `architecture().service_links`.
    #[test]
    fn architecture_summary_includes_service_links() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;
        std::fs::write(
            temp.path().join("api.proto"),
            "service Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply) {}\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let architecture = KnowledgeIndex::new(&graph).architecture(None);

        let names: Vec<&str> = architecture
            .service_links
            .iter()
            .map(|link| link.name.as_str())
            .collect();
        assert!(names.contains(&"GET /users/{id}"));
        assert!(names.contains(&"Greeter.SayHello"));

        Ok(())
    }

    /// LIT-22.4.1 AC1: `find_dead_code` flags an uncalled function and
    /// excludes a called one; `impact_analysis` always traces inbound
    /// regardless of the `direction` passed in, and reports no results for
    /// a query matching no node.
    #[test]
    fn find_dead_code_and_impact_analysis_behave_as_documented()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("app.py"),
            "def used():\n    return 1\n\n\ndef unused():\n    return 2\n\n\ndef caller():\n    return used()\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let dead_names: Vec<String> = index
            .find_dead_code()
            .into_iter()
            .map(|result| result.name)
            .collect();
        assert!(dead_names.iter().any(|name| name.ends_with("::unused")));
        assert!(!dead_names.iter().any(|name| name.ends_with("::used")));

        let used_id = index
            .search(&SearchParams {
                label: Some("Symbol".to_owned()),
                query: Some("app::used".to_owned()),
                limit: 1,
            })
            .into_iter()
            .next()
            .ok_or("missing used() symbol")?
            .id;
        let impact = index
            .impact_analysis(&TraceParams {
                query: used_id.as_str().to_owned(),
                depth: 2,
                direction: TraceDirection::Outbound,
            })
            .ok_or("missing impact result")?;
        // `Calls` relations are attributed to the containing artifact, not
        // the specific calling symbol, so `used()`'s only caller is
        // `app.py` itself, not a `caller` symbol node.
        assert!(
            impact
                .visited
                .iter()
                .any(|hop| hop.node.file_path.as_deref() == Some("app.py"))
        );

        assert!(
            index
                .impact_analysis(&TraceParams {
                    query: "no-such-node".to_owned(),
                    depth: 1,
                    direction: TraceDirection::Both,
                })
                .is_none()
        );

        Ok(())
    }

    /// LIT-22.5.1 AC1/AC4: a small connected graph (a function calling
    /// another in the same file) produces one cluster with real cohesion
    /// and edge-type evidence.
    #[test]
    fn clusters_group_a_small_connected_call_graph() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("app.py"),
            "def used():\n    return 1\n\n\ndef caller():\n    return used()\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let clusters = KnowledgeIndex::new(&graph).clusters();

        assert_eq!(clusters.len(), 1);
        let cluster = &clusters[0];
        assert!(cluster.members.len() >= 2);
        assert!(cluster.cohesion > 0.0);
        assert!(cluster.edge_types.contains(&"Calls".to_owned()));
        assert!(!cluster.top_nodes.is_empty());

        Ok(())
    }

    /// LIT-22.5.1 AC1/AC3/AC4: two unrelated package manifests produce two
    /// disjoint, cross-package clusters (an artifact belonging to its own
    /// local package, which depends on an external one), and clustering
    /// twice over the same unchanged graph is byte-identical.
    #[test]
    fn clusters_separate_disconnected_cross_package_communities()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("a"))?;
        std::fs::create_dir_all(temp.path().join("b"))?;
        std::fs::write(
            temp.path().join("a/package.json"),
            r#"{"name": "pkg-a", "dependencies": {"left-pad": "^1.0.0"}}"#,
        )?;
        std::fs::write(
            temp.path().join("b/package.json"),
            r#"{"name": "pkg-b", "dependencies": {"right-pad": "^1.0.0"}}"#,
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let clusters = index.clusters();
        assert_eq!(clusters.len(), 2);

        let cluster_of = |clusters: &[ArchitectureCluster], package: &str| {
            clusters
                .iter()
                .position(|cluster| cluster.packages.iter().any(|name| name == package))
        };
        let cluster_a = cluster_of(&clusters, "pkg-a").ok_or("missing pkg-a cluster")?;
        let cluster_b = cluster_of(&clusters, "pkg-b").ok_or("missing pkg-b cluster")?;
        assert_ne!(cluster_a, cluster_b);
        assert_eq!(cluster_of(&clusters, "left-pad"), Some(cluster_a));
        assert_eq!(cluster_of(&clusters, "right-pad"), Some(cluster_b));
        for cluster in &clusters {
            assert!(cluster.edge_types.contains(&"DependsOnPackage".to_owned()));
        }

        assert_eq!(clusters, index.clusters());

        Ok(())
    }

    /// LIT-22.4.6 AC1/AC2/AC3/AC4: a full `architecture()` call populates
    /// every optional section; a filtered call populates only the
    /// requested ones and leaves the rest empty; both are deterministic
    /// (re-running produces byte-identical output) and reflect only
    /// evidence already in the graph.
    #[test]
    fn architecture_summary_full_and_filtered_snapshots() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/api"))?;
        std::fs::write(
            temp.path().join("src/api/routes.py"),
            "@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name": "acme-web", "dependencies": {"react": "^18.0.0"}}"#,
        )?;
        std::fs::write(temp.path().join("README.md"), "# Architecture\n")?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let index = KnowledgeIndex::new(&graph);

        let full = index.architecture(None);
        assert!(!full.languages.is_empty());
        assert!(!full.packages.is_empty());
        assert!(!full.service_links.is_empty());
        assert!(!full.layers.is_empty());
        assert!(!full.clusters.is_empty());
        assert!(!full.file_tree.is_empty());
        assert!(!full.architecture_docs.is_empty());
        assert!(!full.hotspots.is_empty());
        assert_eq!(full, index.architecture(None), "must be deterministic");
        assert_eq!(
            full,
            index.architecture(Some(&BTreeSet::from_iter(
                ALL_ARCHITECTURE_ASPECTS.iter().copied()
            ))),
            "explicit full aspect set must match the None default"
        );

        let filtered = index.architecture(Some(&BTreeSet::from([
            ArchitectureAspect::Packages,
            ArchitectureAspect::Layers,
        ])));
        assert!(!filtered.packages.is_empty());
        assert!(!filtered.layers.is_empty());
        assert!(filtered.languages.is_empty());
        assert!(filtered.service_links.is_empty());
        assert!(filtered.clusters.is_empty());
        assert!(filtered.file_tree.is_empty());
        assert!(filtered.architecture_docs.is_empty());
        // Always-on sections stay populated regardless of the filter.
        assert!(!filtered.hotspots.is_empty());
        assert_eq!(filtered.schema, full.schema);

        Ok(())
    }
}
