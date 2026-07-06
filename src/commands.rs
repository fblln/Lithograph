//! Command execution and output rendering.

use crate::agents::{AgentFileOutcome, IntegrateAgentsReport, integrate_agents};
use crate::ask::{McpExport, WikiSearch, render_ask_table};
use crate::cli::{
    AskArgs, Cli, Command, DriftArgs, GoldenArgs, InitArgs, InspectArtifactsArgs, InspectCommand,
    InspectGraphArgs, InspectModulesArgs, InspectTarget, IntegrateAgentsArgs, McpExportArgs,
    McpServerArgs, OutputFormat, QualityArgs, ValidateMermaidArgs, ViewerArgs,
};
use crate::domain::Artifact;
use crate::drift::{DriftDetector, DriftReport};
use crate::generation::{
    DeepInfraConfig, DeepInfraModel, LanguageModel, MockModel, OpenAiConfig, OpenAiModel,
};
use crate::golden::{check_or_update, render_report as render_golden_report};
use crate::graph::{Graph, GraphBuilder, GraphIssue, GraphNode, GraphValidator};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::mcp::WikiMcpServer;
use crate::mermaid::{render_report as render_mermaid_report, validate as validate_mermaid};
use crate::orchestrate::{
    InitReport, UpdateReport, run_init_with_options, run_update_with_options,
};
use crate::plan::{DocumentationModule, ModulePlanner};
use crate::quality::{inspect as inspect_quality, render_table as render_quality_table};
use crate::viewer::{generate as generate_viewer, render_report as render_viewer_report};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;

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
        None => Ok(()),
    }
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
    let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&args.path)?;
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
        InspectTarget::Modules(args) => execute_inspect_modules(args, writer),
    }
}

fn execute_inspect_modules<W>(
    args: InspectModulesArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&args.path)?;
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
    let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&args.path)?;
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
    let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&args.path)?;
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
    use super::{execute, render_artifacts_json, render_artifacts_table, render_graph_diagnostics};
    use crate::cli::{
        AskArgs, Cli, Command, DriftArgs, InitArgs, InspectArtifactsArgs, InspectCommand,
        InspectGraphArgs, InspectModulesArgs, InspectTarget, IntegrateAgentsArgs, McpExportArgs,
        OutputFormat,
    };
    use crate::graph::{GraphIssue, GraphIssueKind};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::{Path, PathBuf};

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
        assert!(
            parsed["tools"]
                .as_array()
                .is_some_and(|tools| tools.len() == 3)
        );
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
}
