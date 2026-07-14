//! Architecture-oriented graph summary (languages, packages, entry points,
//! boundaries, docs, service links, layers, clusters, file tree).

use super::KnowledgeIndex;
use super::clusters::ArchitectureCluster;
use super::common::search_result;
use super::package::PackageSummary;
use super::schema::GraphSchema;
use super::search::SearchResult;
use crate::graph::{ConfigNodeKind, Graph, GraphNode};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// One optional section of [`ArchitectureSummary`] a caller can request via
/// [`KnowledgeIndex::architecture`] (LIT-22.4.6 AC2). `Schema` and
/// `Hotspots` are always included (schema is free -- it's already computed
/// for every other aspect's degree index -- and hotspots is the summary's
/// original always-on section), so they have no aspect variant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArchitectureAspect {
    /// Module-language breakdown.
    Languages,
    /// Local and external package nodes.
    Packages,
    /// Command/container entry points.
    EntryPoints,
    /// External packages, env vars, and unresolved references.
    Boundaries,
    /// Existing architecture/decision documentation.
    ArchitectureDocs,
    /// HTTP/RPC/GraphQL routes and Compose services.
    ServiceLinks,
    /// Per-artifact layer classification (LIT-22.5.2).
    Layers,
    /// Functional architecture communities (LIT-22.5.1).
    Clusters,
    /// Nested repository file tree.
    FileTree,
}

/// All [`ArchitectureAspect`] variants, in the order `architecture()`
/// computes them when no filter is given. Passing `Some` of this set
/// explicitly is equivalent to passing `None`.
pub const ALL_ARCHITECTURE_ASPECTS: &[ArchitectureAspect] = &[
    ArchitectureAspect::Languages,
    ArchitectureAspect::Packages,
    ArchitectureAspect::EntryPoints,
    ArchitectureAspect::Boundaries,
    ArchitectureAspect::ArchitectureDocs,
    ArchitectureAspect::ServiceLinks,
    ArchitectureAspect::Layers,
    ArchitectureAspect::Clusters,
    ArchitectureAspect::FileTree,
];

/// Module count for one language, derived from `Module` graph nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanguageSummary {
    /// Registry language id (e.g. `"python"`, `"tsx"`).
    pub language: String,
    /// Number of `Module` nodes in this language.
    pub module_count: usize,
}

/// One file or directory in [`ArchitectureSummary::file_tree`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileTreeNode {
    /// File or directory name (the final path component).
    pub name: String,
    /// Repository-relative path.
    pub path: String,
    /// `true` for a directory, `false` for a file (an `Artifact` node).
    pub is_directory: bool,
    /// Child entries, directories first then files, each alphabetical.
    pub children: Vec<FileTreeNode>,
}

/// Architecture-oriented graph summary inspired by codebase-memory-style
/// queries. `PartialEq` only, not `Eq` -- `clusters` carries a `f64`
/// cohesion score.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchitectureSummary {
    /// Graph schema counts. Always included.
    pub schema: GraphSchema,
    /// Module-language breakdown.
    pub languages: Vec<LanguageSummary>,
    /// Local and external package nodes.
    pub packages: Vec<PackageSummary>,
    /// Entry points inferred from commands, containers, and high-degree source symbols.
    pub entry_points: Vec<SearchResult>,
    /// High-degree graph nodes. Always included.
    pub hotspots: Vec<SearchResult>,
    /// External packages, env vars, and unresolved references.
    pub boundaries: Vec<SearchResult>,
    /// Existing architecture or decision documentation nodes.
    pub architecture_docs: Vec<SearchResult>,
    /// HTTP routes, gRPC/protobuf RPCs, GraphQL fields, and Compose
    /// services (LIT-22.3.4 AC3): every `Config` node whose kind is
    /// `Route` or `Service`.
    pub service_links: Vec<SearchResult>,
    /// Per-artifact architecture layer classification (LIT-22.5.2 AC2).
    pub layers: Vec<crate::architecture::ArchitectureLayer>,
    /// Functional architecture communities (LIT-22.5.1).
    pub clusters: Vec<ArchitectureCluster>,
    /// Nested repository file tree, rooted at the repository root.
    pub file_tree: Vec<FileTreeNode>,
}

impl<'a> KnowledgeIndex<'a> {
    /// Returns an architecture summary over the graph. `aspects` selects
    /// which optional sections to compute and populate (LIT-22.4.6 AC2);
    /// `None` computes every aspect (equivalent to passing
    /// [`ALL_ARCHITECTURE_ASPECTS`]). Unrequested aspects are skipped
    /// entirely, not just filtered from the output, so a caller that only
    /// needs e.g. `Packages` avoids paying for clustering or layer
    /// detection. `schema` and `hotspots` are always computed; every
    /// section is deterministic for an unchanged graph (AC3).
    pub fn architecture(
        &self,
        aspects: Option<&BTreeSet<ArchitectureAspect>>,
    ) -> ArchitectureSummary {
        let wants = |aspect: ArchitectureAspect| aspects.is_none_or(|set| set.contains(&aspect));
        let degree = self.degree_index();
        let mut languages: BTreeMap<String, usize> = BTreeMap::new();
        let mut packages = Vec::new();
        let mut entry_points = Vec::new();
        let mut boundaries = Vec::new();
        let mut architecture_docs = Vec::new();
        let mut service_links = Vec::new();
        let mut all_results = Vec::new();

        for node in &self.graph.nodes {
            let result = search_result(node, &degree);
            match node {
                GraphNode::Module(module) if wants(ArchitectureAspect::Languages) => {
                    *languages.entry(module_language_id(module)).or_default() += 1;
                }
                GraphNode::Package(package) if wants(ArchitectureAspect::Packages) => {
                    packages.push(PackageSummary {
                        name: package.name.clone(),
                        is_external: package.is_external,
                        in_degree: result.in_degree,
                        out_degree: result.out_degree,
                    });
                    if package.is_external && wants(ArchitectureAspect::Boundaries) {
                        boundaries.push(result.clone());
                    }
                }
                GraphNode::Config(config)
                    if wants(ArchitectureAspect::ServiceLinks)
                        && matches!(
                            config.kind,
                            ConfigNodeKind::Route | ConfigNodeKind::Service
                        ) =>
                {
                    service_links.push(result.clone());
                }
                GraphNode::Command(_) | GraphNode::Container(_)
                    if wants(ArchitectureAspect::EntryPoints) =>
                {
                    entry_points.push(result.clone())
                }
                GraphNode::EnvVar(_) | GraphNode::Unresolved(_)
                    if wants(ArchitectureAspect::Boundaries) =>
                {
                    boundaries.push(result.clone())
                }
                GraphNode::Documentation(doc)
                    if wants(ArchitectureAspect::ArchitectureDocs)
                        && (doc.title.to_lowercase().contains("architecture")
                            || doc.title.to_lowercase().contains("decision")) =>
                {
                    architecture_docs.push(result.clone());
                }
                GraphNode::Artifact(artifact)
                    if wants(ArchitectureAspect::ArchitectureDocs)
                        && (artifact.path.to_lowercase().contains("architecture")
                            || artifact.path.to_lowercase().contains("adr")) =>
                {
                    architecture_docs.push(result.clone());
                }
                _ => {}
            }
            all_results.push(result);
        }

        all_results.sort_by(|a, b| {
            (b.in_degree + b.out_degree)
                .cmp(&(a.in_degree + a.out_degree))
                .then(a.name.cmp(&b.name))
        });
        let mut hotspots = all_results;
        hotspots.truncate(10);
        entry_points.sort_by(|a, b| a.name.cmp(&b.name));
        entry_points.truncate(20);
        boundaries.sort_by(|a, b| a.label.cmp(&b.label).then(a.name.cmp(&b.name)));
        boundaries.truncate(30);
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        architecture_docs.sort_by(|a, b| a.name.cmp(&b.name));
        service_links.sort_by(|a, b| a.name.cmp(&b.name));
        let languages = languages
            .into_iter()
            .map(|(language, module_count)| LanguageSummary {
                language,
                module_count,
            })
            .collect();

        ArchitectureSummary {
            schema: self.schema(),
            languages,
            packages,
            entry_points,
            hotspots,
            boundaries,
            architecture_docs,
            service_links,
            layers: if wants(ArchitectureAspect::Layers) {
                crate::architecture::LayerDetector.detect(self.graph)
            } else {
                Vec::new()
            },
            clusters: if wants(ArchitectureAspect::Clusters) {
                self.clusters()
            } else {
                Vec::new()
            },
            file_tree: if wants(ArchitectureAspect::FileTree) {
                build_file_tree(self.graph)
            } else {
                Vec::new()
            },
        }
    }
}

fn module_language_id(module: &crate::graph::ModuleNode) -> String {
    match module.language {
        crate::graph::ModuleLanguage::Python => "python".to_owned(),
        crate::graph::ModuleLanguage::Rust => "rust".to_owned(),
        crate::graph::ModuleLanguage::TypeScript(language) => language.registry_id().to_owned(),
        crate::graph::ModuleLanguage::SyntaxIndexed(language) => language.registry_id().to_owned(),
    }
}

/// Builds a nested [`FileTreeNode`] tree from every `Artifact` node's path,
/// splitting on `/`. Directories exist only implicitly (as path prefixes
/// shared by multiple artifacts); an empty repository has an empty tree.
fn build_file_tree(graph: &Graph) -> Vec<FileTreeNode> {
    #[derive(Default)]
    struct Builder {
        children: std::collections::BTreeMap<String, Builder>,
        is_file: bool,
    }

    let mut root = Builder::default();
    for node in &graph.nodes {
        let GraphNode::Artifact(artifact) = node else {
            continue;
        };
        let mut cursor = &mut root;
        let components: Vec<&str> = artifact.path.split('/').collect();
        for (index, component) in components.iter().enumerate() {
            cursor = cursor.children.entry((*component).to_owned()).or_default();
            if index == components.len() - 1 {
                cursor.is_file = true;
            }
        }
    }

    fn to_nodes(builder: &Builder, prefix: &str) -> Vec<FileTreeNode> {
        let mut directories = Vec::new();
        let mut files = Vec::new();
        for (name, child) in &builder.children {
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            let node = FileTreeNode {
                name: name.clone(),
                is_directory: !child.is_file,
                children: to_nodes(child, &path),
                path,
            };
            if node.is_directory {
                directories.push(node);
            } else {
                files.push(node);
            }
        }
        directories.into_iter().chain(files).collect()
    }

    to_nodes(&root, "")
}
