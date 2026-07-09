//! Grep-like code search over indexed, safe repository files, with
//! graph-aware filters (path, language, graph node, module/package scope)
//! and results carrying snippets, evidence, and matching graph context
//! (LIT-22.4.2).

use crate::domain::{Artifact, EvidenceRef, ModelExposurePolicy, TextStatus};
use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use crate::plan::DocumentationModule;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::Path;

/// Filters for one code search request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CodeSearchParams {
    /// Case-insensitive substring to search for. Empty matches nothing.
    pub query: String,
    /// Case-insensitive substring filter on the artifact path.
    pub path_contains: Option<String>,
    /// Exact match against `Artifact::detected_format` (e.g. `python`, `rust`).
    pub language: Option<String>,
    /// Restrict the search to files that are members of this
    /// [`DocumentationModule`] id.
    pub module_id: Option<String>,
    /// Restrict the search to files belonging to this package name (via
    /// `BelongsToPackage`).
    pub package: Option<String>,
    /// Restrict the search to the file(s) associated with this graph node
    /// id -- either the node's own artifact, or (for a `Symbol`) the
    /// artifact that contains it.
    pub graph_node_id: Option<String>,
    /// Maximum result count. Defaults to 20 when zero.
    pub limit: usize,
}

/// One matching line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodeSearchResult {
    /// Repository-relative artifact path.
    pub artifact_path: String,
    /// One-based matching line number.
    pub line: u32,
    /// The matching line's text, trimmed.
    pub snippet: String,
    /// Evidence pointing at this exact line.
    pub evidence: EvidenceRef,
    /// Qualified names of graph symbols whose evidence span contains this
    /// line, when any resolve.
    pub graph_context: Vec<String>,
}

/// Grep-like search over safe, indexed repository files.
#[derive(Debug, Clone, Copy, Default)]
pub struct CodeSearch;

impl CodeSearch {
    /// Runs one search against `artifacts`/`graph`/`modules`, reading file
    /// content from `repo_root`. Never reads or returns content from an
    /// artifact that isn't safe, plain text (AC3): binary, redacted, or
    /// `ModelExposurePolicy::Never`/`ModelExposurePolicy::Redacted`
    /// artifacts are skipped before their bytes are ever read.
    pub fn search(
        &self,
        repo_root: &Path,
        artifacts: &[Artifact],
        graph: &Graph,
        modules: &[DocumentationModule],
        params: &CodeSearchParams,
    ) -> Vec<CodeSearchResult> {
        let limit = if params.limit == 0 { 20 } else { params.limit };
        if params.query.is_empty() {
            return Vec::new();
        }
        let needle = params.query.to_lowercase();

        let scoped_paths = scoped_artifact_paths(graph, modules, params);
        let symbol_index = SymbolSpanIndex::build(graph);

        let mut results = Vec::new();
        for artifact in artifacts {
            if !is_searchable(artifact) {
                continue;
            }
            if let Some(paths) = &scoped_paths
                && !paths.contains(artifact.path.as_str())
            {
                continue;
            }
            if let Some(path_filter) = &params.path_contains
                && !artifact
                    .path
                    .as_str()
                    .to_lowercase()
                    .contains(&path_filter.to_lowercase())
            {
                continue;
            }
            if let Some(language) = &params.language
                && artifact.detected_format.as_deref() != Some(language.as_str())
            {
                continue;
            }

            let Ok(text) = std::fs::read_to_string(repo_root.join(artifact.path.as_str())) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                if !line.to_lowercase().contains(&needle) {
                    continue;
                }
                let line_number = index as u32 + 1;
                results.push(CodeSearchResult {
                    artifact_path: artifact.path.as_str().to_owned(),
                    line: line_number,
                    snippet: line.trim().to_owned(),
                    evidence: line_evidence(artifact, line_number),
                    graph_context: symbol_index.containing(artifact.path.as_str(), line_number),
                });
                if results.len() >= limit {
                    return results;
                }
            }
        }
        results
    }
}

/// True when `artifact` is safe to read and expose in search results
/// (AC3): plain text, and not policy-excluded from model/tool exposure.
fn is_searchable(artifact: &Artifact) -> bool {
    artifact.text_status == TextStatus::Text
        && !matches!(
            artifact.model_policy,
            ModelExposurePolicy::Never | ModelExposurePolicy::Redacted
        )
}

fn line_evidence(artifact: &Artifact, line: u32) -> EvidenceRef {
    let base = EvidenceRef::file(
        crate::domain::ArtifactId::from_path(&artifact.path),
        artifact.path.clone(),
    );
    match crate::domain::SourceSpan::new(line, line) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

/// Computes the artifact-path scope implied by `params.module_id`,
/// `params.package`, and `params.graph_node_id` (AC1), intersected
/// together when more than one is set. `None` means "no scope filter."
fn scoped_artifact_paths(
    graph: &Graph,
    modules: &[DocumentationModule],
    params: &CodeSearchParams,
) -> Option<BTreeSet<String>> {
    let mut scopes: Vec<BTreeSet<String>> = Vec::new();

    if let Some(module_id) = &params.module_id {
        let module = modules.iter().find(|module| &module.id == module_id);
        let paths: BTreeSet<String> = module
            .map(|module| module.members.iter())
            .into_iter()
            .flatten()
            .filter_map(|member| artifact_path_for(graph, member))
            .collect();
        scopes.push(paths);
    }

    if let Some(package) = &params.package {
        let package_id = graph.nodes.iter().find_map(|node| match node {
            GraphNode::Package(node) if &node.name == package => Some(&node.id),
            _ => None,
        });
        let paths: BTreeSet<String> = package_id
            .map(|package_id| {
                graph
                    .relations
                    .iter()
                    .filter(|relation| {
                        relation.kind == RelationKind::BelongsToPackage
                            && &relation.target == package_id
                    })
                    .filter_map(|relation| artifact_path_for(graph, &relation.source))
                    .collect()
            })
            .unwrap_or_default();
        scopes.push(paths);
    }

    if let Some(graph_node_id) = &params.graph_node_id {
        let id = GraphNodeId::new(graph_node_id.clone());
        let paths: BTreeSet<String> = artifact_path_for(graph, &id).into_iter().collect();
        scopes.push(paths);
    }

    scopes
        .into_iter()
        .reduce(|a, b| a.intersection(&b).cloned().collect())
}

/// Resolves any graph node id to the repository-relative path of the
/// artifact it lives in: an `Artifact` node resolves to itself; any other
/// node kind resolves via the artifact that `Contains` it, when one exists.
fn artifact_path_for(graph: &Graph, id: &GraphNodeId) -> Option<String> {
    for node in &graph.nodes {
        if let GraphNode::Artifact(artifact) = node
            && artifact.id == *id
        {
            return Some(artifact.path.clone());
        }
    }
    graph
        .relations
        .iter()
        .filter(|relation| relation.kind == RelationKind::Contains && relation.target == *id)
        .find_map(|relation| artifact_path_for(graph, &relation.source))
}

/// Precomputed index from `(artifact_path, line)` to the qualified names
/// of every symbol whose evidence span covers that line, so `search`
/// doesn't rescan the whole graph per matching line.
struct SymbolSpanIndex {
    entries: Vec<(String, u32, u32, String)>,
}

impl SymbolSpanIndex {
    fn build(graph: &Graph) -> Self {
        let entries = graph
            .nodes
            .iter()
            .filter_map(|node| {
                let GraphNode::Symbol(symbol) = node else {
                    return None;
                };
                let span = symbol.evidence.span.as_ref()?;
                Some((
                    symbol.evidence.path.as_str().to_owned(),
                    span.start_line,
                    span.end_line,
                    symbol.qualified_name.clone(),
                ))
            })
            .collect();
        Self { entries }
    }

    fn containing(&self, path: &str, line: u32) -> Vec<String> {
        let mut names: Vec<String> = self
            .entries
            .iter()
            .filter(|(entry_path, start, end, _)| {
                entry_path == path && *start <= line && line <= *end
            })
            .map(|(.., name)| name.clone())
            .collect();
        names.sort();
        names.dedup();
        names
    }
}

#[cfg(test)]
mod tests {
    use super::{CodeSearch, CodeSearchParams};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::plan::ModulePlanner;
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    /// LIT-22.4.2 AC1/AC2: a plain query returns snippets, evidence, and
    /// graph context (the containing symbol's qualified name).
    #[test]
    fn search_returns_snippets_evidence_and_graph_context() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "class RouteService".to_owned(),
                ..Default::default()
            },
        );

        let hit = results.first().ok_or("expected at least one result")?;
        assert_eq!(hit.artifact_path, "src/python_app/service.py");
        assert!(hit.snippet.contains("RouteService"));
        assert!(hit.evidence.span.is_some());
        assert!(!hit.graph_context.is_empty());

        Ok(())
    }

    /// LIT-22.4.2 AC1: a path filter narrows results to matching artifacts.
    #[test]
    fn path_filter_narrows_results() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "pub fn".to_owned(),
                path_contains: Some("rust/".to_owned()),
                ..Default::default()
            },
        );

        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.artifact_path.starts_with("rust/"))
        );

        Ok(())
    }

    /// LIT-22.4.2 AC1: a language filter restricts to one detected format.
    #[test]
    fn language_filter_restricts_to_one_format() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "e".to_owned(),
                language: Some("rust".to_owned()),
                limit: 100,
                ..Default::default()
            },
        );

        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.artifact_path.ends_with(".rs"))
        );

        Ok(())
    }

    /// LIT-22.4.2 AC1: a module scope restricts results to that module's
    /// member artifacts only.
    #[test]
    fn module_scope_restricts_to_member_artifacts() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let python_module = modules
            .iter()
            .find(|module| module.name.contains("python"))
            .ok_or("expected a python module")?;

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "import".to_owned(),
                module_id: Some(python_module.id.clone()),
                limit: 100,
                ..Default::default()
            },
        );

        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.artifact_path.starts_with("src/python_app"))
        );

        Ok(())
    }

    /// LIT-22.4.2 AC1: scoping the search to a graph node id restricts it
    /// to that node's own artifact.
    #[test]
    fn graph_node_scope_restricts_to_its_artifact() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "e".to_owned(),
                graph_node_id: Some("artifact:rust/src/lib.rs".to_owned()),
                limit: 100,
                ..Default::default()
            },
        );

        assert!(!results.is_empty());
        assert!(
            results
                .iter()
                .all(|result| result.artifact_path == "rust/src/lib.rs")
        );

        Ok(())
    }

    /// LIT-22.4.2 AC3: a binary artifact's content is never read or
    /// exposed, even if the raw bytes happen to contain the query text.
    #[test]
    fn unsafe_artifacts_are_never_searched() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "a".to_owned(),
                limit: 1000,
                ..Default::default()
            },
        );

        assert!(
            !results
                .iter()
                .any(|result| result.artifact_path == "data/sample.bin")
        );

        Ok(())
    }

    /// LIT-22.4.2 AC4: an empty query returns no results rather than
    /// matching every line.
    #[test]
    fn empty_query_returns_no_results() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams::default(),
        );

        assert!(results.is_empty());

        Ok(())
    }

    /// LIT-22.4.2 AC4: the result count never exceeds the requested limit.
    #[test]
    fn limit_bounds_result_count() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let results = CodeSearch.search(
            &root,
            &artifacts,
            &graph,
            &modules,
            &CodeSearchParams {
                query: "e".to_owned(),
                limit: 3,
                ..Default::default()
            },
        );

        assert_eq!(results.len(), 3);

        Ok(())
    }
}
