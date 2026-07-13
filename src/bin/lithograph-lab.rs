//! Development CLI for deterministic Lithograph corpus and baseline diagnostics.

use clap::{Args, Parser, Subcommand, ValueEnum};
use lithograph::lab::{BaselineDiff, BenchmarkMode, Corpus, Lab, RunManifest, SuiteTier};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

#[derive(Debug, Parser)]
#[command(
    name = "lithograph-lab",
    about = "Deterministic Lithograph baseline and diagnostic lab"
)]
struct Cli {
    #[arg(long, default_value = "lab/corpus.toml")]
    manifest: PathBuf,
    #[arg(long, default_value = ".lithograph-lab")]
    root: PathBuf,
    #[arg(long, default_value = ".lithograph-lab/corpus")]
    cache: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Manage pinned external repositories.
    Corpus(CorpusArgs),
    /// Execute selected cases and persist content-addressed runs.
    Run(SelectionArgs),
    /// Execute cases and compare them with committed baselines, or check one run.
    Check(CheckArgs),
    /// Show the semantic baseline difference for one run.
    Diff(RunArg),
    /// Inspect a run manifest or one graph-build stage.
    Inspect(InspectArgs),
    /// Explain one expectation result and show reproduction commands.
    Explain(ExplainArgs),
    /// Re-execute the exact case and tier recorded by a run.
    Replay(RunArg),
    /// Explicitly accept one clean run as the case baseline.
    Accept(AcceptArgs),
    /// Serve read-only lab inspection tools as JSON lines.
    Mcp,
    /// Record machine-specific median/MAD performance observations.
    Benchmark(BenchmarkArgs),
    /// Preview or apply a mechanical lab JSON schema migration.
    Migrate(MigrateArgs),
    /// Reduce a failing run to a source-free diagnostic slice.
    Minimize(MinimizeArgs),
}

#[derive(Debug, Args)]
struct CorpusArgs {
    #[command(subcommand)]
    command: CorpusCommand,
}

#[derive(Debug, Subcommand)]
enum CorpusCommand {
    /// Fetch and verify immutable Git cases.
    Fetch(SelectionArgs),
}

#[derive(Debug, Clone, Args)]
struct SelectionArgs {
    #[arg(long, value_enum, default_value_t = TierArg::Pr)]
    suite: TierArg,
    #[arg(long)]
    case: Option<String>,
}

#[derive(Debug, Args)]
struct CheckArgs {
    /// Existing run directory or id. When omitted, selected cases are run first.
    run: Option<PathBuf>,
    #[command(flatten)]
    selection: SelectionArgs,
}

#[derive(Debug, Args)]
struct RunArg {
    /// Run directory or content id.
    run: PathBuf,
}

#[derive(Debug, Args)]
struct InspectArgs {
    /// Run directory or content id.
    run: PathBuf,
    /// Optional case-insensitive graph pass name.
    #[arg(long)]
    stage: Option<String>,
}

#[derive(Debug, Args)]
struct ExplainArgs {
    /// Run directory or content id.
    run: PathBuf,
    /// Expectation id.
    #[arg(long)]
    assertion: String,
}

#[derive(Debug, Args)]
struct AcceptArgs {
    /// Run directory or content id.
    run: PathBuf,
    /// Human review reason recorded with the baseline.
    #[arg(long)]
    reason: String,
    /// Fresh token printed by the review-only first invocation.
    #[arg(long)]
    confirm: Option<String>,
}

#[derive(Debug, Args)]
struct BenchmarkArgs {
    #[command(flatten)]
    selection: SelectionArgs,
    /// Number of warm samples.
    #[arg(long, default_value_t = 5)]
    samples: usize,
    /// Cache/update mode represented by every raw sample.
    #[arg(long, value_enum, default_value_t = ModeArg::WarmCache)]
    mode: ModeArg,
    /// Fail when reviewed relative thresholds regress on this machine history.
    #[arg(long)]
    gate: bool,
}

#[derive(Debug, Args)]
struct MigrateArgs {
    /// Run, baseline, replay, or expectation JSON artifact.
    path: PathBuf,
    /// Apply the previewed mechanical changes. Never accepts a baseline.
    #[arg(long)]
    apply: bool,
}

#[derive(Debug, Args)]
struct MinimizeArgs {
    /// Failing run directory or content id.
    run: PathBuf,
    /// Explicit local-only directory for copying relevant third-party files.
    #[arg(long)]
    materialize: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Cold,
    WarmCache,
    Incremental,
    NoOp,
    CommunityOnly,
}

impl From<ModeArg> for BenchmarkMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Cold => Self::Cold,
            ModeArg::WarmCache => Self::WarmCache,
            ModeArg::Incremental => Self::Incremental,
            ModeArg::NoOp => Self::NoOp,
            ModeArg::CommunityOnly => Self::CommunityOnly,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum TierArg {
    Pr,
    Merge,
    Nightly,
}

impl From<TierArg> for SuiteTier {
    fn from(value: TierArg) -> Self {
        match value {
            TierArg::Pr => Self::Pr,
            TierArg::Merge => Self::Merge,
            TierArg::Nightly => Self::Nightly,
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let corpus = Corpus::load(&cli.manifest, &cli.cache)?;
    let lab = Lab::new(corpus, cli.root);
    let mut stdout = std::io::stdout().lock();
    match cli.command {
        Command::Corpus(CorpusArgs {
            command: CorpusCommand::Fetch(args),
        }) => {
            let cases = lab
                .corpus
                .cases(args.suite.into(), args.case.as_deref())
                .into_iter()
                .cloned()
                .collect::<Vec<_>>();
            let fetched = cases
                .iter()
                .map(|case| {
                    lab.corpus
                        .fetch(case)
                        .map(|path| json!({"case": case.id, "path": path, "verified": true}))
                })
                .collect::<Result<Vec<_>, _>>()?;
            write_json(&mut stdout, &fetched)?;
        }
        Command::Run(args) => {
            let paths = lab.run(args.suite.into(), args.case.as_deref())?;
            write_json(&mut stdout, &paths)?;
        }
        Command::Check(args) => {
            let manifests = if let Some(run) = args.run {
                vec![lab.load_run(&run)?]
            } else {
                lab.run(args.selection.suite.into(), args.selection.case.as_deref())?
                    .iter()
                    .map(|path| lab.load_run(path))
                    .collect::<Result<Vec<_>, _>>()?
            };
            let diffs = manifests
                .iter()
                .map(|manifest| lab.check(manifest))
                .collect::<Result<Vec<_>, _>>()?;
            write_json(&mut stdout, &diffs)?;
            fail_on_dirty(&manifests, &diffs)?;
        }
        Command::Diff(args) => {
            let run = lab.load_run(&args.run)?;
            write_json(&mut stdout, &lab.check(&run)?)?;
        }
        Command::Inspect(args) => {
            let run = lab.load_run(&args.run)?;
            if let Some(stage) = args.stage {
                let trace = run
                    .stages
                    .iter()
                    .find(|trace| trace.pass.as_str().eq_ignore_ascii_case(&stage))
                    .ok_or_else(|| format!("run has no stage `{stage}`"))?;
                write_json(&mut stdout, trace)?;
            } else {
                write_json(&mut stdout, &run)?;
            }
        }
        Command::Explain(args) => {
            let run = lab.load_run(&args.run)?;
            write_json(&mut stdout, &lab.explain(&run, &args.assertion)?)?;
        }
        Command::Replay(args) => {
            let run = lab.load_run(&args.run)?;
            write_json(&mut stdout, &lab.replay(&run)?)?;
        }
        Command::Accept(args) => {
            let run = lab.load_run(&args.run)?;
            if let Some(token) = args.confirm {
                write_json(&mut stdout, &lab.accept(&run, &args.reason, &token)?)?;
            } else {
                write_json(&mut stdout, &lab.acceptance_review(&run, &args.reason)?)?;
            }
        }
        Command::Mcp => serve_mcp(&lab, &mut stdout)?,
        Command::Benchmark(args) => {
            write_json(
                &mut stdout,
                &lab.benchmark(
                    args.selection.suite.into(),
                    args.selection.case.as_deref(),
                    args.samples,
                    args.mode.into(),
                    args.gate,
                )?,
            )?;
        }
        Command::Migrate(args) => {
            write_json(&mut stdout, &lab.migrate(&args.path, args.apply)?)?;
        }
        Command::Minimize(args) => {
            let run = lab.load_run(&args.run)?;
            write_json(
                &mut stdout,
                &lab.minimize(&run, args.materialize.as_deref())?,
            )?;
        }
    }
    Ok(())
}

fn fail_on_dirty(
    runs: &[RunManifest],
    diffs: &[BaselineDiff],
) -> Result<(), Box<dyn std::error::Error>> {
    let failures = runs
        .iter()
        .zip(diffs)
        .filter(|(run, diff)| !run.is_clean() || !diff.is_clean())
        .map(|(run, diff)| {
            format!(
                "{}: first divergent stage={:?}; replay `{}`; explain failed assertions with `cargo run --bin lithograph-lab -- explain {} --assertion <id>`",
                run.case_id, diff.first_divergent_stage, run.reproduce, run.run_id
            )
        })
        .collect::<Vec<_>>();
    if failures.is_empty() {
        Ok(())
    } else {
        Err(failures.join("\n").into())
    }
}

#[derive(Debug, Deserialize)]
struct McpRequest {
    jsonrpc: String,
    #[serde(default)]
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

fn serve_mcp(lab: &Lab, writer: &mut impl Write) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let request: McpRequest = match serde_json::from_str(&line) {
            Ok(request) => request,
            Err(error) => {
                write_mcp_json(
                    writer,
                    &json!({"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":error.to_string()}}),
                )?;
                continue;
            }
        };
        if request.jsonrpc != "2.0" {
            write_mcp_json(
                writer,
                &json!({"jsonrpc":"2.0","id":request.id,"error":{"code":-32600,"message":"jsonrpc must be 2.0"}}),
            )?;
            continue;
        }
        let Some(id) = request.id.clone() else {
            continue;
        };
        if !matches!(
            request.method.as_str(),
            "initialize" | "ping" | "tools/list" | "tools/call"
        ) {
            write_mcp_json(
                writer,
                &json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("unknown MCP method `{}`", request.method)}}),
            )?;
            continue;
        }
        let response = match handle_mcp(lab, &request) {
            Ok(value) => json!({"jsonrpc":"2.0","id":id,"result":value}),
            Err(error) => {
                json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":error.to_string()}})
            }
        };
        write_mcp_json(writer, &response)?;
    }
    Ok(())
}

fn handle_mcp(lab: &Lab, request: &McpRequest) -> Result<Value, Box<dyn std::error::Error>> {
    match request.method.as_str() {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "lithograph-lab", "version": env!("CARGO_PKG_VERSION")}
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": mcp_tools()})),
        "tools/call" => {
            let name = required_param(&request.params, "name")?;
            let arguments = request.params.get("arguments").unwrap_or(&Value::Null);
            let value = handle_mcp_tool(lab, name, arguments)?;
            Ok(json!({
                "content": [{"type": "text", "text": serde_json::to_string_pretty(&value)?}],
                "isError": false
            }))
        }
        unknown => Err(format!("unknown MCP method `{unknown}`").into()),
    }
}

fn mcp_tools() -> Value {
    json!([
        {"name":"list_runs","description":"Lists content-addressed lab runs.","inputSchema":{"type":"object","properties":{},"additionalProperties":false}},
        {"name":"inspect_run","description":"Reads a run manifest.","inputSchema":{"type":"object","properties":{"run":{"type":"string"}},"required":["run"],"additionalProperties":false}},
        {"name":"inspect_stage","description":"Reads one graph-build stage.","inputSchema":{"type":"object","properties":{"run":{"type":"string"},"stage":{"type":"string"}},"required":["run","stage"],"additionalProperties":false}},
        {"name":"diff_run","description":"Diffs a run against its accepted baseline.","inputSchema":{"type":"object","properties":{"run":{"type":"string"}},"required":["run"],"additionalProperties":false}},
        {"name":"explain_assertion","description":"Explains one assertion.","inputSchema":{"type":"object","properties":{"run":{"type":"string"},"assertion":{"type":"string"}},"required":["run","assertion"],"additionalProperties":false}}
    ])
}

fn handle_mcp_tool(
    lab: &Lab,
    tool: &str,
    params: &Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    match tool {
        "list_runs" => Ok(serde_json::to_value(lab.list_runs()?)?),
        "inspect_run" => Ok(serde_json::to_value(load_param_run(lab, params)?)?),
        "inspect_stage" => {
            let run = load_param_run(lab, params)?;
            let stage = required_param(params, "stage")?;
            let trace = run
                .stages
                .iter()
                .find(|trace| trace.pass.as_str().eq_ignore_ascii_case(stage))
                .ok_or_else(|| format!("run has no stage `{stage}`"))?;
            Ok(serde_json::to_value(trace)?)
        }
        "diff_run" => {
            let run = load_param_run(lab, params)?;
            Ok(serde_json::to_value(lab.check(&run)?)?)
        }
        "explain_assertion" => {
            let run = load_param_run(lab, params)?;
            Ok(lab.explain(&run, required_param(params, "assertion")?)?)
        }
        unknown => Err(format!("unknown lab tool `{unknown}`").into()),
    }
}

fn write_mcp_json(writer: &mut impl Write, value: &Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *writer, value).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")
}

fn load_param_run(lab: &Lab, params: &Value) -> Result<RunManifest, Box<dyn std::error::Error>> {
    Ok(lab.load_run(Path::new(required_param(params, "run")?))?)
}

fn required_param<'a>(params: &'a Value, key: &str) -> Result<&'a str, Box<dyn std::error::Error>> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string params.{key}").into())
}

fn write_json(writer: &mut impl Write, value: &impl serde::Serialize) -> std::io::Result<()> {
    serde_json::to_writer_pretty(&mut *writer, value).map_err(std::io::Error::other)?;
    writer.write_all(b"\n")
}
