//! Deterministic documentation module planning.
//!
//! Modules are the unit of documentation generation. They are derived from
//! artifact paths and the semantic graph using fixed rules (crate/package
//! ownership, directory boundaries, artifact category) rather than LLM
//! clustering, so the same repository always plans the same modules.

use crate::domain::{Artifact, ArtifactCategory};
use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Deterministic module boundary category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModuleKind {
    /// A Rust workspace member/crate, keyed by crate name.
    RustCrate,
    /// A Python top-level package, keyed by package directory name.
    PythonPackage,
    /// A source-heavy directory not owned by a crate or package.
    Directory,
    /// Container/CI/deployment artifacts.
    Infrastructure,
    /// Existing documentation artifacts.
    Documentation,
    /// Configuration not owned by another module.
    Configuration,
}

impl ModuleKind {
    fn slug(self) -> &'static str {
        match self {
            Self::RustCrate => "rust-crate",
            Self::PythonPackage => "python-package",
            Self::Directory => "directory",
            Self::Infrastructure => "infrastructure",
            Self::Documentation => "documentation",
            Self::Configuration => "configuration",
        }
    }
}

/// One deterministic documentation module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentationModule {
    /// Stable module identifier.
    pub id: String,
    /// Human-readable module name.
    pub name: String,
    /// Module boundary category.
    pub kind: ModuleKind,
    /// Graph nodes belonging to this module: member artifacts plus every
    /// node reachable from them by a relation, stopping at other artifacts.
    pub members: Vec<GraphNodeId>,
    /// Deterministic hash over member artifact paths and content hashes.
    pub input_hash: String,
    /// Heuristic token estimate for this module's documentation context.
    pub estimated_tokens: u32,
}

/// Plans deterministic documentation modules from a repository's artifacts
/// and semantic graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModulePlanner;

impl ModulePlanner {
    /// Plans modules, sorted by ID for deterministic output.
    pub fn plan(&self, graph: &Graph, artifacts: &[Artifact]) -> Vec<DocumentationModule> {
        let artifact_paths: BTreeSet<String> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str().to_owned())
            .collect();
        let cargo_toml_paths: BTreeSet<String> = artifacts
            .iter()
            .filter(|artifact| file_name(artifact.path.as_str()) == "Cargo.toml")
            .map(|artifact| artifact.path.as_str().to_owned())
            .collect();
        let package_names = package_names_by_manifest(graph);

        let mut buckets: BTreeMap<(ModuleKind, String), Vec<&Artifact>> = BTreeMap::new();
        for artifact in artifacts {
            let key =
                classify_artifact(artifact, &cargo_toml_paths, &artifact_paths, &package_names);
            buckets.entry(key).or_default().push(artifact);
        }

        let by_source = index_relations_by_source(graph);
        let node_by_id = index_nodes_by_id(graph);

        let mut modules: Vec<DocumentationModule> = buckets
            .into_iter()
            .map(|((kind, name), members)| {
                build_module(kind, name, &members, &by_source, &node_by_id)
            })
            .collect();
        modules.sort_by(|a, b| a.id.cmp(&b.id));
        modules
    }
}

fn classify_artifact(
    artifact: &Artifact,
    cargo_toml_paths: &BTreeSet<String>,
    artifact_paths: &BTreeSet<String>,
    package_names: &BTreeMap<String, String>,
) -> (ModuleKind, String) {
    let path = artifact.path.as_str();

    if artifact.detected_format.as_deref() == Some("rust")
        && let Some(manifest) = rust_crate_manifest(path, cargo_toml_paths)
        && let Some(name) = package_names.get(&manifest)
    {
        return (ModuleKind::RustCrate, name.clone());
    }
    if file_name(path) == "Cargo.toml"
        && let Some(name) = package_names.get(path)
    {
        return (ModuleKind::RustCrate, name.clone());
    }
    if artifact.detected_format.as_deref() == Some("python")
        && let Some(root) = python_package_root(path, artifact_paths)
    {
        return (ModuleKind::PythonPackage, top_level_name(&root));
    }

    match artifact.category {
        ArtifactCategory::ContainerDefinition
        | ArtifactCategory::ContinuousIntegration
        | ArtifactCategory::DeploymentDefinition => {
            (ModuleKind::Infrastructure, "Infrastructure".to_owned())
        }
        ArtifactCategory::Documentation => (ModuleKind::Documentation, "Documentation".to_owned()),
        ArtifactCategory::Configuration
        | ArtifactCategory::PackageManifest
        | ArtifactCategory::DependencyLockfile => {
            (ModuleKind::Configuration, "Configuration".to_owned())
        }
        _ => (ModuleKind::Directory, top_level_directory(path)),
    }
}

/// Finds the nearest ancestor directory (including the repository root)
/// that owns a `Cargo.toml`, so files under `src/bin/`, `src/`, etc. all
/// resolve to the same crate manifest.
fn rust_crate_manifest(path: &str, cargo_toml_paths: &BTreeSet<String>) -> Option<String> {
    let mut dir = parent_dir(path);
    loop {
        let candidate = if dir.is_empty() {
            "Cargo.toml".to_owned()
        } else {
            format!("{dir}/Cargo.toml")
        };
        if cargo_toml_paths.contains(&candidate) {
            return Some(candidate);
        }
        if dir.is_empty() {
            return None;
        }
        dir = parent_dir(&dir);
    }
}

/// Finds the outermost ancestor directory that is still a Python package
/// (has an `__init__.py`), so nested subpackages collapse into one
/// top-level package module.
fn python_package_root(path: &str, artifact_paths: &BTreeSet<String>) -> Option<String> {
    let file_dir = parent_dir(path);
    if !has_init(&file_dir, artifact_paths) {
        return None;
    }
    let mut root = file_dir;
    loop {
        let parent = parent_dir(&root);
        if parent.is_empty() || !has_init(&parent, artifact_paths) {
            break;
        }
        root = parent;
    }
    Some(root)
}

fn has_init(dir: &str, artifact_paths: &BTreeSet<String>) -> bool {
    artifact_paths.contains(&format!("{dir}/__init__.py"))
}

fn package_names_by_manifest(graph: &Graph) -> BTreeMap<String, String> {
    graph
        .relations
        .iter()
        .filter(|relation| relation.kind == RelationKind::BelongsToPackage)
        .filter_map(|relation| {
            let manifest_path = relation.source.as_str().strip_prefix("artifact:")?;
            let package = graph.nodes.iter().find_map(|node| match node {
                GraphNode::Package(package) if node.id() == &relation.target => Some(package),
                _ => None,
            })?;
            Some((manifest_path.to_owned(), package.name.clone()))
        })
        .collect()
}

fn index_relations_by_source(graph: &Graph) -> BTreeMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut index: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
    for relation in &graph.relations {
        index
            .entry(relation.source.clone())
            .or_default()
            .push(relation.target.clone());
    }
    index
}

fn index_nodes_by_id(graph: &Graph) -> BTreeMap<GraphNodeId, &GraphNode> {
    graph
        .nodes
        .iter()
        .map(|node| (node.id().clone(), node))
        .collect()
}

/// Expands member artifacts to every node reachable from them, stopping at
/// any other `Artifact` node so one module never absorbs another module's
/// files just because a relation (e.g. a Markdown link) points at them.
fn expand_members(
    seeds: &[GraphNodeId],
    by_source: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    node_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
) -> Vec<GraphNodeId> {
    let mut visited: BTreeSet<GraphNodeId> = seeds.iter().cloned().collect();
    let mut frontier: Vec<GraphNodeId> = seeds.to_vec();

    while let Some(current) = frontier.pop() {
        let Some(targets) = by_source.get(&current) else {
            continue;
        };
        for target in targets {
            if visited.contains(target) {
                continue;
            }
            if matches!(node_by_id.get(target), Some(GraphNode::Artifact(_))) {
                continue;
            }
            visited.insert(target.clone());
            frontier.push(target.clone());
        }
    }

    visited.into_iter().collect()
}

fn build_module(
    kind: ModuleKind,
    name: String,
    members: &[&Artifact],
    by_source: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    node_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
) -> DocumentationModule {
    let seeds: Vec<GraphNodeId> = members
        .iter()
        .map(|artifact| GraphNodeId::new(format!("artifact:{}", artifact.path)))
        .collect();
    let members_expanded = expand_members(&seeds, by_source, node_by_id);

    DocumentationModule {
        id: format!("module-plan:{}:{}", kind.slug(), slugify(&name)),
        name,
        kind,
        members: members_expanded,
        input_hash: compute_input_hash(members),
        estimated_tokens: estimate_tokens(members),
    }
}

fn compute_input_hash(members: &[&Artifact]) -> String {
    let mut sorted: Vec<&&Artifact> = members.iter().collect();
    sorted.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    let joined = sorted
        .iter()
        .map(|artifact| format!("{}:{}", artifact.path, artifact.content_hash))
        .collect::<Vec<_>>()
        .join("\n");
    blake3::hash(joined.as_bytes()).to_hex().to_string()
}

// ponytail: ~4 bytes/token is a coarse, model-agnostic heuristic good enough
// for deciding whether a module needs splitting before generation. Swap for
// a real tokenizer if the context builder (LIT-1.22) needs tighter budgets.
fn estimate_tokens(members: &[&Artifact]) -> u32 {
    let total_bytes: u64 = members.iter().map(|artifact| artifact.size_bytes).sum();
    u32::try_from(total_bytes / 4).unwrap_or(u32::MAX)
}

fn slugify(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn top_level_directory(path: &str) -> String {
    path.split_once('/')
        .map_or_else(|| "root".to_owned(), |(dir, _)| dir.to_owned())
}

fn top_level_name(dir: &str) -> String {
    dir.rsplit('/').next().unwrap_or(dir).to_owned()
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_owned())
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::{ModuleKind, ModulePlanner};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    #[test]
    fn plan_groups_fixture_into_deterministic_modules() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let modules = ModulePlanner.plan(&graph, &artifacts);

        let names: Vec<(ModuleKind, &str)> = modules
            .iter()
            .map(|module| (module.kind, module.name.as_str()))
            .collect();
        assert!(names.contains(&(ModuleKind::RustCrate, "fixture-worker")));
        assert!(names.contains(&(ModuleKind::PythonPackage, "python_app")));
        assert!(names.contains(&(ModuleKind::Infrastructure, "Infrastructure")));
        assert!(names.contains(&(ModuleKind::Documentation, "Documentation")));
        assert!(names.contains(&(ModuleKind::Configuration, "Configuration")));
        assert!(names.contains(&(ModuleKind::Directory, "web")));

        Ok(())
    }

    #[test]
    fn rust_crate_module_contains_lib_and_bin_and_no_vendor_leak()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let crate_module = modules
            .iter()
            .find(|module| module.kind == ModuleKind::RustCrate)
            .ok_or("rust crate module")?;

        assert!(
            crate_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:rust/src/lib.rs")
        );
        assert!(
            crate_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:rust/src/bin/worker.rs")
        );
        assert!(
            !crate_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:vendor/example/lib.rs")
        );
        assert!(
            crate_module
                .members
                .iter()
                .any(|id| id.as_str().starts_with("symbol:rust/src/lib.rs#"))
        );

        let directory_module = modules
            .iter()
            .find(|module| module.kind == ModuleKind::Directory && module.name == "vendor")
            .ok_or("vendor directory module")?;
        assert!(
            directory_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:vendor/example/lib.rs")
        );

        Ok(())
    }

    #[test]
    fn module_plan_is_deterministic_and_has_hashes_and_token_estimates()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let first = ModulePlanner.plan(&graph, &artifacts);
        let second = ModulePlanner.plan(&graph, &artifacts);

        assert_eq!(first, second);
        assert!(first.iter().all(|module| !module.input_hash.is_empty()));
        assert!(first.iter().all(|module| module.estimated_tokens > 0));

        let ids: Vec<&str> = first.iter().map(|module| module.id.as_str()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        assert_eq!(ids, sorted_ids);

        Ok(())
    }

    #[test]
    fn module_plan_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let snapshot = modules
            .iter()
            .map(|module| {
                format!(
                    "{}|{:?}|{}|{}|{}",
                    module.id,
                    module.kind,
                    module.name,
                    module.members.len(),
                    module.estimated_tokens
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            snapshot,
            "\
module-plan:configuration:configuration|Configuration|Configuration|21|258
module-plan:directory:assets|Directory|assets|2|55
module-plan:directory:data|Directory|data|1|7
module-plan:directory:generated|Directory|generated|3|32
module-plan:directory:root|Directory|root|6|68
module-plan:directory:vendor|Directory|vendor|3|15
module-plan:directory:web|Directory|web|5|131
module-plan:documentation:documentation|Documentation|Documentation|13|945
module-plan:infrastructure:infrastructure|Infrastructure|Infrastructure|19|335
module-plan:python-package:python-app|PythonPackage|python_app|16|291
module-plan:rust-crate:fixture-worker|RustCrate|fixture-worker|13|319"
        );

        Ok(())
    }
}
