use super::*;

impl BuilderState {
    /// Produces a deterministic read-only checkpoint without consuming state.
    pub(super) fn snapshot(&self) -> Graph {
        let mut nodes: Vec<GraphNode> = self.nodes.values().cloned().collect();
        nodes.sort_by(|a, b| a.id().cmp(b.id()));
        let mut relations = self.relations.clone();
        relations
            .sort_by(|a, b| (&a.source, a.kind, &a.target).cmp(&(&b.source, b.kind, &b.target)));
        Graph { nodes, relations }
    }
    pub(super) fn finish(self) -> Graph {
        self.snapshot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    #[test]
    fn graph_covers_every_node_kind_and_relation_has_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let mut seen_artifact = false;
        let mut seen_symbol = false;
        let mut seen_config = false;
        let mut seen_documentation = false;
        let mut seen_container = false;
        let mut seen_command = false;
        let mut seen_env_var = false;
        let mut seen_module = false;
        let mut seen_package = false;
        let mut seen_unresolved = false;
        for node in &graph.nodes {
            match node {
                GraphNode::Artifact(_) => seen_artifact = true,
                GraphNode::Symbol(_) => seen_symbol = true,
                GraphNode::Config(_) => seen_config = true,
                GraphNode::Documentation(_) => seen_documentation = true,
                GraphNode::Container(_) => seen_container = true,
                GraphNode::Command(_) => seen_command = true,
                GraphNode::EnvVar(_) => seen_env_var = true,
                GraphNode::Module(_) => seen_module = true,
                GraphNode::Package(_) => seen_package = true,
                GraphNode::Unresolved(_) => seen_unresolved = true,
            }
        }
        assert!(seen_artifact && seen_symbol && seen_config && seen_documentation);
        assert!(seen_container && seen_command && seen_env_var && seen_module);
        assert!(seen_package && seen_unresolved);

        assert!(!graph.relations.is_empty());
        assert!(
            graph
                .relations
                .iter()
                .all(|relation| !relation.evidence.is_empty())
        );
        let ids: std::collections::BTreeSet<_> = graph.nodes.iter().map(|node| node.id()).collect();
        for relation in &graph.relations {
            assert!(
                ids.contains(&relation.source),
                "dangling source {relation:?}"
            );
            assert!(
                ids.contains(&relation.target),
                "dangling target {relation:?}"
            );
        }

        Ok(())
    }

    #[test]
    fn graph_export_is_deterministic_json() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let first = GraphBuilder.build(&root, &artifacts).to_json()?;
        let second = GraphBuilder.build(&root, &artifacts).to_json()?;

        assert_eq!(first, second);
        assert!(first.contains("\"node_type\": \"Artifact\""));
        let round_tripped: crate::graph::Graph = serde_json::from_str(&first)?;
        assert_eq!(GraphBuilder.build(&root, &artifacts), round_tripped);

        Ok(())
    }

    #[test]
    fn graph_keeps_every_artifact_node_including_unsupported()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        for artifact in &artifacts {
            let id = format!("artifact:{}", artifact.path);
            assert!(
                graph.nodes.iter().any(|node| node.id().as_str() == id),
                "missing artifact node for {}",
                artifact.path
            );
        }
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == "artifact:data/sample.bin")
        );

        Ok(())
    }

    /// A relation that stays genuinely unresolved keeps its `Unresolved`
    /// node in the graph -- pruning only removes nodes no relation targets
    /// anymore, never a node a caller might still want to inspect.
    #[test]
    fn unresolved_nodes_still_targeted_by_a_relation_are_not_pruned()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/main/java"))?;
        std::fs::write(
            temp.path().join("src/main/java/App.java"),
            "import com.example.totally.unknown.Widget;\nclass App {}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let relation = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/main/java/App.java"
            })
            .ok_or("missing import relation")?;
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id() == &relation.target
                    && matches!(node, GraphNode::Unresolved(_))),
            "an unresolvable import's Unresolved node must still be present, not pruned"
        );

        Ok(())
    }
}
