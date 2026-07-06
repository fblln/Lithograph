//! Deterministic repository research artifacts used as an intermediate
//! memory layer between graph analysis and documentation composition.

use crate::domain::{Artifact, ArtifactCategory};
use crate::graph::{Graph, GraphNode, RelationKind};
use crate::plan::DocumentationModule;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Structured, persisted research facts for one repository snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResearchBrief {
    /// High-level module facts and likely responsibilities.
    pub system_context: Vec<String>,
    /// Command/runtime/configuration facts that suggest workflows.
    pub workflows: Vec<String>,
    /// External packages, env vars, unresolved refs, and boundary relations.
    pub boundaries: Vec<String>,
    /// Manifests, deployment/config files, packages, and env vars.
    pub configuration: Vec<String>,
    /// Largest or most connected modules to prioritize in summaries.
    pub key_modules: Vec<String>,
}

/// Builds deterministic research artifacts from the already-validated graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct ResearchBuilder;

impl ResearchBuilder {
    /// Derives a compact research brief without calling a model.
    pub fn build(
        &self,
        artifacts: &[Artifact],
        graph: &Graph,
        modules: &[DocumentationModule],
    ) -> ResearchBrief {
        ResearchBrief {
            system_context: system_context(modules),
            workflows: workflows(graph),
            boundaries: boundaries(graph),
            configuration: configuration(artifacts, graph),
            key_modules: key_modules(modules, graph),
        }
    }
}

fn system_context(modules: &[DocumentationModule]) -> Vec<String> {
    modules
        .iter()
        .take(20)
        .map(|module| {
            format!(
                "{} ({:?}) owns {} graph member(s) and is estimated at {} tokens",
                module.name,
                module.kind,
                module.members.len(),
                module.estimated_tokens
            )
        })
        .collect()
}

fn workflows(graph: &Graph) -> Vec<String> {
    let labels = node_labels(graph);
    graph
        .relations
        .iter()
        .filter(|relation| {
            matches!(
                relation.kind,
                RelationKind::RunsCommand
                    | RelationKind::UsesImage
                    | RelationKind::BuildsImage
                    | RelationKind::PublishesImage
                    | RelationKind::ReadsEnv
            )
        })
        .take(40)
        .map(|relation| {
            format!(
                "{} -[{:?}]-> {} ({:?})",
                labels
                    .get(relation.source.as_str())
                    .map_or(relation.source.as_str(), String::as_str),
                relation.kind,
                labels
                    .get(relation.target.as_str())
                    .map_or(relation.target.as_str(), String::as_str),
                relation.confidence
            )
        })
        .collect()
}

fn boundaries(graph: &Graph) -> Vec<String> {
    let labels = node_labels(graph);
    let mut facts = Vec::new();
    for node in &graph.nodes {
        match node {
            GraphNode::Package(package) if package.is_external => {
                facts.push(format!("external package: {}", package.name));
            }
            GraphNode::EnvVar(env) => facts.push(format!("environment variable: {}", env.name)),
            GraphNode::Unresolved(unresolved) => {
                facts.push(format!("unresolved reference: {}", unresolved.value));
            }
            _ => {}
        }
        if facts.len() >= 40 {
            return facts;
        }
    }
    for relation in &graph.relations {
        if matches!(
            relation.kind,
            RelationKind::DependsOnPackage | RelationKind::ReadsEnv | RelationKind::References
        ) {
            facts.push(format!(
                "{} crosses boundary via {:?} to {}",
                labels
                    .get(relation.source.as_str())
                    .map_or(relation.source.as_str(), String::as_str),
                relation.kind,
                labels
                    .get(relation.target.as_str())
                    .map_or(relation.target.as_str(), String::as_str)
            ));
        }
        if facts.len() >= 40 {
            break;
        }
    }
    facts
}

fn configuration(artifacts: &[Artifact], graph: &Graph) -> Vec<String> {
    let mut facts: Vec<String> = artifacts
        .iter()
        .filter(|artifact| {
            matches!(
                artifact.category,
                ArtifactCategory::Configuration
                    | ArtifactCategory::BuildDefinition
                    | ArtifactCategory::PackageManifest
                    | ArtifactCategory::DependencyLockfile
                    | ArtifactCategory::ContainerDefinition
                    | ArtifactCategory::DeploymentDefinition
                    | ArtifactCategory::ContinuousIntegration
                    | ArtifactCategory::DatabaseSchema
                    | ArtifactCategory::DatabaseMigration
            )
        })
        .take(30)
        .map(|artifact| format!("{:?}: {}", artifact.category, artifact.path))
        .collect();
    for node in &graph.nodes {
        match node {
            GraphNode::Config(config) => facts.push(format!("{:?}: {}", config.kind, config.name)),
            GraphNode::Package(package) => facts.push(format!(
                "package: {}{}",
                package.name,
                if package.is_external {
                    " (external)"
                } else {
                    ""
                }
            )),
            _ => {}
        }
        if facts.len() >= 60 {
            break;
        }
    }
    facts
}

fn key_modules(modules: &[DocumentationModule], graph: &Graph) -> Vec<String> {
    let mut degree: BTreeMap<&str, usize> = BTreeMap::new();
    for module in modules {
        degree.insert(module.id.as_str(), module.members.len());
    }
    for relation in &graph.relations {
        for module in modules {
            if (module.members.contains(&relation.source)
                || module.members.contains(&relation.target))
                && let Some(count) = degree.get_mut(module.id.as_str())
            {
                *count += 1;
            }
        }
    }
    let mut modules_by_score: Vec<(&DocumentationModule, usize)> = modules
        .iter()
        .map(|module| (module, degree.get(module.id.as_str()).copied().unwrap_or(0)))
        .collect();
    modules_by_score.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.name.cmp(&b.0.name)));
    modules_by_score
        .into_iter()
        .take(10)
        .map(|(module, score)| format!("{} ({:?}) score {}", module.name, module.kind, score))
        .collect()
}

fn node_labels(graph: &Graph) -> BTreeMap<&str, String> {
    graph
        .nodes
        .iter()
        .map(|node| {
            let label = match node {
                GraphNode::Artifact(node) => node.path.clone(),
                GraphNode::Symbol(node) => node.qualified_name.clone(),
                GraphNode::Config(node) => node.name.clone(),
                GraphNode::Documentation(node) => node.title.clone(),
                GraphNode::Container(node) => node.reference.clone(),
                GraphNode::Command(node) => node.text.clone(),
                GraphNode::EnvVar(node) => node.name.clone(),
                GraphNode::Module(node) => node.path.clone(),
                GraphNode::Package(node) => node.name.clone(),
                GraphNode::Unresolved(node) => node.value.clone(),
            };
            (node.id().as_str(), label)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ResearchBuilder;
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::plan::ModulePlanner;
    use std::path::Path;

    #[test]
    fn builds_research_brief_from_polyglot_fixture() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let brief = ResearchBuilder.build(&artifacts, &graph, &modules);

        assert!(!brief.system_context.is_empty());
        assert!(!brief.configuration.is_empty());
        assert!(!brief.key_modules.is_empty());

        Ok(())
    }
}
