//! `inspect`: print deterministic repository inventory, graph, environment,
//! module plan, dependency matrix, and run metrics data.

use crate::cli::{
    InspectArtifactsArgs, InspectCommand, InspectDsmArgs, InspectEnvArgs, InspectGraphArgs,
    InspectMetricsArgs, InspectModulesArgs, InspectTarget, OutputFormat,
};
use crate::domain::Artifact;
use crate::graph::{
    DependencyMatrix, Graph, GraphBuilder, GraphIssue, GraphNode, GraphValidator, KnowledgeIndex,
};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::plan::{DocumentationModule, ModulePlanner};
use crate::resolve::{EnvironmentExplanation, explain_environment};
use crate::run::{PerformanceBudget, RunMetadata};
use crate::storage::JsonStore;
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Write;

pub(crate) fn execute_inspect<W>(
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
fn render_metrics_table(
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
fn render_modules_table(modules: &[DocumentationModule]) -> String {
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
fn render_environment_table(explanation: &EnvironmentExplanation) -> String {
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
fn render_dsm_table(matrix: &DependencyMatrix) -> String {
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
pub(crate) fn render_graph_diagnostics(issues: &[GraphIssue]) -> String {
    let mut message = format!("graph validation failed with {} issue(s):\n", issues.len());
    for issue in issues {
        message.push_str(&format!("  - [{:?}] {issue}\n", issue.kind));
    }
    message
}

/// Renders a deterministic human-readable graph node/relation summary.
fn render_graph_table(graph: &Graph) -> String {
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
        GraphNode::Rationale(_) => "rationale",
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
pub(crate) fn render_artifacts_table(artifacts: &[Artifact]) -> String {
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
pub(crate) fn render_artifacts_json(artifacts: &[Artifact]) -> Result<String, serde_json::Error> {
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
