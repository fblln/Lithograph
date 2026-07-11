//! Command execution and output rendering.

use crate::adr::{AdrRecord, AdrStore, AdrSummary};
use crate::agents::{AgentFileOutcome, IntegrateAgentsReport, integrate_agents};
use crate::ask::{McpExport, WikiSearch, render_ask_table};
use crate::cli::{
    AdrCommand, AdrCreateArgs, AdrDeleteArgs, AdrGetArgs, AdrListArgs, AdrTarget, AdrUpdateArgs,
    AskArgs, Cli, Command, DriftArgs, GoldenArgs, GraphCommand, GraphExportArgs, GraphImportArgs,
    GraphTarget, InitArgs, InspectArtifactsArgs, InspectCommand, InspectDsmArgs, InspectEnvArgs,
    InspectGraphArgs, InspectMetricsArgs, InspectModulesArgs, InspectTarget, IntegrateAgentsArgs,
    IntegrateMcpArgs, McpExportArgs, McpServerArgs, OutputFormat, QualityArgs, ServeArgs,
    ValidateMermaidArgs, ViewerArgs, WatchArgs,
};
use crate::domain::Artifact;
use crate::drift::{DriftDetector, DriftReport};
use crate::generation::{
    DeepInfraConfig, DeepInfraModel, LanguageModel, MockModel, OpenAiConfig, OpenAiModel,
};
use crate::golden::{check_or_update, render_report as render_golden_report};
use crate::graph::{
    DependencyMatrix, Graph, GraphArtifactReport, GraphBuilder, GraphIssue, GraphNode, GraphStore,
    GraphValidator, KnowledgeIndex,
};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::mcp::WikiMcpServer;
use crate::mcp_targets::{
    AgentTarget, IntegrationOutcome, TargetDetection, apply, detect, preview,
};
use crate::mermaid::{
    fix_path as fix_mermaid_path, render_report as render_mermaid_report,
    validate as validate_mermaid,
};
use crate::orchestrate::{
    InitReport, UpdateReport, run_init_with_options, run_update, run_update_with_options,
};
use crate::plan::{DocumentationModule, ModulePlanner};
use crate::quality::{inspect as inspect_quality, render_table as render_quality_table};
use crate::resolve::{EnvironmentExplanation, explain_environment};
use crate::run::{PerformanceBudget, RunMetadata};
use crate::storage::JsonStore;
use crate::viewer::{generate as generate_viewer, render_report as render_viewer_report};
use crate::watch::{WatchConfig, poll_once, render_report as render_watch_report};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;
use std::time::Duration;

/// Runs parsed CLI arguments and writes command output.
pub fn execute<W>(cli: Cli, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match cli.command {
        Some(Command::Init(args)) => execute_init(args, writer),
        Some(Command::Update(args)) => execute_update(args, writer),
        Some(Command::Inspect(command)) => execute_inspect(command, writer),
        Some(Command::IntegrateAgents(args)) => execute_integrate_agents(args, writer),
        Some(Command::Drift(args)) => execute_drift(args, writer),
        Some(Command::Ask(args)) => execute_ask(args, writer),
        Some(Command::McpExport(args)) => execute_mcp_export(args, writer),
        Some(Command::Golden(args)) => execute_golden(args, writer),
        Some(Command::Quality(args)) => execute_quality(args, writer),
        Some(Command::ValidateMermaid(args)) => execute_validate_mermaid(args, writer),
        Some(Command::McpServer(args)) => execute_mcp_server(args, writer),
        Some(Command::Viewer(args)) => execute_viewer(args, writer),
        Some(Command::Serve(args)) => execute_serve(args, writer),
        Some(Command::Graph(args)) => execute_graph(args, writer),
        Some(Command::Adr(command)) => execute_adr(command, writer),
        Some(Command::Watch(args)) => execute_watch(args, writer),
        Some(Command::IntegrateMcp(args)) => execute_integrate_mcp(args, writer),
        None => Ok(()),
    }
}

fn execute_adr<W>(command: AdrCommand, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match command.target {
        AdrTarget::Create(args) => execute_adr_create(args, writer),
        AdrTarget::Get(args) => execute_adr_get(args, writer),
        AdrTarget::Update(args) => execute_adr_update(args, writer),
        AdrTarget::Delete(args) => execute_adr_delete(args, writer),
        AdrTarget::List(args) => execute_adr_list(args, writer),
    }
}

fn execute_adr_create<W>(
    args: AdrCreateArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let record = AdrStore::new(&args.path).create(
        &args.title,
        &args.context,
        &args.decision,
        args.consequences.as_deref(),
    )?;
    write_adr_record(writer, &record, args.format)
}

fn execute_adr_get<W>(args: AdrGetArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let record = AdrStore::new(&args.path).get(&args.id)?;
    write_adr_record(writer, &record, args.format)
}

fn execute_adr_update<W>(
    args: AdrUpdateArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let store = AdrStore::new(&args.path);
    let mut record = store.get(&args.id)?;
    if let (Some(section), Some(value)) = (&args.section, &args.value) {
        record = store.update_section(&args.id, section, value)?;
    }
    if let Some(status) = args.status {
        record = store.update_status(&args.id, status.into())?;
    }
    write_adr_record(writer, &record, args.format)
}

fn execute_adr_delete<W>(
    args: AdrDeleteArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    AdrStore::new(&args.path).delete(&args.id)?;
    writer.write_all(format!("deleted {}\n", args.id).as_bytes())?;
    Ok(())
}

fn execute_adr_list<W>(args: AdrListArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let summaries = AdrStore::new(&args.path).list();
    let output = match args.format {
        OutputFormat::Table => render_adr_list_table(&summaries),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&summaries)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

fn write_adr_record<W>(
    writer: &mut W,
    record: &AdrRecord,
    format: OutputFormat,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let output = match format {
        OutputFormat::Table => render_adr_record_table(record),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(record)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders one ADR as a deterministic, human-readable table.
pub fn render_adr_record_table(record: &AdrRecord) -> String {
    let mut output = format!("{} [{:?}] {}\n", record.id, record.status, record.title);
    for (section, content) in &record.sections {
        output.push_str(&format!("- {section}: {content}\n"));
    }
    output
}

/// Renders every ADR as a deterministic, human-readable table.
pub fn render_adr_list_table(summaries: &[AdrSummary]) -> String {
    if summaries.is_empty() {
        return "no ADRs recorded\n".to_owned();
    }
    let mut output = format!("{} ADR(s):\n", summaries.len());
    for summary in summaries {
        output.push_str(&format!(
            "{} [{:?}] {}\n",
            summary.id, summary.status, summary.title
        ));
    }
    output
}

fn execute_graph<W>(command: GraphCommand, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match command.target {
        GraphTarget::Export(args) => execute_graph_export(args, writer),
        GraphTarget::Import(args) => execute_graph_import(args, writer),
    }
}

fn execute_graph_export<W>(
    args: GraphExportArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = GraphStore::new(&args.path).export_artifact(&args.output)?;
    writer.write_all(render_graph_artifact_report("exported", &report).as_bytes())?;
    Ok(())
}

fn execute_graph_import<W>(
    args: GraphImportArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = GraphStore::new(&args.path).import_artifact(&args.artifact)?;
    writer.write_all(render_graph_artifact_report("imported", &report).as_bytes())?;
    Ok(())
}

/// Renders a deterministic graph artifact operation summary.
pub fn render_graph_artifact_report(action: &str, report: &GraphArtifactReport) -> String {
    format!(
        "graph artifact {action}\n\
         artifact: {}\n\
         snapshot: {}\n\
         legacy graph: {}\n\
         format: {}\n\
         compression: {}\n\
         checksum: {}\n\
         schema: {}\n\
         graph model: {}\n\
         nodes: {}\n\
         relations: {}\n",
        report.artifact_path.display(),
        report.snapshot_path.display(),
        report.legacy_graph_path.display(),
        report.metadata.artifact_format_version,
        report.metadata.compression,
        report.metadata.snapshot_checksum,
        report.metadata.schema_version,
        report.metadata.graph_model_version,
        report.metadata.node_count,
        report.metadata.relation_count,
    )
}

fn execute_golden<W>(args: GoldenArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = check_or_update(&args.path, &args.golden_dir, args.update)?;
    writer.write_all(render_golden_report(&report).as_bytes())?;
    Ok(())
}

fn execute_quality<W>(args: QualityArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = inspect_quality(&args.path)?;
    let output = match args.format {
        OutputFormat::Table => render_quality_table(&report),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&report)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    if report.is_clean() {
        Ok(())
    } else {
        Err("quality inspection failed".into())
    }
}

fn execute_validate_mermaid<W>(
    args: ValidateMermaidArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    if args.fix {
        let files_changed = fix_mermaid_path(&args.path)?;
        writer.write_all(format!("fixed node ids in {files_changed} file(s)\n").as_bytes())?;
    }
    let report = validate_mermaid(&args.path, args.node_validator.as_deref())?;
    writer.write_all(render_mermaid_report(&report).as_bytes())?;
    if report.is_clean() {
        Ok(())
    } else {
        Err("Mermaid validation failed".into())
    }
}

fn execute_mcp_server<W>(
    args: McpServerArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let stdin = std::io::stdin();
    WikiMcpServer::new(&args.path).run(stdin.lock(), writer)
}

fn execute_viewer<W>(args: ViewerArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let output_dir = if args.output_dir.is_absolute() {
        args.output_dir
    } else {
        args.path.join(args.output_dir)
    };
    let report = generate_viewer(&args.path, &output_dir)?;
    writer.write_all(render_viewer_report(&report).as_bytes())?;
    Ok(())
}

fn execute_serve<W>(args: ServeArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let assets = if args.assets.is_absolute() {
        args.assets
    } else {
        args.path.join(args.assets)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(crate::serve::run(&args.path, &assets, args.port, writer))?;
    Ok(())
}

/// Polls `args.path` for staleness and, with `--auto-index`, runs `update`
/// when staleness is detected (AC1: never auto-indexes without that
/// explicit flag). `--once` polls a single time and returns; otherwise this
/// loops with a real `std::thread::sleep` between polls, which is why only
/// the single-poll (`--once`) path is unit-tested -- an infinite loop with
/// real sleeps has no deterministic endpoint to assert against.
fn execute_watch<W>(args: WatchArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let config = WatchConfig {
        max_artifacts: args.max_artifacts,
        poll_interval: Duration::from_secs(args.interval_secs),
    };

    loop {
        let report = poll_once(&args.path, &config)?;
        writer.write_all(render_watch_report(&report).as_bytes())?;
        if report.stale && args.auto_index {
            let (model, model_name) = select_model()?;
            let update_report = run_update(&args.path, model.as_ref(), &model_name, "v1")?;
            writer.write_all(render_update_report(&update_report).as_bytes())?;
        }
        if args.once {
            return Ok(());
        }
        std::thread::sleep(config.poll_interval);
    }
}

fn execute_ask<W>(args: AskArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let answer = WikiSearch.ask(&args.path, &args.question)?;
    let output = match args.format {
        OutputFormat::Table => render_ask_table(&answer),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&answer)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

fn execute_mcp_export<W>(
    args: McpExportArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let export: McpExport = WikiSearch.export(&args.path, args.question.as_deref())?;
    let mut json = serde_json::to_string_pretty(&export)?;
    json.push('\n');
    writer.write_all(json.as_bytes())?;
    Ok(())
}

fn execute_drift<W>(args: DriftArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::cache_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let graph = GraphBuilder.build(&args.path, &artifacts);
    let report = DriftDetector.scan(&artifacts, &graph, &args.path);

    let output = match args.format {
        OutputFormat::Table => render_drift_table(&report),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&report)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders a deterministic, human-readable drift report: one line per
/// finding (kind, artifact path, one-based line when evidence has a span,
/// and the stale detail text), or a clear "no drift" message when empty.
pub fn render_drift_table(report: &DriftReport) -> String {
    if report.findings.is_empty() {
        return "no drift detected\n".to_owned();
    }
    let mut output = format!("{} drift finding(s):\n", report.findings.len());
    for finding in &report.findings {
        let line = finding
            .evidence
            .span
            .as_ref()
            .map(|span| span.start_line.to_string())
            .unwrap_or_else(|| "-".to_owned());
        output.push_str(&format!(
            "  [{:?}] {}:{line} {}\n",
            finding.kind, finding.artifact_path, finding.detail
        ));
    }
    output
}

fn execute_integrate_agents<W>(
    args: IntegrateAgentsArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = integrate_agents(&args.path)?;
    writer.write_all(render_integrate_agents_report(&report).as_bytes())?;
    Ok(())
}

/// Renders a deterministic, human-readable `integrate-agents` summary.
pub fn render_integrate_agents_report(report: &IntegrateAgentsReport) -> String {
    if report.results.is_empty() {
        return "no AGENTS.md or CLAUDE.md found at the repository root; nothing to do\n"
            .to_owned();
    }
    let mut output = String::new();
    for result in &report.results {
        let outcome = match result.outcome {
            AgentFileOutcome::Created => "created",
            AgentFileOutcome::Refreshed => "refreshed",
            AgentFileOutcome::Unchanged => "unchanged",
        };
        output.push_str(&format!("{}: {outcome}\n", result.path.display()));
    }
    output
}

/// Detects, previews, or applies per-agent MCP integration (LIT-22.8.3).
/// Without `args.target`, this only detects and reports (AC1); with a
/// target and no `--apply`, it previews (AC2); `--apply` is the only path
/// that writes.
fn execute_integrate_mcp<W>(
    args: IntegrateMcpArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let Some(requested) = args.target.as_deref() else {
        let detections = detect(&args.path);
        writer.write_all(render_detections(&detections, args.format)?.as_bytes())?;
        return Ok(());
    };
    let target = AgentTarget::parse(requested).ok_or_else(|| {
        format!(
            "unknown --target \"{requested}\"; expected one of codex, claude, gemini, zed, aider"
        )
    })?;
    let outcome = if args.apply {
        apply(&args.path, target)?
    } else {
        preview(&args.path, target)?
    };
    writer.write_all(render_outcome(&outcome, args.apply, args.format)?.as_bytes())?;
    Ok(())
}

fn render_detections(
    detections: &[TargetDetection],
    format: OutputFormat,
) -> Result<String, Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        let mut json = serde_json::to_string_pretty(detections)?;
        json.push('\n');
        return Ok(json);
    }
    let mut output = String::new();
    for detection in detections {
        if !detection.supported {
            output.push_str(&format!(
                "{}: unsupported -- {}\n",
                detection.target,
                detection.reason.as_deref().unwrap_or("not supported")
            ));
            continue;
        }
        let path = detection
            .config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        output.push_str(&format!(
            "{}: supported, config={path}, exists={}, integrated={}\n",
            detection.target, detection.config_exists, detection.already_integrated
        ));
    }
    Ok(output)
}

fn render_outcome(
    outcome: &IntegrationOutcome,
    applied: bool,
    format: OutputFormat,
) -> Result<String, Box<dyn std::error::Error>> {
    if format == OutputFormat::Json {
        let mut json = serde_json::to_string_pretty(outcome)?;
        json.push('\n');
        return Ok(json);
    }
    let mode = if applied && outcome.changed {
        "applied"
    } else if applied {
        "applied (no change)"
    } else {
        "previewed"
    };
    Ok(format!(
        "{}: {mode}\npath: {}\nchanged: {}\n---\n{}",
        outcome.target,
        outcome.config_path.display(),
        outcome.changed,
        outcome.content
    ))
}

fn execute_init<W>(args: InitArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let (model, model_name) = select_model()?;
    let report = run_init_with_options(
        &args.path,
        model.as_ref(),
        &model_name,
        &args.prompt_version,
        args.semantic_grouping,
    )?;
    writer.write_all(render_init_report(&report).as_bytes())?;
    Ok(())
}

fn execute_update<W>(args: InitArgs, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let (model, model_name) = select_model()?;
    let report = run_update_with_options(
        &args.path,
        model.as_ref(),
        &model_name,
        &args.prompt_version,
        args.semantic_grouping,
    )?;
    writer.write_all(render_update_report(&report).as_bytes())?;
    Ok(())
}

/// Selects a language model backend from environment variables, in order:
/// DeepInfra (via `rig-core`) when `LITHOGRAPH_DEEPINFRA_API_KEY` is set,
/// then the direct OpenAI-compatible adapter when `LITHOGRAPH_OPENAI_API_KEY`
/// is set, otherwise the deterministic mock (the zero-configuration default
/// so `init` always works without credentials).
fn select_model() -> Result<(Box<dyn LanguageModel>, String), Box<dyn std::error::Error>> {
    if let Ok(api_key) = std::env::var("LITHOGRAPH_DEEPINFRA_API_KEY") {
        let model_name = std::env::var("LITHOGRAPH_DEEPINFRA_MODEL").map_err(|_| {
            "LITHOGRAPH_DEEPINFRA_API_KEY is set but LITHOGRAPH_DEEPINFRA_MODEL is not; \
             set it to the DeepInfra model path to use (e.g. a DeepSeek model)"
        })?;
        let mut config = DeepInfraConfig::new(api_key, model_name.clone());
        if let Ok(base_url) = std::env::var("LITHOGRAPH_DEEPINFRA_BASE_URL") {
            config = config.with_base_url(base_url);
        }
        if let Ok(reasoning_effort) = std::env::var("LITHOGRAPH_DEEPINFRA_REASONING_EFFORT") {
            config = config.with_reasoning_effort(reasoning_effort);
        }
        return Ok((Box::new(DeepInfraModel::new(config)?), model_name));
    }

    match std::env::var("LITHOGRAPH_OPENAI_API_KEY") {
        Ok(api_key) => {
            let base_url = std::env::var("LITHOGRAPH_OPENAI_BASE_URL")
                .unwrap_or_else(|_| "https://api.openai.com/v1".to_owned());
            let model_name = std::env::var("LITHOGRAPH_OPENAI_MODEL")
                .unwrap_or_else(|_| "gpt-4o-mini".to_owned());
            let mut config = OpenAiConfig::new(base_url, api_key, model_name.clone());
            if let Ok(reasoning_effort) = std::env::var("LITHOGRAPH_OPENAI_REASONING_EFFORT") {
                config = config.with_reasoning_effort(reasoning_effort);
            }
            Ok((Box::new(OpenAiModel::new(config)), model_name))
        }
        Err(_) => Ok((Box::new(MockModel), "mock".to_owned())),
    }
}

/// Renders a deterministic, human-readable `init` summary.
pub fn render_init_report(report: &InitReport) -> String {
    format!(
        "artifacts: {}\n\
         graph nodes: {}\n\
         graph relations: {}\n\
         modules: {}\n\
         pages: {}\n\
         pages written: {}\n\
         changed artifacts: {}\n\
         reanalyzed artifacts: {}\n\
         graph: {}\n\
         manifest: {}\n\
         run metadata: {}\n",
        report.artifact_count,
        report.graph_node_count,
        report.graph_relation_count,
        report.module_count,
        report.page_count,
        report.pages_written,
        report.changed_artifact_count,
        report.artifacts_reanalyzed_count,
        report.graph_path.display(),
        report.manifest_path.display(),
        report.run_metadata_path.display(),
    )
}

/// Renders a deterministic, human-readable `update` summary.
pub fn render_update_report(report: &UpdateReport) -> String {
    format!(
        "artifacts: {}\n\
         graph nodes: {}\n\
         graph relations: {}\n\
         modules: {}\n\
         pages: {}\n\
         pages regenerated: {}\n\
         changed artifacts: {}\n\
         reanalyzed artifacts: {}\n\
         graph: {}\n\
         manifest: {}\n\
         run metadata: {}\n",
        report.artifact_count,
        report.graph_node_count,
        report.graph_relation_count,
        report.module_count,
        report.page_count,
        report.pages_regenerated,
        report.changed_artifact_count,
        report.artifacts_reanalyzed_count,
        report.graph_path.display(),
        report.manifest_path.display(),
        report.run_metadata_path.display(),
    )
}

fn execute_inspect<W>(
    command: InspectCommand,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match command.target {
        InspectTarget::Artifacts(args) => execute_inspect_artifacts(args, writer),
        InspectTarget::Graph(args) => execute_inspect_graph(args, writer),
        InspectTarget::Env(args) => execute_inspect_env(args, writer),
        InspectTarget::Modules(args) => execute_inspect_modules(args, writer),
        InspectTarget::Dsm(args) => execute_inspect_dsm(args, writer),
        InspectTarget::Metrics(args) => execute_inspect_metrics(args, writer),
    }
}

/// Reads the last recorded `.lithograph/run.json` and renders its metrics
/// (LIT-22.8.4 AC1/AC2). When any `--max-*`/`--min-*` threshold is set,
/// also checks it as a [`PerformanceBudget`] and, on violation, returns an
/// error after writing the report -- the caller always sees the numbers
/// even when the command exits non-zero (AC3).
fn execute_inspect_metrics<W>(
    args: InspectMetricsArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let run_metadata_path = args.path.join(".lithograph/run.json");
    let metadata: RunMetadata = JsonStore.read(&run_metadata_path)?.ok_or_else(|| {
        format!(
            "no run metadata found at {}; run `init` or `update` first",
            run_metadata_path.display()
        )
    })?;
    let budget = PerformanceBudget {
        max_graph_node_count: args.max_graph_nodes,
        max_graph_relation_count: args.max_graph_relations,
        min_cache_hit_rate: args
            .min_cache_hit_rate_percent
            .map(|percent| f64::from(percent) / 100.0),
        max_estimated_prompt_tokens: args.max_tokens,
    };
    let violations = budget.check(&metadata);

    let output = match args.format {
        OutputFormat::Table => render_metrics_table(&metadata, &violations),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&serde_json::json!({
                "metrics": &metadata,
                "violations": &violations,
            }))?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;

    if violations.is_empty() {
        return Ok(());
    }
    let summary = violations
        .iter()
        .map(|violation| {
            format!(
                "{} (limit {}, actual {})",
                violation.metric, violation.limit, violation.actual
            )
        })
        .collect::<Vec<_>>()
        .join(", ");
    Err(format!(
        "{} performance budget violation(s): {summary}",
        violations.len()
    )
    .into())
}

/// Renders a deterministic, human-readable run metrics summary.
pub fn render_metrics_table(
    metadata: &RunMetadata,
    violations: &[crate::run::BudgetViolation],
) -> String {
    let mut output = format!(
        "run: {} ({})\n\
         graph nodes: {}\n\
         graph relations: {}\n\
         cache hits: {}\n\
         cache misses: {}\n\
         cache hit rate: {:.2}\n\
         estimated prompt tokens: {}\n",
        metadata.run_id,
        metadata.command,
        metadata.graph_node_count,
        metadata.graph_relation_count,
        metadata.cache_hits,
        metadata.cache_misses,
        metadata.cache_hit_rate(),
        metadata.estimated_prompt_tokens,
    );
    for timing in &metadata.stage_timings {
        output.push_str(&format!(
            "stage {:?}: {}ms\n",
            timing.stage, timing.duration_ms
        ));
    }
    if violations.is_empty() {
        output.push_str("budget: within every configured threshold\n");
    } else {
        for violation in violations {
            output.push_str(&format!(
                "budget violation: {} (limit {}, actual {})\n",
                violation.metric, violation.limit, violation.actual
            ));
        }
    }
    output
}

fn execute_inspect_modules<W>(
    args: InspectModulesArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let graph = GraphBuilder.build(&args.path, &artifacts);
    let modules = if args.semantic_grouping {
        ModulePlanner.plan_with_semantic_grouping(&graph, &artifacts)
    } else {
        ModulePlanner.plan(&graph, &artifacts)
    };

    let output = match args.format {
        OutputFormat::Table => render_modules_table(&modules),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&modules)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders a deterministic human-readable module tree: kind, name, member
/// count, input hash, and token estimate per module.
pub fn render_modules_table(modules: &[DocumentationModule]) -> String {
    let rows: Vec<(String, &str, usize, &str, u32)> = modules
        .iter()
        .map(|module| {
            (
                format!("{:?}", module.kind),
                module.name.as_str(),
                module.members.len(),
                module.input_hash.as_str(),
                module.estimated_tokens,
            )
        })
        .collect();
    let kind_width = rows
        .iter()
        .map(|row| row.0.len())
        .max()
        .unwrap_or("kind".len())
        .max("kind".len());
    let name_width = rows
        .iter()
        .map(|row| row.1.len())
        .max()
        .unwrap_or("name".len())
        .max("name".len());

    let mut output = format!(
        "{:<kind_width$}  {:<name_width$}  {:>7}  {:<64}  {:>6}\n",
        "kind", "name", "members", "input_hash", "tokens"
    );
    for (kind, name, member_count, input_hash, tokens) in rows {
        output.push_str(&format!(
            "{kind:<kind_width$}  {name:<name_width$}  {member_count:>7}  {input_hash:<64}  {tokens:>6}\n"
        ));
    }
    output
}

fn execute_inspect_graph<W>(
    args: InspectGraphArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let graph = GraphBuilder.build(&args.path, &artifacts);
    let issues = GraphValidator.validate(&graph, &artifacts);
    if !issues.is_empty() {
        return Err(render_graph_diagnostics(&issues).into());
    }

    let output = match args.format {
        OutputFormat::Table => render_graph_table(&graph),
        OutputFormat::Json => graph.to_json()?,
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

fn execute_inspect_env<W>(
    args: InspectEnvArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let graph = GraphBuilder.build(&args.path, &artifacts);
    let issues = GraphValidator.validate(&graph, &artifacts);
    if !issues.is_empty() {
        return Err(render_graph_diagnostics(&issues).into());
    }
    let explanation = explain_environment(&graph, args.variable.as_deref());
    let output = match args.format {
        OutputFormat::Table => render_environment_table(&explanation),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&explanation)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders environment explanations in a stable, reviewable table.
pub fn render_environment_table(explanation: &EnvironmentExplanation) -> String {
    if explanation.variables.is_empty() {
        return "no environment variables matched\n".to_owned();
    }
    let mut output = String::new();
    for variable in &explanation.variables {
        output.push_str(&format!("{} ({})\n", variable.name, variable.canonical));
        output.push_str(&format!(
            "  resolved: {}\n",
            if variable.resolved.is_empty() {
                "none".to_owned()
            } else {
                variable
                    .resolved
                    .iter()
                    .map(|link| format!("{} [{:?}]", link.config_key, link.confidence))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        ));
        output.push_str(&format!("  code users: {}\n", variable.code_users.len()));
        output.push_str(&format!("  definitions: {}\n", variable.definitions.len()));
        if !variable.candidates.is_empty() {
            output.push_str(&format!(
                "  candidates: {}\n",
                variable
                    .candidates
                    .iter()
                    .map(|candidate| candidate.config_key.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if let Some(reason) = &variable.unresolved_reason {
            output.push_str(&format!("  reason: {reason}\n"));
        }
    }
    output
}

fn execute_inspect_dsm<W>(
    args: InspectDsmArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let graph = GraphBuilder.build(&args.path, &artifacts);
    let issues = GraphValidator.validate(&graph, &artifacts);
    if !issues.is_empty() {
        return Err(render_graph_diagnostics(&issues).into());
    }
    let matrix = KnowledgeIndex::new(&graph).dependency_matrix();
    let output = match args.format {
        OutputFormat::Table => render_dsm_table(&matrix),
        OutputFormat::Json => serde_json::to_string_pretty(&matrix)? + "\n",
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders a bounded human-readable DSM. JSON remains complete for automation.
pub fn render_dsm_table(matrix: &DependencyMatrix) -> String {
    const TABLE_CAP: usize = 40;
    let shown = matrix.modules.len().min(TABLE_CAP);
    let mut output = format!(
        "modules: {}\ncycles: {}\n",
        matrix.modules.len(),
        matrix.cycles.len()
    );
    if matrix.modules.len() > TABLE_CAP {
        output.push_str(&format!(
            "showing first {TABLE_CAP} modules; use --format json for the complete matrix\n"
        ));
    }
    output.push_str("module\t");
    output.push_str(
        &matrix.modules[..shown]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\t"),
    );
    output.push('\n');
    for (row, module) in matrix.modules.iter().take(shown).enumerate() {
        output.push_str(module.as_str());
        output.push('\t');
        output.push_str(
            &matrix.cells[row][..shown]
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("\t"),
        );
        output.push('\n');
    }
    output
}

/// Renders graph validation issues as an actionable diagnostic message.
pub fn render_graph_diagnostics(issues: &[GraphIssue]) -> String {
    let mut message = format!("graph validation failed with {} issue(s):\n", issues.len());
    for issue in issues {
        message.push_str(&format!("  - [{:?}] {issue}\n", issue.kind));
    }
    message
}

/// Renders a deterministic human-readable graph node/relation summary.
pub fn render_graph_table(graph: &Graph) -> String {
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for node in &graph.nodes {
        *counts.entry(node_kind_label(node)).or_insert(0) += 1;
    }

    let mut output = format!("nodes: {}\n", graph.nodes.len());
    for (label, count) in &counts {
        output.push_str(&format!("  {label:<13} {count:>5}\n"));
    }
    output.push_str(&format!("relations: {}\n", graph.relations.len()));
    output
}

fn node_kind_label(node: &GraphNode) -> &'static str {
    match node {
        GraphNode::Artifact(_) => "artifact",
        GraphNode::Symbol(_) => "symbol",
        GraphNode::Config(_) => "config",
        GraphNode::Documentation(_) => "documentation",
        GraphNode::Container(_) => "container",
        GraphNode::Command(_) => "command",
        GraphNode::EnvVar(_) => "env_var",
        GraphNode::Module(_) => "module",
        GraphNode::Package(_) => "package",
        GraphNode::Unresolved(_) => "unresolved",
    }
}

fn execute_inspect_artifacts<W>(
    args: InspectArtifactsArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::scan_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let output = match args.format {
        OutputFormat::Table => render_artifacts_table(&artifacts),
        OutputFormat::Json => render_artifacts_json(&artifacts)?,
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders artifacts as a deterministic human-readable table.
pub fn render_artifacts_table(artifacts: &[Artifact]) -> String {
    let rows = artifact_rows(artifacts);
    let path_width = rows
        .iter()
        .map(|row| row.path.len())
        .max()
        .unwrap_or("path".len())
        .max("path".len());
    let category_width = rows
        .iter()
        .map(|row| row.category.len())
        .max()
        .unwrap_or("category".len())
        .max("category".len());
    let format_width = rows
        .iter()
        .map(|row| row.format.len())
        .max()
        .unwrap_or("format".len())
        .max("format".len());

    let mut output = String::new();
    output.push_str(&format!(
        "{:<path_width$}  {:<category_width$}  {:<format_width$}  {:<16}  {:>8}  {:<64}  {:<10}  {:<11}  {:>3}  {:>3}\n",
        "path",
        "category",
        "format",
        "support",
        "size",
        "hash",
        "text",
        "model",
        "gen",
        "ven",
    ));
    output.push_str(&format!(
        "{:-<path_width$}  {:-<category_width$}  {:-<format_width$}  {:-<16}  {:->8}  {:-<64}  {:-<10}  {:-<11}  {:->3}  {:->3}\n",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
    ));
    for row in rows {
        output.push_str(&format!(
            "{:<path_width$}  {:<category_width$}  {:<format_width$}  {:<16}  {:>8}  {:<64}  {:<10}  {:<11}  {:>3}  {:>3}\n",
            row.path,
            row.category,
            row.format,
            row.support_tier,
            row.size_bytes,
            row.content_hash,
            row.text_status,
            row.model_policy,
            row.generated_score,
            row.vendored_score,
        ));
    }
    output
}

/// Renders artifacts as deterministic pretty JSON.
pub fn render_artifacts_json(artifacts: &[Artifact]) -> Result<String, serde_json::Error> {
    let output = ArtifactListOutput {
        artifacts: artifact_rows(artifacts),
    };
    let mut json = serde_json::to_string_pretty(&output)?;
    json.push('\n');
    Ok(json)
}

fn artifact_rows(artifacts: &[Artifact]) -> Vec<ArtifactOutputRow> {
    artifacts.iter().map(ArtifactOutputRow::from).collect()
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArtifactListOutput {
    artifacts: Vec<ArtifactOutputRow>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct ArtifactOutputRow {
    path: String,
    category: String,
    format: String,
    support_tier: String,
    size_bytes: u64,
    content_hash: String,
    text_status: String,
    model_policy: String,
    generated_score: u8,
    vendored_score: u8,
}

impl From<&Artifact> for ArtifactOutputRow {
    fn from(artifact: &Artifact) -> Self {
        Self {
            path: artifact.path.as_str().to_owned(),
            category: format!("{:?}", artifact.category),
            format: artifact
                .detected_format
                .clone()
                .unwrap_or_else(|| "-".to_owned()),
            support_tier: format!("{:?}", artifact.support_tier),
            size_bytes: artifact.size_bytes,
            content_hash: artifact.content_hash.as_str().to_owned(),
            text_status: format!("{:?}", artifact.text_status),
            model_policy: format!("{:?}", artifact.model_policy),
            generated_score: artifact.generated_score,
            vendored_score: artifact.vendored_score,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        execute, render_artifacts_json, render_artifacts_table, render_graph_diagnostics,
        select_model,
    };
    use crate::cli::{
        AdrCommand, AdrCreateArgs, AdrDeleteArgs, AdrGetArgs, AdrListArgs, AdrStatusArg, AdrTarget,
        AdrUpdateArgs, AskArgs, Cli, Command, DriftArgs, GraphCommand, GraphExportArgs,
        GraphImportArgs, GraphTarget, InitArgs, InspectArtifactsArgs, InspectCommand,
        InspectGraphArgs, InspectMetricsArgs, InspectModulesArgs, InspectTarget,
        IntegrateAgentsArgs, IntegrateMcpArgs, McpExportArgs, OutputFormat, ValidateMermaidArgs,
        WatchArgs,
    };
    use crate::graph::{GraphIssue, GraphIssueKind};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::{Path, PathBuf};

    /// LIT-22.8.5 AC3: `init`/`update` default to the offline, network-free
    /// `MockModel` unless a caller explicitly opts in with an API key
    /// environment variable -- read-only (never sets or unsets an env var,
    /// so this can't race any other test) and true in every normal test
    /// environment, since nothing in this codebase sets either variable.
    #[test]
    fn select_model_defaults_to_the_offline_mock_model_without_api_keys_set()
    -> Result<(), Box<dyn std::error::Error>> {
        assert!(std::env::var("LITHOGRAPH_DEEPINFRA_API_KEY").is_err());
        assert!(std::env::var("LITHOGRAPH_OPENAI_API_KEY").is_err());

        let (_, model_name) = select_model()?;

        assert_eq!(model_name, "mock");

        Ok(())
    }

    #[test]
    fn execute_init_writes_docs_and_reports_counts() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        let cli = Cli {
            command: Some(Command::Init(InitArgs {
                path: temp.path().to_path_buf(),
                prompt_version: "v1".to_owned(),
                semantic_grouping: false,
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("artifacts:"));
        assert!(output.contains("pages written:"));
        assert!(temp.path().join("docs/lithograph/quickstart.md").exists());

        Ok(())
    }

    #[test]
    fn execute_update_reports_regenerated_pages() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        let update_cli = || Cli {
            command: Some(Command::Update(InitArgs {
                path: temp.path().to_path_buf(),
                prompt_version: "v1".to_owned(),
                semantic_grouping: false,
            })),
        };
        let mut output = Vec::new();

        execute(update_cli(), &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("pages regenerated:"));
        assert!(temp.path().join(".lithograph/run.json").exists());

        Ok(())
    }

    fn watch_args(path: &Path, once: bool, auto_index: bool) -> WatchArgs {
        WatchArgs {
            path: path.to_path_buf(),
            max_artifacts: 20_000,
            interval_secs: 0,
            once,
            auto_index,
        }
    }

    #[test]
    fn execute_watch_once_reports_stale_before_init() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        let mut output = Vec::new();

        execute(
            Cli {
                command: Some(Command::Watch(watch_args(temp.path(), true, false))),
            },
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.starts_with("stale:"));
        assert!(!temp.path().join(".lithograph/run.json").exists());

        Ok(())
    }

    #[test]
    fn execute_watch_once_reports_up_to_date_after_init() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        execute(
            Cli {
                command: Some(Command::Init(InitArgs {
                    path: temp.path().to_path_buf(),
                    prompt_version: "v1".to_owned(),
                    semantic_grouping: false,
                })),
            },
            &mut Vec::new(),
        )?;
        let mut output = Vec::new();

        execute(
            Cli {
                command: Some(Command::Watch(watch_args(temp.path(), true, false))),
            },
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.starts_with("up to date:"));

        Ok(())
    }

    #[test]
    fn execute_watch_without_auto_index_never_writes_docs() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;

        execute(
            Cli {
                command: Some(Command::Watch(watch_args(temp.path(), true, false))),
            },
            &mut Vec::new(),
        )?;

        assert!(!temp.path().join("docs/lithograph/quickstart.md").exists());

        Ok(())
    }

    #[test]
    fn execute_watch_with_auto_index_runs_update_when_stale()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        let mut output = Vec::new();

        execute(
            Cli {
                command: Some(Command::Watch(watch_args(temp.path(), true, true))),
            },
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("stale:"));
        assert!(output.contains("pages regenerated:"));
        assert!(temp.path().join("docs/lithograph/quickstart.md").exists());

        Ok(())
    }

    #[test]
    fn execute_watch_rejects_repositories_over_the_safe_artifact_limit()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        let args = WatchArgs {
            max_artifacts: 1,
            ..watch_args(temp.path(), true, false)
        };

        match execute(
            Cli {
                command: Some(Command::Watch(args)),
            },
            &mut Vec::new(),
        ) {
            Ok(()) => Err("expected a project-too-large error".into()),
            Err(error) => {
                assert!(error.to_string().contains("safe watch limit"));
                Ok(())
            }
        }
    }

    #[test]
    fn execute_graph_export_and_import_round_trips_artifact()
    -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::TempDir::new()?;
        let destination = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            source.path(),
        )?;
        execute(
            Cli {
                command: Some(Command::Init(InitArgs {
                    path: source.path().to_path_buf(),
                    prompt_version: "v1".to_owned(),
                    semantic_grouping: false,
                })),
            },
            &mut Vec::new(),
        )?;
        let artifact_path = source.path().join("graph.lithograph-graph.gz");

        let mut export_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Graph(GraphCommand {
                    target: GraphTarget::Export(GraphExportArgs {
                        path: source.path().to_path_buf(),
                        output: artifact_path.clone(),
                    }),
                })),
            },
            &mut export_output,
        )?;
        let export_output = String::from_utf8(export_output)?;

        let mut import_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Graph(GraphCommand {
                    target: GraphTarget::Import(GraphImportArgs {
                        path: destination.path().to_path_buf(),
                        artifact: artifact_path.clone(),
                    }),
                })),
            },
            &mut import_output,
        )?;
        let import_output = String::from_utf8(import_output)?;

        assert!(artifact_path.exists());
        assert!(export_output.contains("graph artifact exported"));
        assert!(import_output.contains("graph artifact imported"));
        assert!(
            destination
                .path()
                .join(".lithograph/graph/current.json")
                .exists()
        );
        assert!(destination.path().join(".lithograph/graph.json").exists());

        Ok(())
    }

    #[test]
    fn execute_ask_and_mcp_export_read_generated_docs() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        let mut init_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Init(InitArgs {
                    path: temp.path().to_path_buf(),
                    prompt_version: "v1".to_owned(),
                    semantic_grouping: false,
                })),
            },
            &mut init_output,
        )?;

        let mut ask_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Ask(AskArgs {
                    path: temp.path().to_path_buf(),
                    question: "source evidence".to_owned(),
                    format: OutputFormat::Table,
                })),
            },
            &mut ask_output,
        )?;
        let ask_output = String::from_utf8(ask_output)?;
        assert!(ask_output.contains("generated wiki page"));

        let mut export_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::McpExport(McpExportArgs {
                    path: temp.path().to_path_buf(),
                    question: Some("modules".to_owned()),
                })),
            },
            &mut export_output,
        )?;
        let parsed: serde_json::Value = serde_json::from_slice(&export_output)?;
        assert!(parsed["tools"].as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.as_str() == Some("read_research_memory"))
        }));
        assert!(
            parsed["structure"]
                .as_array()
                .is_some_and(|pages| !pages.is_empty())
        );
        assert!(parsed["answer"].is_object());

        Ok(())
    }

    #[test]
    fn execute_inspect_modules_table_lists_kind_name_and_tokens()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let cli = Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Modules(InspectModulesArgs {
                    path: root,
                    semantic_grouping: false,
                    format: OutputFormat::Table,
                }),
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("kind"));
        assert!(output.contains("input_hash"));
        assert!(output.contains("RustCrate"));
        assert!(output.contains("fixture-worker"));
        assert!(output.contains("PythonPackage"));

        Ok(())
    }

    #[test]
    fn execute_inspect_modules_json_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let modules_cli = || Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Modules(InspectModulesArgs {
                    path: root.clone(),
                    semantic_grouping: false,
                    format: OutputFormat::Json,
                }),
            })),
        };
        let mut first = Vec::new();
        let mut second = Vec::new();

        execute(modules_cli(), &mut first)?;
        execute(modules_cli(), &mut second)?;

        assert_eq!(first, second);
        let parsed: serde_json::Value = serde_json::from_slice(&first)?;
        assert!(parsed.as_array().is_some_and(|modules| modules.len() == 11));

        Ok(())
    }

    #[test]
    fn execute_integrate_agents_creates_then_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("AGENTS.md"),
            "# Agents\n\nExisting instructions.\n",
        )?;
        let cli = || Cli {
            command: Some(Command::IntegrateAgents(IntegrateAgentsArgs {
                path: temp.path().to_path_buf(),
            })),
        };

        let mut first = Vec::new();
        execute(cli(), &mut first)?;
        let first = String::from_utf8(first)?;
        assert!(first.contains("created"));

        let mut second = Vec::new();
        execute(cli(), &mut second)?;
        let second = String::from_utf8(second)?;
        assert!(second.contains("unchanged"));

        Ok(())
    }

    fn integrate_mcp_args(path: &Path, target: Option<&str>, apply: bool) -> IntegrateMcpArgs {
        IntegrateMcpArgs {
            path: path.to_path_buf(),
            target: target.map(str::to_owned),
            apply,
            format: OutputFormat::Table,
        }
    }

    #[test]
    fn execute_integrate_mcp_without_target_only_detects() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        let mut output = Vec::new();

        execute(
            Cli {
                command: Some(Command::IntegrateMcp(integrate_mcp_args(
                    temp.path(),
                    None,
                    false,
                ))),
            },
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("claude: supported"));
        assert!(output.contains("aider: unsupported"));
        assert!(std::fs::read_dir(temp.path())?.next().is_none());

        Ok(())
    }

    #[test]
    fn execute_integrate_mcp_preview_does_not_write() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let mut output = Vec::new();

        execute(
            Cli {
                command: Some(Command::IntegrateMcp(integrate_mcp_args(
                    temp.path(),
                    Some("claude"),
                    false,
                ))),
            },
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("previewed"));
        assert!(output.contains("mcpServers"));
        assert!(!temp.path().join(".mcp.json").exists());

        Ok(())
    }

    #[test]
    fn execute_integrate_mcp_apply_writes_then_is_idempotent()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let cli = |target| Cli {
            command: Some(Command::IntegrateMcp(integrate_mcp_args(
                temp.path(),
                Some(target),
                true,
            ))),
        };

        let mut first = Vec::new();
        execute(cli("zed"), &mut first)?;
        let first = String::from_utf8(first)?;
        assert!(first.contains("applied"));
        assert!(temp.path().join(".zed/settings.json").exists());

        let mut second = Vec::new();
        execute(cli("zed"), &mut second)?;
        let second = String::from_utf8(second)?;
        assert!(second.contains("applied (no change)"));

        Ok(())
    }

    #[test]
    fn execute_integrate_mcp_reports_actionable_message_for_unsupported_target()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        match execute(
            Cli {
                command: Some(Command::IntegrateMcp(integrate_mcp_args(
                    temp.path(),
                    Some("aider"),
                    false,
                ))),
            },
            &mut Vec::new(),
        ) {
            Ok(()) => Err("expected an unsupported-target error".into()),
            Err(error) => {
                assert!(error.to_string().contains("no native MCP"));
                Ok(())
            }
        }
    }

    #[test]
    fn execute_integrate_mcp_rejects_an_unknown_target() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        match execute(
            Cli {
                command: Some(Command::IntegrateMcp(integrate_mcp_args(
                    temp.path(),
                    Some("not-a-real-target"),
                    false,
                ))),
            },
            &mut Vec::new(),
        ) {
            Ok(()) => Err("expected an unknown-target error".into()),
            Err(error) => {
                assert!(error.to_string().contains("unknown --target"));
                Ok(())
            }
        }
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut stack = vec![from.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                let destination = to.join(path.strip_prefix(from)?);
                if path.is_dir() {
                    std::fs::create_dir_all(&destination)?;
                    stack.push(path);
                } else {
                    if let Some(parent) = destination.parent() {
                        std::fs::create_dir_all(parent)?;
                    }
                    std::fs::copy(&path, &destination)?;
                }
            }
        }
        Ok(())
    }

    #[test]
    fn table_renderer_includes_required_columns() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let table = render_artifacts_table(&artifacts);

        assert!(table.contains("path"));
        assert!(table.contains("category"));
        assert!(table.contains("format"));
        assert!(table.contains("support"));
        assert!(table.contains("size"));
        assert!(table.contains("hash"));
        assert!(table.contains("text"));
        assert!(table.contains("model"));
        assert!(table.contains("generated/client.py"));
        assert!(table.contains("GeneratedSource"));

        Ok(())
    }

    #[test]
    fn json_renderer_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let json = render_artifacts_json(&artifacts)?;
        let rerendered = render_artifacts_json(&artifacts)?;

        assert_eq!(json, rerendered);
        assert!(json.contains("\"path\": \".github/workflows/ci.yml\""));
        assert!(json.contains("\"model_policy\": \"Never\""));

        Ok(())
    }

    #[test]
    fn execute_inspect_artifacts_writes_json() -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let cli = Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Artifacts(InspectArtifactsArgs {
                    path: root,
                    format: OutputFormat::Json,
                }),
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("\"artifacts\""));
        assert!(output.contains("\"category\": \"ContainerDefinition\""));

        Ok(())
    }

    #[test]
    fn execute_drift_reports_no_drift_on_the_clean_fixture()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let cli = Cli {
            command: Some(Command::Drift(DriftArgs {
                path: root,
                format: OutputFormat::Table,
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert_eq!(output, "no drift detected\n");

        Ok(())
    }

    #[test]
    fn execute_drift_json_reports_a_finding_on_a_repo_with_drift()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        std::fs::write(
            temp.path().join("docs/broken.md"),
            "See [missing](./does-not-exist.md) for details.\n",
        )?;
        let cli = Cli {
            command: Some(Command::Drift(DriftArgs {
                path: temp.path().to_path_buf(),
                format: OutputFormat::Json,
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("\"BrokenLink\""));
        assert!(output.contains("does-not-exist.md"));

        Ok(())
    }

    /// Regression test: `drift`/`inspect artifacts`/`inspect graph`/
    /// `inspect modules` each used to walk with a bare `WalkOptions::default()`
    /// -- unlike `init`/`update`'s `scan_and_plan`, which already excludes
    /// `.lithograph/**` and `docs/lithograph/**` via `scan_exclude_globs()`.
    /// On a repository that had already been `init`ed, this meant every one
    /// of these commands re-ingested `.lithograph/cache/analysis/*.json`
    /// (Lithograph's own cached analysis output) as if it were repository
    /// source, generating thousands of spurious graph nodes from JSON that
    /// happened to look like config/image/port values. Proven live against
    /// an external repository via the LIT-22 comparison against
    /// codebase-memory-mcp on `ridgeline`, where this single bug accounted
    /// for the overwhelming majority of a 79,301-vs-2,697 node-count gap.
    #[test]
    fn drift_and_inspect_commands_never_rescan_lithographs_own_output()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        execute(
            Cli {
                command: Some(Command::Init(InitArgs {
                    path: temp.path().to_path_buf(),
                    prompt_version: "v1".to_owned(),
                    semantic_grouping: false,
                })),
            },
            &mut Vec::new(),
        )?;
        assert!(
            std::fs::read_dir(temp.path().join(".lithograph/cache/analysis"))?
                .next()
                .is_some(),
            "expected init to have populated the analysis cache"
        );

        let mut artifacts_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Inspect(InspectCommand {
                    target: InspectTarget::Artifacts(InspectArtifactsArgs {
                        path: temp.path().to_path_buf(),
                        format: OutputFormat::Json,
                    }),
                })),
            },
            &mut artifacts_output,
        )?;
        let artifacts_output = String::from_utf8(artifacts_output)?;
        assert!(!artifacts_output.contains(".lithograph/"));
        assert!(!artifacts_output.contains("docs/lithograph/"));

        let mut graph_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Inspect(InspectCommand {
                    target: InspectTarget::Graph(InspectGraphArgs {
                        path: temp.path().to_path_buf(),
                        format: OutputFormat::Json,
                    }),
                })),
            },
            &mut graph_output,
        )?;
        let graph_output = String::from_utf8(graph_output)?;
        assert!(!graph_output.contains(".lithograph/"));
        assert!(!graph_output.contains("docs/lithograph/"));

        let mut modules_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Inspect(InspectCommand {
                    target: InspectTarget::Modules(InspectModulesArgs {
                        path: temp.path().to_path_buf(),
                        semantic_grouping: false,
                        format: OutputFormat::Json,
                    }),
                })),
            },
            &mut modules_output,
        )?;
        let modules_output = String::from_utf8(modules_output)?;
        assert!(!modules_output.contains(".lithograph/"));
        assert!(!modules_output.contains("docs/lithograph/"));

        // `drift` must still see docs/lithograph/*.md (its entire purpose is
        // comparing generated docs against current repository facts), while
        // never treating .lithograph/cache/**/*.json as a Makefile,
        // package.json, or graph-building input.
        let mut drift_output = Vec::new();
        execute(
            Cli {
                command: Some(Command::Drift(DriftArgs {
                    path: temp.path().to_path_buf(),
                    format: OutputFormat::Json,
                })),
            },
            &mut drift_output,
        )?;
        assert!(!String::from_utf8(drift_output)?.contains(".lithograph/"));
        assert!(temp.path().join("docs/lithograph/quickstart.md").exists());

        Ok(())
    }

    /// LIT-22.5.4 AC1/AC4: exercises create -> get -> update section ->
    /// update status -> list -> delete -> list through the CLI end to end.
    #[test]
    fn execute_adr_create_get_update_list_delete_round_trips()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().to_path_buf();

        let mut created = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::Create(AdrCreateArgs {
                        path: path.clone(),
                        title: "Use Postgres".to_owned(),
                        context: "We need a database.".to_owned(),
                        decision: "Use Postgres.".to_owned(),
                        consequences: None,
                        format: OutputFormat::Json,
                    }),
                })),
            },
            &mut created,
        )?;
        let created: crate::adr::AdrRecord = serde_json::from_slice(&created)?;
        assert_eq!(created.id, "ADR-0001");
        assert_eq!(created.status, crate::adr::AdrStatus::Proposed);

        let mut got = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::Get(AdrGetArgs {
                        path: path.clone(),
                        id: created.id.clone(),
                        format: OutputFormat::Table,
                    }),
                })),
            },
            &mut got,
        )?;
        let got = String::from_utf8(got)?;
        assert!(got.contains("Use Postgres"));
        assert!(got.contains("Proposed"));

        let mut updated = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::Update(AdrUpdateArgs {
                        path: path.clone(),
                        id: created.id.clone(),
                        section: Some("consequences".to_owned()),
                        value: Some("Adds an ops dependency.".to_owned()),
                        status: Some(AdrStatusArg::Accepted),
                        format: OutputFormat::Json,
                    }),
                })),
            },
            &mut updated,
        )?;
        let updated: crate::adr::AdrRecord = serde_json::from_slice(&updated)?;
        assert_eq!(updated.status, crate::adr::AdrStatus::Accepted);
        assert_eq!(
            updated.sections.get("consequences").map(String::as_str),
            Some("Adds an ops dependency.")
        );

        let mut listed = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::List(AdrListArgs {
                        path: path.clone(),
                        format: OutputFormat::Table,
                    }),
                })),
            },
            &mut listed,
        )?;
        assert!(String::from_utf8(listed)?.contains("ADR-0001"));

        let mut deleted = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::Delete(AdrDeleteArgs {
                        path: path.clone(),
                        id: created.id.clone(),
                    }),
                })),
            },
            &mut deleted,
        )?;
        assert!(String::from_utf8(deleted)?.contains("deleted ADR-0001"));

        let mut listed_after_delete = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::List(AdrListArgs {
                        path,
                        format: OutputFormat::Table,
                    }),
                })),
            },
            &mut listed_after_delete,
        )?;
        assert_eq!(
            String::from_utf8(listed_after_delete)?,
            "no ADRs recorded\n"
        );

        Ok(())
    }

    /// LIT-22.5.4 AC2/AC4: an unknown section key surfaces as an actionable
    /// CLI error rather than succeeding silently.
    #[test]
    fn execute_adr_update_rejects_unknown_section() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().to_path_buf();
        let mut created = Vec::new();
        execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::Create(AdrCreateArgs {
                        path: path.clone(),
                        title: "Use Postgres".to_owned(),
                        context: "We need a database.".to_owned(),
                        decision: "Use Postgres.".to_owned(),
                        consequences: None,
                        format: OutputFormat::Json,
                    }),
                })),
            },
            &mut created,
        )?;
        let created: crate::adr::AdrRecord = serde_json::from_slice(&created)?;

        let mut output = Vec::new();
        let result = execute(
            Cli {
                command: Some(Command::Adr(AdrCommand {
                    target: AdrTarget::Update(AdrUpdateArgs {
                        path,
                        id: created.id,
                        section: Some("not-a-real-section".to_owned()),
                        value: Some("value".to_owned()),
                        status: None,
                        format: OutputFormat::Table,
                    }),
                })),
            },
            &mut output,
        );
        match result {
            Ok(()) => return Err("expected an unknown-section error".into()),
            Err(error) => assert!(error.to_string().contains("unknown ADR section")),
        }

        Ok(())
    }

    #[test]
    fn execute_without_command_writes_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let cli = Cli { command: None };
        let mut output = Vec::new();

        execute(cli, &mut output)?;

        assert!(output.is_empty());

        Ok(())
    }

    #[test]
    fn execute_inspect_graph_writes_json() -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let cli = Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Graph(InspectGraphArgs {
                    path: root,
                    format: OutputFormat::Json,
                }),
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("\"nodes\""));
        assert!(output.contains("\"node_type\": \"Artifact\""));

        Ok(())
    }

    #[test]
    fn execute_inspect_graph_table_lists_node_counts() -> Result<(), Box<dyn std::error::Error>> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let cli = Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Graph(InspectGraphArgs {
                    path: root,
                    format: OutputFormat::Table,
                }),
            })),
        };
        let mut output = Vec::new();

        execute(cli, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("nodes:"));
        assert!(output.contains("relations:"));
        assert!(output.contains("artifact"));
        assert!(output.contains("symbol"));

        Ok(())
    }

    fn inspect_metrics_args(
        path: &Path,
        max_graph_nodes: Option<usize>,
        min_cache_hit_rate_percent: Option<u8>,
    ) -> InspectMetricsArgs {
        InspectMetricsArgs {
            path: path.to_path_buf(),
            max_graph_nodes,
            max_graph_relations: None,
            min_cache_hit_rate_percent,
            max_tokens: None,
            format: OutputFormat::Table,
        }
    }

    #[test]
    fn execute_inspect_metrics_reports_persisted_run_metadata()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        execute(
            Cli {
                command: Some(Command::Init(InitArgs {
                    path: temp.path().to_path_buf(),
                    prompt_version: "v1".to_owned(),
                    semantic_grouping: false,
                })),
            },
            &mut Vec::new(),
        )?;
        let mut output = Vec::new();

        execute(
            Cli {
                command: Some(Command::Inspect(InspectCommand {
                    target: InspectTarget::Metrics(inspect_metrics_args(temp.path(), None, None)),
                })),
            },
            &mut output,
        )?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("graph nodes:"));
        assert!(output.contains("cache hit rate:"));
        assert!(output.contains("estimated prompt tokens:"));
        assert!(output.contains("stage PreprocessIndex:"));
        assert!(output.contains("within every configured threshold"));

        Ok(())
    }

    #[test]
    fn execute_inspect_metrics_without_a_prior_run_reports_an_actionable_error()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        match execute(
            Cli {
                command: Some(Command::Inspect(InspectCommand {
                    target: InspectTarget::Metrics(inspect_metrics_args(temp.path(), None, None)),
                })),
            },
            &mut Vec::new(),
        ) {
            Ok(()) => Err("expected a missing-run-metadata error".into()),
            Err(error) => {
                assert!(error.to_string().contains("run `init` or `update` first"));
                Ok(())
            }
        }
    }

    #[test]
    fn execute_inspect_metrics_fails_only_through_the_configured_budget_threshold()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        execute(
            Cli {
                command: Some(Command::Init(InitArgs {
                    path: temp.path().to_path_buf(),
                    prompt_version: "v1".to_owned(),
                    semantic_grouping: false,
                })),
            },
            &mut Vec::new(),
        )?;
        let within_budget = Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Metrics(inspect_metrics_args(
                    temp.path(),
                    Some(usize::MAX),
                    None,
                )),
            })),
        };
        let mut within_output = Vec::new();
        execute(within_budget, &mut within_output)?;
        assert!(String::from_utf8(within_output)?.contains("within every configured threshold"));

        let over_budget = Cli {
            command: Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Metrics(inspect_metrics_args(temp.path(), Some(0), None)),
            })),
        };
        match execute(over_budget, &mut Vec::new()) {
            Ok(()) => Err("expected a budget-violation error".into()),
            Err(error) => {
                assert!(error.to_string().contains("graph_node_count"));
                Ok(())
            }
        }
    }

    #[test]
    fn render_graph_diagnostics_lists_each_issue_actionably() {
        let issues = vec![
            GraphIssue {
                kind: GraphIssueKind::DanglingRelationTarget,
                message: "relation:1 has target symbol:missing which is not a graph node"
                    .to_owned(),
            },
            GraphIssue {
                kind: GraphIssueKind::InvalidSourceSpan,
                message:
                    "evidence for src/lib.rs spans lines 1-100 but the artifact has only 5 lines"
                        .to_owned(),
            },
        ];

        let message = render_graph_diagnostics(&issues);

        assert!(message.contains("2 issue(s)"));
        assert!(message.contains("DanglingRelationTarget"));
        assert!(message.contains("symbol:missing"));
        assert!(message.contains("InvalidSourceSpan"));
        assert!(message.contains("only 5 lines"));
    }

    /// LIT-22.7.2 AC3: `validate-mermaid --fix` rewrites unsafe node ids
    /// in place before re-validating; without `--fix`, the same command
    /// leaves the file untouched and still fails.
    #[test]
    fn validate_mermaid_fix_flag_rewrites_ids_then_passes() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("diagram.md"),
            "```mermaid\nflowchart TD\n  caf\u{e9}[\"Overview\"]\n```\n",
        )?;

        let without_fix = Cli {
            command: Some(Command::ValidateMermaid(ValidateMermaidArgs {
                path: temp.path().to_path_buf(),
                node_validator: None,
                fix: false,
            })),
        };
        let mut output = Vec::new();
        assert!(execute(without_fix, &mut output).is_err());
        assert!(
            std::fs::read_to_string(temp.path().join("diagram.md"))?.contains('\u{e9}'),
            "without --fix the file must be untouched"
        );

        let with_fix = Cli {
            command: Some(Command::ValidateMermaid(ValidateMermaidArgs {
                path: temp.path().to_path_buf(),
                node_validator: None,
                fix: true,
            })),
        };
        let mut output = Vec::new();
        execute(with_fix, &mut output)?;
        let output = String::from_utf8(output)?;

        assert!(output.contains("fixed node ids in 1 file(s)"));
        assert!(
            !std::fs::read_to_string(temp.path().join("diagram.md"))?.contains('\u{e9}'),
            "--fix must rewrite the unsafe id"
        );

        Ok(())
    }
}
