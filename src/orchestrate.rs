//! `init` orchestration: wires scan, analysis, graph, module planning,
//! generation, evidence validation, and writes into one product loop.

use crate::adr::AdrStore;
use crate::analysis::AnalysisCache;
use crate::domain::{Artifact, EvidenceRef};
use crate::drift::DriftDetector;
use crate::generation::{
    ArchitectureViewContext, ContextBuilder, LanguageModel, ModelError, PageRenderer, RenderError,
};
use crate::graph::{Graph, GraphBuilder, GraphIssue, GraphStore, GraphValidator};
use crate::inventory::{RepositoryWalker, WalkError, WalkOptions};
use crate::manifest::{
    DocumentationPage, GenerationTask, PageManifest, PageManifestBuilder, TaskKind,
};
use crate::plan::{DocumentationModule, ModulePlanner};
use crate::research::{AgentMemoryIndex, ResearchBrief, ResearchBuilder};
use crate::run::{
    PipelineInvalidationMetadata, PipelineStage, RepositorySnapshot, RunMetadata, RunMetadataInput,
    StageTiming,
};
use crate::storage::JsonStore;
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Runs one pipeline stage, recording its wall-clock duration into `timings`
/// and tagging any error with which stage produced it (LIT-22.6.1 AC1/AC3).
fn timed_stage<T>(
    stage: PipelineStage,
    timings: &mut Vec<StageTiming>,
    run: impl FnOnce() -> Result<T, InitError>,
) -> Result<T, InitError> {
    let started = Instant::now();
    let result = run();
    timings.push(StageTiming {
        stage,
        duration_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    });
    result.map_err(|error| InitError::Stage(stage, Box::new(error)))
}

/// Exclude patterns shared by every scan: Lithograph's own output must
/// never become input to its next run, or every run would document (and
/// hash-invalidate on) the previous run's generated pages and metadata.
pub(crate) fn scan_exclude_globs() -> Vec<String> {
    vec!["docs/lithograph/**".to_owned(), ".lithograph/**".to_owned()]
}

/// Runs the shared scan -> graph -> validate -> plan pipeline used by both
/// `init` and `update`. `cache` lets an artifact whose content hash was seen
/// on an earlier run (by either command) skip a fresh read+parse.
fn scan_and_plan(
    repo_root: &Path,
    cache: Option<&AnalysisCache>,
    semantic_grouping: bool,
) -> Result<(Vec<Artifact>, Graph, Vec<DocumentationModule>), InitError> {
    let walk_options = WalkOptions {
        exclude_globs: scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(repo_root)?;
    let graph = GraphBuilder.build_with_cache(repo_root, &artifacts, cache);

    let issues = GraphValidator.validate(&graph, &artifacts);
    if !issues.is_empty() {
        return Err(InitError::GraphInvalid(issues));
    }

    let modules = if semantic_grouping {
        ModulePlanner.plan_with_semantic_grouping(&graph, &artifacts)
    } else {
        ModulePlanner.plan(&graph, &artifacts)
    };
    Ok((artifacts, graph, modules))
}

/// Directory holding cached per-artifact analyzer output, keyed by content
/// hash. Lives under `.lithograph/`, already excluded from every scan by
/// [`scan_exclude_globs`].
fn analysis_cache(repo_root: &Path) -> AnalysisCache {
    AnalysisCache::new(repo_root.join(".lithograph/cache/analysis"))
}

/// Scans drift and loads ADR summaries for the architecture page only
/// (LIT-22.7.1): every other page kind skips this entirely, since it's
/// only worth the filesystem scan when actually building that one page.
fn architecture_view_context(
    task_kind: TaskKind,
    artifacts: &[Artifact],
    graph: &Graph,
    repo_root: &Path,
) -> Option<ArchitectureViewContext> {
    if task_kind != TaskKind::Architecture {
        return None;
    }
    Some(ArchitectureViewContext {
        drift: DriftDetector.scan(artifacts, graph, repo_root),
        adr_summaries: AdrStore::new(repo_root).list(),
    })
}

/// Counts and output paths from one `init` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitReport {
    /// Number of repository artifacts discovered.
    pub artifact_count: usize,
    /// Number of graph nodes built.
    pub graph_node_count: usize,
    /// Number of graph relations built.
    pub graph_relation_count: usize,
    /// Number of deterministic documentation modules planned.
    pub module_count: usize,
    /// Number of documentation pages planned.
    pub page_count: usize,
    /// Number of pages whose content actually changed and were written.
    pub pages_written: usize,
    /// Number of artifacts changed since the previous run (all of them, on
    /// a first run).
    pub changed_artifact_count: usize,
    /// Number of artifacts actually read and reparsed this run -- the rest
    /// were served from the analysis cache by unchanged content hash.
    pub artifacts_reanalyzed_count: usize,
    /// Path written for the graph export.
    pub graph_path: PathBuf,
    /// Path written for the page manifest.
    pub manifest_path: PathBuf,
    /// Path written for this run's metadata.
    pub run_metadata_path: PathBuf,
}

/// Error from an `init` run. No partial output is left validated as
/// correct: a graph or evidence failure stops the run before any further
/// page is written.
#[derive(Debug)]
pub enum InitError {
    /// Repository scan failed.
    Walk(WalkError),
    /// The built graph failed validation before any generation began.
    GraphInvalid(Vec<GraphIssue>),
    /// A model request failed.
    Model(ModelError),
    /// A generated page failed evidence validation or could not be written.
    Render(RenderError),
    /// Failed to serialize graph/manifest JSON.
    Json(serde_json::Error),
    /// Filesystem I/O failure.
    Io(std::io::Error),
    /// An internal consistency invariant (page/task/module correspondence) broke.
    Message(String),
    /// An error occurred within a specific pipeline stage (LIT-22.6.1 AC3).
    Stage(PipelineStage, Box<InitError>),
}

impl Display for InitError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Walk(error) => write!(formatter, "repository scan failed: {error}"),
            Self::GraphInvalid(issues) => {
                writeln!(
                    formatter,
                    "graph validation failed with {} issue(s):",
                    issues.len()
                )?;
                for issue in issues {
                    writeln!(formatter, "  - {issue}")?;
                }
                Ok(())
            }
            Self::Model(error) => write!(formatter, "model request failed: {error}"),
            Self::Render(error) => write!(formatter, "{error}"),
            Self::Json(error) => write!(formatter, "failed to serialize output: {error}"),
            Self::Io(error) => write!(formatter, "failed to write output: {error}"),
            Self::Message(message) => formatter.write_str(message),
            Self::Stage(stage, error) => write!(formatter, "{stage:?} stage failed: {error}"),
        }
    }
}

impl std::error::Error for InitError {}

impl From<WalkError> for InitError {
    fn from(error: WalkError) -> Self {
        Self::Walk(error)
    }
}

impl From<serde_json::Error> for InitError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

impl From<std::io::Error> for InitError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// Runs the full `init` pipeline against `repo_root`, generating pages with
/// `model` and writing `.lithograph/graph.json` and `.lithograph/manifest.json`.
pub fn run_init(
    repo_root: &Path,
    model: &dyn LanguageModel,
    model_name: &str,
    prompt_version: &str,
) -> Result<InitReport, InitError> {
    run_init_with_options(repo_root, model, model_name, prompt_version, false)
}

/// Runs the full `init` pipeline with explicit planning options.
pub fn run_init_with_options(
    repo_root: &Path,
    model: &dyn LanguageModel,
    model_name: &str,
    prompt_version: &str,
    semantic_grouping: bool,
) -> Result<InitReport, InitError> {
    let cache = analysis_cache(repo_root);
    let mut stage_timings: Vec<StageTiming> = Vec::new();

    let (artifacts, graph, modules) =
        timed_stage(PipelineStage::PreprocessIndex, &mut stage_timings, || {
            scan_and_plan(repo_root, Some(&cache), semantic_grouping)
        })?;

    let research = timed_stage(PipelineStage::Research, &mut stage_timings, || {
        Ok(ResearchBuilder.build(&artifacts, &graph, &modules))
    })?;

    let (manifest, written_pages) =
        timed_stage(PipelineStage::Compose, &mut stage_timings, || {
            let modules_by_id: BTreeMap<&str, &DocumentationModule> = modules
                .iter()
                .map(|module| (module.id.as_str(), module))
                .collect();

            let mut manifest: PageManifest =
                PageManifestBuilder.build(&modules, prompt_version, model_name);
            let tasks = manifest.tasks.clone();
            let mut written_pages: Vec<String> = Vec::new();

            for task in &tasks {
                let context = match task.kind {
                    TaskKind::ModulePage => {
                        let page = manifest
                            .pages
                            .iter()
                            .find(|page| page.id == task.page_id)
                            .ok_or_else(|| {
                                InitError::Message(format!("no page planned for task {}", task.id))
                            })?;
                        let module_id = page.module_id.as_deref().ok_or_else(|| {
                            InitError::Message(format!("module page {} has no module_id", page.id))
                        })?;
                        let module = modules_by_id.get(module_id).ok_or_else(|| {
                            InitError::Message(format!("no planned module {module_id}"))
                        })?;
                        ContextBuilder.build_module_context(module, &graph, &artifacts, repo_root)
                    }
                    _ => ContextBuilder.build_summary_context(
                        task.kind,
                        &modules,
                        &graph,
                        &artifacts,
                        Some(&research),
                        architecture_view_context(task.kind, &artifacts, &graph, repo_root)
                            .as_ref(),
                    ),
                };

                let request = context.clone().into_request(model_name, prompt_version);
                let generation = model.generate_json(&request).map_err(InitError::Model)?;

                let page = manifest
                    .pages
                    .iter_mut()
                    .find(|page| page.id == task.page_id)
                    .ok_or_else(|| {
                        InitError::Message(format!("no page planned for task {}", task.id))
                    })?;
                let outcome = PageRenderer
                    .render_and_write(page, &generation, &context, repo_root)
                    .map_err(InitError::Render)?;
                if outcome.written {
                    written_pages.push(task.page_id.clone());
                }
            }

            Ok((manifest, written_pages))
        })?;

    let stage_started = Instant::now();
    let validate_output = (|| -> Result<InitReport, InitError> {
        let lithograph_dir = repo_root.join(".lithograph");
        let snapshot_path = lithograph_dir.join("snapshot.json");
        let previous_snapshot: Option<RepositorySnapshot> = JsonStore.read(&snapshot_path)?;

        let (mut run_metadata, snapshot) = RunMetadata::compute(RunMetadataInput {
            command: "init",
            repo_root,
            artifacts: &artifacts,
            graph: &graph,
            manifest: &manifest,
            written_pages: &written_pages,
            previous_snapshot: previous_snapshot.as_ref(),
            pipeline: PipelineInvalidationMetadata::current(prompt_version, semantic_grouping),
        })?;

        let graph_store_outcome = GraphStore::new(repo_root).save(&graph)?;
        let graph_path = graph_store_outcome.legacy_graph_path;
        write_research_artifacts(&lithograph_dir, &research)?;
        let manifest_path = lithograph_dir.join("manifest.json");
        JsonStore.write_if_changed(&manifest_path, &manifest)?;
        JsonStore.write_if_changed(&snapshot_path, &snapshot)?;
        // run.json is an append-style record of this run's own facts
        // (including a fresh run_id and this run's stage timings), so it is
        // always (re)written, unlike the state files above which stay
        // byte-stable across a true no-op run (AC2).
        stage_timings.push(StageTiming {
            stage: PipelineStage::ValidateOutput,
            duration_ms: u64::try_from(stage_started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        run_metadata.stage_timings = stage_timings.clone();
        let run_metadata_path = lithograph_dir.join("run.json");
        JsonStore.write(&run_metadata_path, &run_metadata)?;

        Ok(InitReport {
            artifact_count: artifacts.len(),
            graph_node_count: graph.nodes.len(),
            graph_relation_count: graph.relations.len(),
            module_count: modules.len(),
            page_count: manifest.pages.len(),
            pages_written: written_pages.len(),
            changed_artifact_count: run_metadata.changed_artifacts.len(),
            artifacts_reanalyzed_count: cache.misses(),
            graph_path,
            manifest_path,
            run_metadata_path,
        })
    })();
    validate_output
        .map_err(|error| InitError::Stage(PipelineStage::ValidateOutput, Box::new(error)))
}

/// Counts and output paths from one `update` run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReport {
    /// Number of repository artifacts discovered on this rescan.
    pub artifact_count: usize,
    /// Number of graph nodes built.
    pub graph_node_count: usize,
    /// Number of graph relations built.
    pub graph_relation_count: usize,
    /// Number of deterministic documentation modules planned.
    pub module_count: usize,
    /// Number of documentation pages planned.
    pub page_count: usize,
    /// Number of pages actually regenerated this run.
    pub pages_regenerated: usize,
    /// Number of artifacts changed since the previous run.
    pub changed_artifact_count: usize,
    /// Number of artifacts actually read and reparsed this run -- the rest
    /// were served from the analysis cache by unchanged content hash.
    pub artifacts_reanalyzed_count: usize,
    /// Path written for the graph export.
    pub graph_path: PathBuf,
    /// Path written for the page manifest.
    pub manifest_path: PathBuf,
    /// Path written for this run's metadata.
    pub run_metadata_path: PathBuf,
}

/// Runs `update`: a full rescan (same as `init`), but only regenerates
/// pages whose planning-time `input_hash` changed from the previous run
/// (a page with no previous entry, e.g. a brand-new module, always counts
/// as changed). `input_hash` is itself a hash over a page's dependencies'
/// content, so comparing it before/after is a complete, self-contained way
/// to detect "dependencies or ancestors changed" (AC2): repository-level
/// pages depend on every module's members, so any module change already
/// changes their `input_hash` too, with no separate ancestor-walk needed.
/// Pages that do not need regeneration keep their previous evidence/output_hash
/// untouched and their file is never rewritten, so an unrelated page's mtime
/// never changes.
///
/// Falls back to full generation (like `init`) when no previous manifest
/// exists yet.
pub fn run_update(
    repo_root: &Path,
    model: &dyn LanguageModel,
    model_name: &str,
    prompt_version: &str,
) -> Result<UpdateReport, InitError> {
    run_update_with_options(repo_root, model, model_name, prompt_version, false)
}

/// Runs `update` with explicit planning options.
pub fn run_update_with_options(
    repo_root: &Path,
    model: &dyn LanguageModel,
    model_name: &str,
    prompt_version: &str,
    semantic_grouping: bool,
) -> Result<UpdateReport, InitError> {
    let lithograph_dir = repo_root.join(".lithograph");
    let manifest_path = lithograph_dir.join("manifest.json");
    let snapshot_path = lithograph_dir.join("snapshot.json");
    let cache = analysis_cache(repo_root);
    let mut stage_timings: Vec<StageTiming> = Vec::new();

    let (artifacts, graph, modules, previous_manifest, previous_snapshot) =
        timed_stage(PipelineStage::PreprocessIndex, &mut stage_timings, || {
            let previous_manifest: Option<PageManifest> = JsonStore.read(&manifest_path)?;
            let previous_snapshot: Option<RepositorySnapshot> = JsonStore.read(&snapshot_path)?;
            let (artifacts, graph, modules) =
                scan_and_plan(repo_root, Some(&cache), semantic_grouping)?;
            Ok((
                artifacts,
                graph,
                modules,
                previous_manifest,
                previous_snapshot,
            ))
        })?;
    let previous_pages_by_id: BTreeMap<&str, &DocumentationPage> = previous_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .pages
                .iter()
                .map(|page| (page.id.as_str(), page))
                .collect()
        })
        .unwrap_or_default();
    let previous_tasks_by_page_id: BTreeMap<&str, &GenerationTask> = previous_manifest
        .as_ref()
        .map(|manifest| {
            manifest
                .tasks
                .iter()
                .map(|task| (task.page_id.as_str(), task))
                .collect()
        })
        .unwrap_or_default();

    let research = timed_stage(PipelineStage::Research, &mut stage_timings, || {
        Ok(ResearchBuilder.build(&artifacts, &graph, &modules))
    })?;

    let (manifest, written_pages) =
        timed_stage(PipelineStage::Compose, &mut stage_timings, || {
            let modules_by_id: BTreeMap<&str, &DocumentationModule> = modules
                .iter()
                .map(|module| (module.id.as_str(), module))
                .collect();

            let mut manifest: PageManifest =
                PageManifestBuilder.build(&modules, prompt_version, model_name);
            let tasks = manifest.tasks.clone();
            let mut written_pages: Vec<String> = Vec::new();

            for task in &tasks {
                let current_input_hash = manifest
                    .pages
                    .iter()
                    .find(|page| page.id == task.page_id)
                    .ok_or_else(|| {
                        InitError::Message(format!("no page planned for task {}", task.id))
                    })?
                    .input_hash
                    .clone();
                let previous_page = previous_pages_by_id.get(task.page_id.as_str());
                let previous_task = previous_tasks_by_page_id.get(task.page_id.as_str());
                let needs_regeneration = previous_page
                    .is_none_or(|previous| previous.input_hash != current_input_hash)
                    || previous_task
                        .is_none_or(|previous| !task.is_version_compatible_with(previous));

                if !needs_regeneration {
                    if let Some(previous) = previous_page {
                        let page = manifest
                            .pages
                            .iter_mut()
                            .find(|page| page.id == task.page_id)
                            .ok_or_else(|| {
                                InitError::Message(format!("no page planned for task {}", task.id))
                            })?;
                        page.evidence = previous.evidence.clone();
                        page.output_hash = previous.output_hash.clone();
                    }
                    continue;
                }

                let context = match task.kind {
                    TaskKind::ModulePage => {
                        let page = manifest
                            .pages
                            .iter()
                            .find(|page| page.id == task.page_id)
                            .ok_or_else(|| {
                                InitError::Message(format!("no page planned for task {}", task.id))
                            })?;
                        let module_id = page.module_id.as_deref().ok_or_else(|| {
                            InitError::Message(format!("module page {} has no module_id", page.id))
                        })?;
                        let module = modules_by_id.get(module_id).ok_or_else(|| {
                            InitError::Message(format!("no planned module {module_id}"))
                        })?;
                        ContextBuilder.build_module_context(module, &graph, &artifacts, repo_root)
                    }
                    _ => ContextBuilder.build_summary_context(
                        task.kind,
                        &modules,
                        &graph,
                        &artifacts,
                        Some(&research),
                        architecture_view_context(task.kind, &artifacts, &graph, repo_root)
                            .as_ref(),
                    ),
                };

                let request = context.clone().into_request(model_name, prompt_version);
                let generation = model.generate_json(&request).map_err(InitError::Model)?;

                let page = manifest
                    .pages
                    .iter_mut()
                    .find(|page| page.id == task.page_id)
                    .ok_or_else(|| {
                        InitError::Message(format!("no page planned for task {}", task.id))
                    })?;
                let outcome = PageRenderer
                    .render_and_write(page, &generation, &context, repo_root)
                    .map_err(InitError::Render)?;
                if outcome.written {
                    written_pages.push(task.page_id.clone());
                }
            }

            Ok((manifest, written_pages))
        })?;

    let stage_started = Instant::now();
    let validate_output = (|| -> Result<UpdateReport, InitError> {
        let (mut run_metadata, snapshot) = RunMetadata::compute(RunMetadataInput {
            command: "update",
            repo_root,
            artifacts: &artifacts,
            graph: &graph,
            manifest: &manifest,
            written_pages: &written_pages,
            previous_snapshot: previous_snapshot.as_ref(),
            pipeline: PipelineInvalidationMetadata::current(prompt_version, semantic_grouping),
        })?;

        let graph_store_outcome = GraphStore::new(repo_root).save(&graph)?;
        let graph_path = graph_store_outcome.legacy_graph_path;
        write_research_artifacts(&lithograph_dir, &research)?;
        JsonStore.write_if_changed(&manifest_path, &manifest)?;
        JsonStore.write_if_changed(&snapshot_path, &snapshot)?;
        stage_timings.push(StageTiming {
            stage: PipelineStage::ValidateOutput,
            duration_ms: u64::try_from(stage_started.elapsed().as_millis()).unwrap_or(u64::MAX),
        });
        run_metadata.stage_timings = stage_timings.clone();
        let run_metadata_path = lithograph_dir.join("run.json");
        JsonStore.write(&run_metadata_path, &run_metadata)?;

        Ok(UpdateReport {
            artifact_count: artifacts.len(),
            graph_node_count: graph.nodes.len(),
            graph_relation_count: graph.relations.len(),
            module_count: modules.len(),
            page_count: manifest.pages.len(),
            pages_regenerated: written_pages.len(),
            changed_artifact_count: run_metadata.changed_artifacts.len(),
            artifacts_reanalyzed_count: cache.misses(),
            graph_path,
            manifest_path,
            run_metadata_path,
        })
    })();
    validate_output
        .map_err(|error| InitError::Stage(PipelineStage::ValidateOutput, Box::new(error)))
}

/// Returns every evidence ref recorded across the written manifest's pages,
/// for callers that want to report on generated evidence coverage.
pub fn manifest_evidence(manifest_path: &Path) -> Result<Vec<EvidenceRef>, InitError> {
    let json = std::fs::read_to_string(manifest_path)?;
    let manifest = PageManifest::from_json(&json)?;
    Ok(manifest
        .pages
        .into_iter()
        .flat_map(|page| page.evidence)
        .collect())
}

fn write_research_artifacts(
    lithograph_dir: &Path,
    research: &ResearchBrief,
) -> Result<(), InitError> {
    let research_dir = lithograph_dir.join("research");
    JsonStore.write_if_changed(&research_dir.join("brief.json"), research)?;
    AgentMemoryIndex::new(research.agent_memory.clone(), research.input_hash.clone())
        .persist(&research_dir)?;
    JsonStore.write_if_changed(
        &research_dir.join("configuration.json"),
        &research.configuration,
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{InitError, run_init, run_update};
    use crate::generation::{LanguageModel, MockModel, ModelError, ModelRequest, PageGeneration};
    use crate::run::PipelineStage;
    use std::path::Path;

    /// Always fails, so tests can assert `Compose`-stage error attribution
    /// (LIT-22.6.1 AC3) without needing a real model failure.
    struct FailingModel;

    impl LanguageModel for FailingModel {
        fn generate_text(&self, _request: &ModelRequest) -> Result<String, ModelError> {
            Err(ModelError {
                message: "synthetic failure".to_owned(),
            })
        }

        fn generate_json(&self, _request: &ModelRequest) -> Result<PageGeneration, ModelError> {
            Err(ModelError {
                message: "synthetic failure".to_owned(),
            })
        }
    }

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    #[test]
    fn init_writes_repository_and_module_pages_graph_and_manifest()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;

        let report = run_init(temp.path(), &MockModel, "mock", "v1")?;

        assert!(report.artifact_count > 0);
        assert!(report.graph_node_count > 0);
        assert!(report.graph_relation_count > 0);
        assert_eq!(report.module_count, 11);
        assert_eq!(report.page_count, report.module_count + 6);
        assert_eq!(report.pages_written, report.page_count);

        assert!(temp.path().join("docs/lithograph/overview.md").exists());
        assert!(temp.path().join("docs/lithograph/quickstart.md").exists());
        assert!(temp.path().join("docs/lithograph/architecture.md").exists());
        assert!(temp.path().join("docs/lithograph/workflows.md").exists());
        assert!(temp.path().join("docs/lithograph/boundaries.md").exists());
        assert!(
            temp.path()
                .join("docs/lithograph/configuration.md")
                .exists()
        );
        assert!(temp.path().join(".lithograph/graph.json").exists());
        assert!(temp.path().join(".lithograph/graph/current.json").exists());
        assert!(temp.path().join(".lithograph/manifest.json").exists());

        let manifest_json = std::fs::read_to_string(temp.path().join(".lithograph/manifest.json"))?;
        assert!(manifest_json.contains("\"output_hash\""));

        Ok(())
    }

    /// LIT-22.6.1 AC1: every run persists all four stages' timing to
    /// `run.json`, in pipeline order.
    #[test]
    fn run_json_persists_all_four_stage_timings() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;

        let report = run_init(temp.path(), &MockModel, "mock", "v1")?;
        let run_json = std::fs::read_to_string(&report.run_metadata_path)?;
        let run_metadata: serde_json::Value = serde_json::from_str(&run_json)?;
        let stages: Vec<&str> = run_metadata["stage_timings"]
            .as_array()
            .ok_or("missing stage_timings array")?
            .iter()
            .map(|entry| entry["stage"].as_str().unwrap_or(""))
            .collect();
        assert_eq!(
            stages,
            vec!["PreprocessIndex", "Research", "Compose", "ValidateOutput"]
        );

        Ok(())
    }

    /// LIT-22.6.1 AC3/AC4: a failure inside the compose stage (a failing
    /// model) is tagged with `PipelineStage::Compose`, and no run.json,
    /// manifest, or graph file is written for the failed run (partial
    /// output never looks like a completed one).
    #[test]
    fn model_failure_is_tagged_with_the_compose_stage_and_writes_nothing()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;

        match run_init(temp.path(), &FailingModel, "mock", "v1") {
            Ok(_) => return Err("expected a Compose-stage failure".into()),
            Err(InitError::Stage(PipelineStage::Compose, inner)) => {
                assert!(matches!(*inner, InitError::Model(_)));
            }
            Err(other) => {
                return Err(format!("expected a Compose-stage error, got: {other}").into());
            }
        }
        assert!(!temp.path().join(".lithograph/manifest.json").exists());
        assert!(!temp.path().join(".lithograph/run.json").exists());
        assert!(!temp.path().join(".lithograph/graph.json").exists());

        Ok(())
    }

    #[test]
    fn init_and_update_never_touch_agent_instruction_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;
        let agents_md = "# Agent instructions\n\nBe helpful.\n";
        let claude_md = "# Claude instructions\n\nFollow project conventions.\n";
        std::fs::write(temp.path().join("AGENTS.md"), agents_md)?;
        std::fs::write(temp.path().join("CLAUDE.md"), claude_md)?;

        run_init(temp.path(), &MockModel, "mock", "v1")?;
        run_update(temp.path(), &MockModel, "mock", "v1")?;

        assert_eq!(
            std::fs::read_to_string(temp.path().join("AGENTS.md"))?,
            agents_md
        );
        assert_eq!(
            std::fs::read_to_string(temp.path().join("CLAUDE.md"))?,
            claude_md
        );

        Ok(())
    }

    #[test]
    fn init_is_a_no_op_write_on_the_second_run() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;

        let first = run_init(temp.path(), &MockModel, "mock", "v1")?;
        let second = run_init(temp.path(), &MockModel, "mock", "v1")?;

        assert_eq!(first.pages_written, first.page_count);
        assert_eq!(second.pages_written, 0);

        Ok(())
    }

    #[test]
    fn update_with_no_previous_run_does_full_generation() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;

        let report = run_update(temp.path(), &MockModel, "mock", "v1")?;

        assert_eq!(report.pages_regenerated, report.page_count);
        assert_eq!(report.changed_artifact_count, report.artifact_count);
        assert!(temp.path().join("docs/lithograph/quickstart.md").exists());

        Ok(())
    }

    #[test]
    fn update_with_no_changes_regenerates_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        let report = run_update(temp.path(), &MockModel, "mock", "v1")?;

        assert_eq!(report.pages_regenerated, 0);
        assert_eq!(report.changed_artifact_count, 0);
        assert_eq!(report.artifacts_reanalyzed_count, 0);

        Ok(())
    }

    #[test]
    fn update_regenerates_when_prompt_version_changes_without_source_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        let report = super::run_update(temp.path(), &MockModel, "mock", "v2")?;
        let manifest_json = std::fs::read_to_string(temp.path().join(".lithograph/manifest.json"))?;

        assert_eq!(report.changed_artifact_count, report.artifact_count);
        assert_eq!(report.artifacts_reanalyzed_count, 0);
        assert_eq!(report.pages_regenerated, report.page_count);
        assert!(manifest_json.contains("\"prompt_version\": \"v2\""));
        assert!(manifest_json.contains("\"context_schema_version\""));

        Ok(())
    }

    #[test]
    fn one_file_change_regenerates_only_its_module_and_the_summary_pages()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(&fixture_root(), temp.path())?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        let rust_page = temp
            .path()
            .join("docs/lithograph/modules/rust-crate/fixture-worker.md");
        let python_page = temp
            .path()
            .join("docs/lithograph/modules/python-package/python-app.md");
        let quickstart_page = temp.path().join("docs/lithograph/quickstart.md");
        assert!(rust_page.exists());
        assert!(python_page.exists());
        let rust_hash_before = std::fs::read_to_string(&rust_page)?;
        let python_hash_before = std::fs::read_to_string(&python_page)?;
        let quickstart_before = std::fs::read_to_string(&quickstart_page)?;

        let lib_rs = temp.path().join("rust/src/lib.rs");
        let mut source = std::fs::read_to_string(&lib_rs)?;
        source.push_str("\n// a deliberate one-file change\n");
        std::fs::write(&lib_rs, source)?;

        let report = run_update(temp.path(), &MockModel, "mock", "v1")?;

        assert_eq!(report.changed_artifact_count, 1);
        assert_eq!(report.artifacts_reanalyzed_count, 1);
        assert_eq!(std::fs::read_to_string(&python_page)?, python_hash_before);
        assert_ne!(std::fs::read_to_string(&rust_page)?, rust_hash_before);
        assert_ne!(
            std::fs::read_to_string(&quickstart_page)?,
            quickstart_before
        );
        // Rust crate page + every repository-level page regenerates; the
        // unrelated Python package page does not.
        assert_eq!(report.pages_regenerated, 7);

        Ok(())
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for entry in walk_files(from)? {
            let relative = entry.strip_prefix(from)?;
            let destination = to.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry, &destination)?;
        }
        Ok(())
    }

    fn walk_files(root: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        Ok(files)
    }
}
