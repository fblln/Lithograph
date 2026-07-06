//! Page and generation task manifest: the reproducibility record tying
//! documentation output back to graph dependencies, evidence, and the
//! model/prompt version that produced it.

use crate::domain::EvidenceRef;
use crate::graph::GraphNodeId;
use crate::plan::DocumentationModule;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// One documentation page's dependency and hash record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentationPage {
    /// Stable page identifier.
    pub id: String,
    /// Output path under the configured docs directory.
    pub path: String,
    /// Source module, when this page documents one module.
    pub module_id: Option<String>,
    /// Graph nodes this page's content depends on, for update invalidation.
    pub dependencies: Vec<GraphNodeId>,
    /// Evidence refs actually cited by generated content. Empty until the
    /// page has been generated at least once.
    pub evidence: Vec<EvidenceRef>,
    /// Hash over this page's dependencies, used to detect stale content.
    pub input_hash: String,
    /// Hash of the last written output. `None` until the page is rendered.
    pub output_hash: Option<String>,
}

/// Documentation page category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskKind {
    /// Repository-wide quickstart page.
    Quickstart,
    /// Repository-wide architecture/overview page.
    Architecture,
    /// One module's leaf documentation page.
    ModulePage,
}

/// One bounded model-generation task for a page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GenerationTask {
    /// Stable task identifier.
    pub id: String,
    /// Page category this task generates.
    pub kind: TaskKind,
    /// Page this task produces.
    pub page_id: String,
    /// Hash over the task's inputs, mirrors the page's `input_hash` at
    /// task-creation time so a stored task can detect staleness on its own.
    pub input_hash: String,
    /// Prompt template version used for this task.
    pub prompt_version: String,
    /// Model identifier used for this task.
    pub model: String,
}

/// The complete page/task manifest for one documentation run.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PageManifest {
    /// All planned documentation pages.
    pub pages: Vec<DocumentationPage>,
    /// All planned generation tasks, one per page.
    pub tasks: Vec<GenerationTask>,
}

impl PageManifest {
    /// Renders the manifest as deterministic pretty JSON.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }

    /// Parses a manifest from JSON, as previously written by [`Self::to_json`].
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    /// Returns the IDs of pages whose dependencies intersect `changed_nodes`,
    /// for selective regeneration on `update`.
    pub fn pages_affected_by(&self, changed_nodes: &BTreeSet<GraphNodeId>) -> BTreeSet<String> {
        self.pages
            .iter()
            .filter(|page| {
                page.dependencies
                    .iter()
                    .any(|dependency| changed_nodes.contains(dependency))
            })
            .map(|page| page.id.clone())
            .collect()
    }
}

/// Builds a [`PageManifest`] from a deterministic module plan.
///
/// One [`DocumentationPage`]/[`GenerationTask`] pair is created per module,
/// plus a quickstart and architecture page/task pair that depend on every
/// module (they summarize the whole repository).
#[derive(Debug, Clone, Copy, Default)]
pub struct PageManifestBuilder;

impl PageManifestBuilder {
    /// Builds the manifest for `modules`, stamping every task with
    /// `prompt_version` and `model`.
    pub fn build(
        &self,
        modules: &[DocumentationModule],
        prompt_version: &str,
        model: &str,
    ) -> PageManifest {
        let mut pages = Vec::new();
        let mut tasks = Vec::new();

        for module in modules {
            let page_id = format!("page:module:{}", module.id);
            let slug = module
                .id
                .trim_start_matches("module-plan:")
                .replace(':', "/");
            pages.push(DocumentationPage {
                id: page_id.clone(),
                path: format!("docs/lithograph/modules/{slug}.md"),
                module_id: Some(module.id.clone()),
                dependencies: module.members.clone(),
                evidence: Vec::new(),
                input_hash: module.input_hash.clone(),
                output_hash: None,
            });
            tasks.push(GenerationTask {
                id: format!("task:{page_id}"),
                kind: TaskKind::ModulePage,
                page_id,
                input_hash: module.input_hash.clone(),
                prompt_version: prompt_version.to_owned(),
                model: model.to_owned(),
            });
        }

        let all_dependencies: Vec<GraphNodeId> = modules
            .iter()
            .flat_map(|module| module.members.iter().cloned())
            .collect();
        let repository_hash = repository_input_hash(modules);

        for (kind, id, path) in [
            (
                TaskKind::Quickstart,
                "page:quickstart",
                "docs/lithograph/quickstart.md",
            ),
            (
                TaskKind::Architecture,
                "page:architecture",
                "docs/lithograph/architecture.md",
            ),
        ] {
            pages.push(DocumentationPage {
                id: id.to_owned(),
                path: path.to_owned(),
                module_id: None,
                dependencies: all_dependencies.clone(),
                evidence: Vec::new(),
                input_hash: repository_hash.clone(),
                output_hash: None,
            });
            tasks.push(GenerationTask {
                id: format!("task:{id}"),
                kind,
                page_id: id.to_owned(),
                input_hash: repository_hash.clone(),
                prompt_version: prompt_version.to_owned(),
                model: model.to_owned(),
            });
        }

        pages.sort_by(|a, b| a.id.cmp(&b.id));
        tasks.sort_by(|a, b| a.id.cmp(&b.id));
        PageManifest { pages, tasks }
    }
}

fn repository_input_hash(modules: &[DocumentationModule]) -> String {
    let mut hashes: Vec<&str> = modules
        .iter()
        .map(|module| module.input_hash.as_str())
        .collect();
    hashes.sort_unstable();
    blake3::hash(hashes.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{PageManifest, PageManifestBuilder, TaskKind};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::plan::ModulePlanner;
    use std::collections::BTreeSet;
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn fixture_manifest() -> Result<PageManifest, Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        Ok(PageManifestBuilder.build(&modules, "v1", "mock"))
    }

    #[test]
    fn builds_one_page_per_module_plus_quickstart_and_architecture()
    -> Result<(), Box<dyn std::error::Error>> {
        let manifest = fixture_manifest()?;

        assert!(
            manifest
                .pages
                .iter()
                .any(|page| page.id == "page:quickstart")
        );
        assert!(
            manifest
                .pages
                .iter()
                .any(|page| page.id == "page:architecture")
        );
        let module_pages = manifest
            .pages
            .iter()
            .filter(|page| page.module_id.is_some())
            .count();
        assert_eq!(module_pages, 11);
        assert_eq!(manifest.tasks.len(), manifest.pages.len());
        assert!(
            manifest
                .tasks
                .iter()
                .any(|task| task.kind == TaskKind::Quickstart)
        );

        for page in &manifest.pages {
            assert!(
                !page.dependencies.is_empty(),
                "{} has no dependencies",
                page.id
            );
            assert!(page.output_hash.is_none());
        }
        for task in &manifest.tasks {
            assert_eq!(task.prompt_version, "v1");
            assert_eq!(task.model, "mock");
        }

        Ok(())
    }

    #[test]
    fn manifest_round_trips_through_json() -> Result<(), Box<dyn std::error::Error>> {
        let manifest = fixture_manifest()?;

        let json = manifest.to_json()?;
        let round_tripped = PageManifest::from_json(&json)?;

        assert_eq!(manifest, round_tripped);
        assert!(json.contains("\"prompt_version\": \"v1\""));
        assert!(json.contains("\"kind\": \"ModulePage\""));

        Ok(())
    }

    #[test]
    fn pages_affected_by_maps_changed_nodes_to_owning_pages()
    -> Result<(), Box<dyn std::error::Error>> {
        let manifest = fixture_manifest()?;
        let module_page = manifest
            .pages
            .iter()
            .find(|page| page.module_id.is_some())
            .ok_or("module page")?;
        let changed_node = module_page.dependencies.first().ok_or("dependency")?;

        let changed: BTreeSet<_> = [changed_node.clone()].into_iter().collect();
        let affected = manifest.pages_affected_by(&changed);

        assert!(affected.contains(&module_page.id));
        assert!(affected.contains("page:quickstart"));
        assert!(affected.contains("page:architecture"));

        let unrelated: BTreeSet<_> = [crate::graph::GraphNodeId::new("artifact:does-not-exist")]
            .into_iter()
            .collect();
        assert!(manifest.pages_affected_by(&unrelated).is_empty());

        Ok(())
    }
}
