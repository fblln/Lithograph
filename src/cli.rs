//! Command-line argument definitions.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Repository knowledge compiler that builds evidence-backed documentation.
#[derive(Debug, Parser)]
#[command(name = "lithograph")]
#[command(version)]
#[command(about = "Compile repository knowledge into evidence-backed documentation.")]
#[command(long_about = None)]
pub(crate) struct Cli {
    /// Command to run.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level Lithograph commands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub(crate) enum Command {
    /// Scan, analyze, plan, generate, and write documentation for a repository.
    Init(InitArgs),
    /// Rescan and selectively regenerate documentation for changed content.
    Update(InitArgs),
    /// Inspect deterministic repository inventory data.
    Inspect(InspectCommand),
    /// Add or refresh a Lithograph reference section in top-level
    /// `AGENTS.md`/`CLAUDE.md`. The only command allowed to edit those files.
    IntegrateAgents(IntegrateAgentsArgs),
    /// Scan Markdown documentation for likely drift against the current
    /// repository and graph. Deterministic: never calls a language model.
    Drift(DriftArgs),
    /// Ask a deterministic local question against generated Lithograph docs.
    Ask(AskArgs),
    /// Export generated wiki data in an MCP-style JSON shape.
    McpExport(McpExportArgs),
    /// Check or update deterministic golden snapshots for generated output.
    Golden(GoldenArgs),
    /// Inspect generated wiki quality without model calls.
    Quality(QualityArgs),
    /// Validate Mermaid fences, optionally through a local Node validator.
    ValidateMermaid(ValidateMermaidArgs),
    /// Serve deterministic MCP requests over stdin/stdout JSON lines.
    McpServer(McpServerArgs),
    /// Generate a lightweight static viewer for generated docs.
    Viewer(ViewerArgs),
    /// Serve the graph explorer UI and read-only graph APIs locally.
    /// Binds `127.0.0.1` only; never reachable from another machine.
    Serve(ServeArgs),
    /// Export or import team-shareable graph artifacts.
    Graph(GraphCommand),
    /// Show the shortest chain of relations connecting two graph nodes.
    Path(PathArgs),
    /// Explain one graph node: its evidence, and what it connects to.
    Explain(ExplainArgs),
    /// List what depends on the given nodes or changed files -- "what breaks
    /// if this changes".
    Affected(AffectedArgs),
    /// Create, read, update, delete, and list architecture decision records.
    Adr(AdrCommand),
    /// Record answer outcomes and reflect them into reusable research lessons.
    Research(ResearchCommand),
    /// Poll a repository for staleness against its last recorded snapshot.
    /// Disabled by default beyond this explicit command: reports staleness
    /// only, unless `--auto-index` is passed.
    Watch(WatchArgs),
    /// Detect, preview, or apply per-agent MCP server integration (Codex,
    /// Claude, Gemini, Zed). Without `--target`, only detects and reports;
    /// `--apply` requires `--target` and is the only way anything is written.
    IntegrateMcp(IntegrateMcpArgs),
}

/// Research feedback command namespace.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ResearchCommand {
    /// Feedback operation.
    #[command(subcommand)]
    pub target: ResearchTarget,
}

/// Research feedback operations.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub(crate) enum ResearchTarget {
    /// Record the observed outcome of one answer.
    SaveResult(ResearchSaveResultArgs),
    /// Aggregate recorded outcomes into deterministic lessons.
    Reflect(ResearchReflectArgs),
}

/// Arguments for `research save-result`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ResearchSaveResultArgs {
    /// Repository path.
    pub path: PathBuf,
    /// Question that was answered.
    #[arg(long)]
    pub question: String,
    /// Answer that was evaluated.
    #[arg(long)]
    pub answer: String,
    /// Cited graph node ids, comma-delimited or repeated.
    #[arg(long = "node", value_delimiter = ',')]
    pub cited_node_ids: Vec<String>,
    /// Observed outcome.
    #[arg(long, value_enum)]
    pub outcome: ResearchOutcomeArg,
    /// Replacement guidance; required for a corrected outcome.
    #[arg(long)]
    pub correction: Option<String>,
    /// Unix timestamp in seconds; defaults to the current time.
    #[arg(long)]
    pub recorded_at: Option<u64>,
}

/// Arguments for `research reflect`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ResearchReflectArgs {
    /// Repository path.
    pub path: PathBuf,
    /// Unix timestamp in seconds used for all decay calculations.
    #[arg(long)]
    pub now: Option<u64>,
}

/// CLI spelling for an answer outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "snake_case")]
pub(crate) enum ResearchOutcomeArg {
    /// The answer and its cited sources were useful.
    Useful,
    /// The cited sources led to a dead end.
    DeadEnd,
    /// The answer needed explicit replacement guidance.
    Corrected,
}

impl From<ResearchOutcomeArg> for crate::research_feedback::AnswerOutcome {
    fn from(value: ResearchOutcomeArg) -> Self {
        match value {
            ResearchOutcomeArg::Useful => Self::Useful,
            ResearchOutcomeArg::DeadEnd => Self::DeadEnd,
            ResearchOutcomeArg::Corrected => Self::Corrected,
        }
    }
}

/// Arguments for `integrate-mcp`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct IntegrateMcpArgs {
    /// Repository path to integrate.
    pub path: PathBuf,
    /// Agent target id (`codex`, `claude`, `gemini`, `zed`, `aider`). When
    /// omitted, every target is detected and reported without writing.
    #[arg(long)]
    pub target: Option<String>,
    /// Write the target's merged MCP config. Requires `--target`; without
    /// this flag a given `--target` is only previewed, never written.
    #[arg(long, requires = "target")]
    pub apply: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `watch`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct WatchArgs {
    /// Repository path to watch.
    pub path: PathBuf,
    /// Maximum artifacts one poll may scan before refusing to proceed.
    #[arg(long, default_value_t = 20_000)]
    pub max_artifacts: usize,
    /// Seconds to wait between polls when watching continuously.
    #[arg(long, default_value_t = 5)]
    pub interval_secs: u64,
    /// Poll exactly once and exit, instead of watching continuously.
    #[arg(long)]
    pub once: bool,
    /// Automatically run `update` when staleness is detected. Disabled by
    /// default: without this flag, `watch` only reports staleness.
    #[arg(long)]
    pub auto_index: bool,
}

/// ADR command namespace.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AdrCommand {
    /// ADR operation.
    #[command(subcommand)]
    pub target: AdrTarget,
}

/// ADR operations.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub(crate) enum AdrTarget {
    /// Create a new ADR.
    Create(AdrCreateArgs),
    /// Read one ADR by id.
    Get(AdrGetArgs),
    /// Update one ADR's section content or status.
    Update(AdrUpdateArgs),
    /// Delete one ADR by id.
    Delete(AdrDeleteArgs),
    /// List every ADR.
    List(AdrListArgs),
}

/// Arguments for `adr create`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AdrCreateArgs {
    /// Repository path.
    pub path: PathBuf,
    /// Short decision title.
    #[arg(long)]
    pub title: String,
    /// Context section content.
    #[arg(long)]
    pub context: String,
    /// Decision section content.
    #[arg(long)]
    pub decision: String,
    /// Optional consequences section content.
    #[arg(long)]
    pub consequences: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `adr get`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AdrGetArgs {
    /// Repository path.
    pub path: PathBuf,
    /// ADR id, e.g. `ADR-0001`.
    pub id: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `adr update`. Provide either `--section`/`--value`, or
/// `--status`, or both.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AdrUpdateArgs {
    /// Repository path.
    pub path: PathBuf,
    /// ADR id, e.g. `ADR-0001`.
    pub id: String,
    /// Section to update: `context`, `decision`, or `consequences`.
    #[arg(long, requires = "value")]
    pub section: Option<String>,
    /// New content for `--section`.
    #[arg(long, requires = "section")]
    pub value: Option<String>,
    /// New lifecycle status.
    #[arg(long, value_enum)]
    pub status: Option<AdrStatusArg>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `adr delete`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AdrDeleteArgs {
    /// Repository path.
    pub path: PathBuf,
    /// ADR id, e.g. `ADR-0001`.
    pub id: String,
}

/// Arguments for `adr list`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AdrListArgs {
    /// Repository path.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// CLI-facing mirror of [`crate::adr::AdrStatus`] (clap's `ValueEnum` derive
/// needs a local type in most configurations here).
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AdrStatusArg {
    /// Drafted, not yet decided.
    Proposed,
    /// Actively in effect.
    Accepted,
    /// No longer recommended, but not replaced by a specific other ADR.
    Deprecated,
    /// Replaced by a later decision.
    Superseded,
}

impl From<AdrStatusArg> for crate::adr::AdrStatus {
    fn from(value: AdrStatusArg) -> Self {
        match value {
            AdrStatusArg::Proposed => Self::Proposed,
            AdrStatusArg::Accepted => Self::Accepted,
            AdrStatusArg::Deprecated => Self::Deprecated,
            AdrStatusArg::Superseded => Self::Superseded,
        }
    }
}

/// Graph artifact command namespace.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct GraphCommand {
    /// Graph artifact operation.
    #[command(subcommand)]
    pub target: GraphTarget,
}

/// Graph artifact operations.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub(crate) enum GraphTarget {
    /// Export the current graph snapshot as a compressed artifact.
    Export(GraphExportArgs),
    /// Import a compressed graph artifact into this repository's graph store.
    Import(GraphImportArgs),
    /// Render the deterministic report for the current graph snapshot.
    Report(GraphReportArgs),
}

/// Arguments for `graph report`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct GraphReportArgs {
    /// Repository path with a generated Lithograph graph store.
    #[arg(default_value = ".")]
    pub path: PathBuf,
    /// Exclude `Unresolved` reference nodes (and their edges) from the report,
    /// including the unresolved-gaps section. Off by default.
    #[arg(long)]
    pub hide_unresolved: bool,
}

/// Arguments for `path`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct PathArgs {
    /// Repository path with a generated Lithograph graph store.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Node id, name, or substring to start from.
    pub from: String,
    /// Node id, name, or substring to reach.
    pub to: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `explain`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ExplainArgs {
    /// Repository path with a generated Lithograph graph store.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Node id, name, or substring to explain.
    pub node: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `affected`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AffectedArgs {
    /// Repository path with a generated Lithograph graph store.
    #[arg(long, default_value = ".")]
    pub path: PathBuf,
    /// Node ids, names, or changed file paths to analyze.
    pub targets: Vec<String>,
    /// Read targets from stdin, one per line, so a changed-file list can be
    /// piped straight in: `git diff --name-only | lithograph affected --stdin`.
    #[arg(long)]
    pub stdin: bool,
    /// How many relation hops of dependents to follow.
    #[arg(long, default_value_t = 2)]
    pub depth: usize,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `graph export`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct GraphExportArgs {
    /// Repository path with a generated Lithograph graph store.
    pub path: PathBuf,
    /// Output compressed artifact path.
    #[arg(long)]
    pub output: PathBuf,
}

/// Arguments for `graph import`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct GraphImportArgs {
    /// Repository path whose graph store should receive the artifact.
    pub path: PathBuf,
    /// Compressed artifact path to import.
    pub artifact: PathBuf,
}

/// Arguments for `golden`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct GoldenArgs {
    /// Repository path with generated Lithograph output.
    pub path: PathBuf,
    /// Snapshot directory.
    #[arg(long, default_value = "tests/golden/polyglot")]
    pub golden_dir: PathBuf,
    /// Update snapshots instead of checking them.
    #[arg(long)]
    pub update: bool,
}

/// Arguments for `quality`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct QualityArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `validate-mermaid`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ValidateMermaidArgs {
    /// Repository path or Markdown file to validate.
    pub path: PathBuf,
    /// Optional local Node validator script. It receives Mermaid text on stdin.
    #[arg(long)]
    pub node_validator: Option<PathBuf>,
    /// Rewrite unsafe node ids to deterministic ASCII ids in place, then
    /// re-validate. Never run unless explicitly requested (LIT-22.7.2 AC3).
    #[arg(long)]
    pub fix: bool,
}

/// Arguments for `mcp-server`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct McpServerArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
}

/// Arguments for `viewer`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ViewerArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Output directory for the static viewer.
    #[arg(long, default_value = ".lithograph/viewer")]
    pub output_dir: PathBuf,
}

/// Arguments for `serve`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct ServeArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Additional explicitly allowlisted repositories, as `ID=PATH`.
    /// Repeat for multiple projects. IDs are stable URL-safe names; no
    /// parent directory is scanned for repositories.
    #[arg(long = "project", value_name = "ID=PATH")]
    pub projects: Vec<String>,
    /// Directory of static UI assets to serve, e.g. a built graph explorer
    /// bundle. Relative paths are resolved against `path`. Missing
    /// directories are tolerated: the graph API still serves, but static
    /// routes 404.
    #[arg(long, default_value = ".lithograph/viewer")]
    pub assets: PathBuf,
    /// Local TCP port to bind. `0` picks an OS-assigned ephemeral port.
    #[arg(long, default_value_t = 4317)]
    pub port: u16,
}

/// Arguments for `drift`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct DriftArgs {
    /// Repository path to scan.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `ask`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct AskArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Question to answer from generated docs.
    pub question: String,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `mcp-export`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct McpExportArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Optional question to answer in the export payload.
    #[arg(long)]
    pub question: Option<String>,
}

/// Arguments for `init` and `update`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InitArgs {
    /// Repository path to compile documentation for.
    pub path: PathBuf,
    /// Prompt template version stamped on generated pages.
    #[arg(long, default_value = "v1")]
    pub prompt_version: String,
    /// Use deterministic semantic grouping when planning documentation modules.
    #[arg(long)]
    pub semantic_grouping: bool,
    /// Include conventional test files and directories in the scan.
    #[arg(long)]
    pub include_tests: bool,
}

/// Arguments for `integrate-agents`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct IntegrateAgentsArgs {
    /// Repository path whose top-level `AGENTS.md`/`CLAUDE.md` should be updated.
    pub path: PathBuf,
}

/// Inspect command namespace.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectCommand {
    /// Inspect target.
    #[command(subcommand)]
    pub target: InspectTarget,
}

/// Inspectable repository data.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub(crate) enum InspectTarget {
    /// Print artifact inventory.
    Artifacts(InspectArtifactsArgs),
    /// Print the semantic graph.
    Graph(InspectGraphArgs),
    /// Explain environment-variable reads, definitions, and config links.
    Env(InspectEnvArgs),
    /// Print the deterministic module plan.
    Modules(InspectModulesArgs),
    /// Print the module dependency matrix and cycles.
    Dsm(InspectDsmArgs),
    /// Print the last recorded run's metrics: index/generation time, graph
    /// size, cache hit rate, and token estimate. Optionally checks them
    /// against explicit budget thresholds.
    Metrics(InspectMetricsArgs),
}

/// Arguments for `inspect modules`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectModulesArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Use deterministic semantic grouping when planning modules.
    #[arg(long)]
    pub semantic_grouping: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect dsm`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectDsmArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect artifacts`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectArtifactsArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect graph`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectGraphArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
    /// Exclude `Unresolved` reference nodes (and their edges) from the output.
    /// Off by default.
    #[arg(long)]
    pub hide_unresolved: bool,
}

/// Arguments for `inspect env`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectEnvArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Optional environment variable name or stable node id to inspect.
    #[arg(long)]
    pub variable: Option<String>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect metrics`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub(crate) struct InspectMetricsArgs {
    /// Repository path whose last recorded `.lithograph/run.json` to inspect.
    pub path: PathBuf,
    /// Fail with the exceeded thresholds listed when the graph has more
    /// than this many nodes.
    #[arg(long)]
    pub max_graph_nodes: Option<usize>,
    /// Fail when the graph has more than this many relations.
    #[arg(long)]
    pub max_graph_relations: Option<usize>,
    /// Fail when the analysis cache hit rate drops below this percentage
    /// (`0`-`100`). An integer percentage, not a float, so this argument
    /// stays `Eq`-comparable like every other CLI argument struct.
    #[arg(long)]
    pub min_cache_hit_rate_percent: Option<u8>,
    /// Fail when the estimated prompt token count exceeds this value.
    #[arg(long)]
    pub max_tokens: Option<u64>,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum OutputFormat {
    /// Human-readable table.
    Table,
    /// Deterministic JSON.
    Json,
}

impl Cli {
    /// Parses command-line arguments from the current process.
    pub(crate) fn parse_args() -> Self {
        Self::parse()
    }

    /// Parses command-line arguments from an explicit iterator.
    ///
    /// Tests use this path to verify the CLI definition without spawning a
    /// process. User-facing process behavior is covered by integration tests.
    pub(crate) fn parse_from_args<I, T>(args: I) -> Self
    where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone,
    {
        Self::parse_from(args)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AdrCommand, AdrCreateArgs, AdrListArgs, AdrStatusArg, AdrTarget, AdrUpdateArgs, AskArgs,
        Cli, Command, DriftArgs, GoldenArgs, GraphCommand, GraphExportArgs, GraphImportArgs,
        GraphReportArgs, GraphTarget, InitArgs, InspectArtifactsArgs, InspectCommand,
        InspectDsmArgs, InspectEnvArgs, InspectGraphArgs, InspectModulesArgs, InspectTarget,
        IntegrateAgentsArgs, McpExportArgs, McpServerArgs, OutputFormat, QualityArgs,
        ResearchCommand, ResearchOutcomeArg, ResearchReflectArgs, ResearchSaveResultArgs,
        ResearchTarget, ServeArgs, ValidateMermaidArgs, ViewerArgs,
    };
    use std::path::PathBuf;

    #[test]
    fn parses_binary_name_without_subcommands() {
        let cli = Cli::parse_from_args(["lithograph"]);

        assert_eq!(cli.command, None);
    }

    #[test]
    fn parses_inspect_artifacts_defaults_to_table() {
        let cli = Cli::parse_from_args(["lithograph", "inspect", "artifacts", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Artifacts(InspectArtifactsArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    format: OutputFormat::Table,
                }),
            }))
        );
    }

    #[test]
    fn parses_inspect_artifacts_json_format() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "inspect",
            "artifacts",
            "fixtures/polyglot",
            "--format",
            "json",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Artifacts(InspectArtifactsArgs {
                    format: OutputFormat::Json,
                    ..
                }),
            }))
        ));
    }

    #[test]
    fn parses_inspect_graph_defaults_to_table() {
        let cli = Cli::parse_from_args(["lithograph", "inspect", "graph", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Graph(InspectGraphArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    format: OutputFormat::Table,
                    hide_unresolved: false,
                }),
            }))
        );
    }

    #[test]
    fn parses_inspect_graph_hide_unresolved_flag() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "inspect",
            "graph",
            "fixtures/polyglot",
            "--hide-unresolved",
        ]);
        assert_eq!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Graph(InspectGraphArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    format: OutputFormat::Table,
                    hide_unresolved: true,
                }),
            }))
        );
    }

    #[test]
    fn parses_inspect_graph_json_format() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "inspect",
            "graph",
            "fixtures/polyglot",
            "--format",
            "json",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Graph(InspectGraphArgs {
                    format: OutputFormat::Json,
                    ..
                }),
            }))
        ));
    }

    #[test]
    fn parses_inspect_env_filter_and_json_format() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "inspect",
            "env",
            "fixtures/polyglot",
            "--variable",
            "DATABASE_URL",
            "--format",
            "json",
        ]);

        assert_eq!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Env(InspectEnvArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    variable: Some("DATABASE_URL".to_owned()),
                    format: OutputFormat::Json,
                }),
            }))
        );
    }

    #[test]
    fn parses_inspect_dsm_json_format() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "inspect",
            "dsm",
            "fixtures/polyglot",
            "--format",
            "json",
        ]);
        assert!(matches!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Dsm(InspectDsmArgs {
                    format: OutputFormat::Json,
                    ..
                })
            }))
        ));
    }

    #[test]
    fn parses_init_defaults_prompt_version() {
        let cli = Cli::parse_from_args(["lithograph", "init", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Init(InitArgs {
                path: PathBuf::from("fixtures/polyglot"),
                prompt_version: "v1".to_owned(),
                semantic_grouping: false,
                include_tests: false,
            }))
        );
    }

    #[test]
    fn parses_init_prompt_version_override() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "init",
            "fixtures/polyglot",
            "--prompt-version",
            "v2",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Init(InitArgs { prompt_version, .. })) if prompt_version == "v2"
        ));
    }

    #[test]
    fn parses_update_defaults_prompt_version() {
        let cli = Cli::parse_from_args(["lithograph", "update", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Update(InitArgs {
                path: PathBuf::from("fixtures/polyglot"),
                prompt_version: "v1".to_owned(),
                semantic_grouping: false,
                include_tests: false,
            }))
        );
    }

    #[test]
    fn parses_init_semantic_grouping_flag() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "init",
            "fixtures/polyglot",
            "--semantic-grouping",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Init(InitArgs {
                semantic_grouping: true,
                ..
            }))
        ));
    }

    #[test]
    fn parses_init_include_tests_flag() {
        let cli =
            Cli::parse_from_args(["lithograph", "init", "fixtures/polyglot", "--include-tests"]);

        assert!(matches!(
            cli.command,
            Some(Command::Init(InitArgs {
                include_tests: true,
                ..
            }))
        ));
    }

    #[test]
    fn parses_inspect_modules_defaults_to_table() {
        let cli = Cli::parse_from_args(["lithograph", "inspect", "modules", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Modules(InspectModulesArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    semantic_grouping: false,
                    format: OutputFormat::Table,
                }),
            }))
        );
    }

    #[test]
    fn parses_inspect_modules_json_format() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "inspect",
            "modules",
            "fixtures/polyglot",
            "--format",
            "json",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Inspect(InspectCommand {
                target: InspectTarget::Modules(InspectModulesArgs {
                    format: OutputFormat::Json,
                    ..
                }),
            }))
        ));
    }

    #[test]
    fn parses_ask_and_mcp_export() {
        let ask = Cli::parse_from_args(["lithograph", "ask", "fixtures/polyglot", "architecture"]);
        assert_eq!(
            ask.command,
            Some(Command::Ask(AskArgs {
                path: PathBuf::from("fixtures/polyglot"),
                question: "architecture".to_owned(),
                format: OutputFormat::Table,
            }))
        );

        let export = Cli::parse_from_args([
            "lithograph",
            "mcp-export",
            "fixtures/polyglot",
            "--question",
            "modules",
        ]);
        assert_eq!(
            export.command,
            Some(Command::McpExport(McpExportArgs {
                path: PathBuf::from("fixtures/polyglot"),
                question: Some("modules".to_owned()),
            }))
        );
    }

    #[test]
    fn parses_graph_artifact_commands() {
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "graph",
                "export",
                "fixtures/polyglot",
                "--output",
                "graph.lithograph-graph.gz",
            ])
            .command,
            Some(Command::Graph(GraphCommand {
                target: GraphTarget::Export(GraphExportArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    output: PathBuf::from("graph.lithograph-graph.gz"),
                }),
            }))
        );

        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "graph",
                "import",
                "fixtures/polyglot",
                "graph.lithograph-graph.gz",
            ])
            .command,
            Some(Command::Graph(GraphCommand {
                target: GraphTarget::Import(GraphImportArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    artifact: PathBuf::from("graph.lithograph-graph.gz"),
                }),
            }))
        );
        assert_eq!(
            Cli::parse_from_args(["lithograph", "graph", "report", "fixtures/polyglot"]).command,
            Some(Command::Graph(GraphCommand {
                target: GraphTarget::Report(GraphReportArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    hide_unresolved: false,
                }),
            }))
        );
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "serve",
                "fixtures/polyglot",
                "--project",
                "api=/repos/api",
                "--project",
                "web=/repos/web",
            ])
            .command,
            Some(Command::Serve(ServeArgs {
                path: PathBuf::from("fixtures/polyglot"),
                projects: vec!["api=/repos/api".to_owned(), "web=/repos/web".to_owned()],
                assets: PathBuf::from(".lithograph/viewer"),
                port: 4317,
            }))
        );
    }

    #[test]
    fn parses_adr_commands() {
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "adr",
                "create",
                "fixtures/polyglot",
                "--title",
                "Use Postgres",
                "--context",
                "We need a database.",
                "--decision",
                "Use Postgres.",
            ])
            .command,
            Some(Command::Adr(AdrCommand {
                target: AdrTarget::Create(AdrCreateArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    title: "Use Postgres".to_owned(),
                    context: "We need a database.".to_owned(),
                    decision: "Use Postgres.".to_owned(),
                    consequences: None,
                    format: OutputFormat::Table,
                }),
            }))
        );

        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "adr",
                "update",
                "fixtures/polyglot",
                "ADR-0001",
                "--section",
                "consequences",
                "--value",
                "Adds an ops dependency.",
                "--status",
                "accepted",
            ])
            .command,
            Some(Command::Adr(AdrCommand {
                target: AdrTarget::Update(AdrUpdateArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    id: "ADR-0001".to_owned(),
                    section: Some("consequences".to_owned()),
                    value: Some("Adds an ops dependency.".to_owned()),
                    status: Some(AdrStatusArg::Accepted),
                    format: OutputFormat::Table,
                }),
            }))
        );

        assert_eq!(
            Cli::parse_from_args(["lithograph", "adr", "list", "fixtures/polyglot"]).command,
            Some(Command::Adr(AdrCommand {
                target: AdrTarget::List(AdrListArgs {
                    path: PathBuf::from("fixtures/polyglot"),
                    format: OutputFormat::Table,
                }),
            }))
        );
    }

    #[test]
    fn parses_lit_15_stabilization_commands() {
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "golden",
                "fixtures/polyglot",
                "--golden-dir",
                "tests/golden/polyglot",
                "--update",
            ])
            .command,
            Some(Command::Golden(GoldenArgs {
                path: PathBuf::from("fixtures/polyglot"),
                golden_dir: PathBuf::from("tests/golden/polyglot"),
                update: true,
            }))
        );
        assert_eq!(
            Cli::parse_from_args(["lithograph", "quality", "fixtures/polyglot"]).command,
            Some(Command::Quality(QualityArgs {
                path: PathBuf::from("fixtures/polyglot"),
                format: OutputFormat::Table,
            }))
        );
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "validate-mermaid",
                "docs",
                "--node-validator",
                "scripts/validate-mermaid.mjs",
            ])
            .command,
            Some(Command::ValidateMermaid(ValidateMermaidArgs {
                path: PathBuf::from("docs"),
                node_validator: Some(PathBuf::from("scripts/validate-mermaid.mjs")),
                fix: false,
            }))
        );
        assert_eq!(
            Cli::parse_from_args(["lithograph", "mcp-server", "fixtures/polyglot"]).command,
            Some(Command::McpServer(McpServerArgs {
                path: PathBuf::from("fixtures/polyglot"),
            }))
        );
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "viewer",
                "fixtures/polyglot",
                "--output-dir",
                "viewer",
            ])
            .command,
            Some(Command::Viewer(ViewerArgs {
                path: PathBuf::from("fixtures/polyglot"),
                output_dir: PathBuf::from("viewer"),
            }))
        );
        assert_eq!(
            Cli::parse_from_args(["lithograph", "serve", "fixtures/polyglot"]).command,
            Some(Command::Serve(ServeArgs {
                path: PathBuf::from("fixtures/polyglot"),
                projects: Vec::new(),
                assets: PathBuf::from(".lithograph/viewer"),
                port: 4317,
            }))
        );
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "serve",
                "fixtures/polyglot",
                "--assets",
                "ui/dist",
                "--port",
                "0",
            ])
            .command,
            Some(Command::Serve(ServeArgs {
                path: PathBuf::from("fixtures/polyglot"),
                projects: Vec::new(),
                assets: PathBuf::from("ui/dist"),
                port: 0,
            }))
        );
    }

    #[test]
    fn parses_drift_defaults_to_table() {
        let cli = Cli::parse_from_args(["lithograph", "drift", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Drift(DriftArgs {
                path: PathBuf::from("fixtures/polyglot"),
                format: OutputFormat::Table,
            }))
        );
    }

    #[test]
    fn parses_drift_json_format() {
        let cli = Cli::parse_from_args([
            "lithograph",
            "drift",
            "fixtures/polyglot",
            "--format",
            "json",
        ]);

        assert!(matches!(
            cli.command,
            Some(Command::Drift(DriftArgs {
                format: OutputFormat::Json,
                ..
            }))
        ));
    }

    #[test]
    fn parses_integrate_agents() {
        let cli = Cli::parse_from_args(["lithograph", "integrate-agents", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::IntegrateAgents(IntegrateAgentsArgs {
                path: PathBuf::from("fixtures/polyglot"),
            }))
        );
    }

    #[test]
    fn parses_research_feedback_commands() {
        assert_eq!(
            Cli::parse_from_args([
                "lithograph",
                "research",
                "save-result",
                ".",
                "--question",
                "where?",
                "--answer",
                "here",
                "--node",
                "b,a",
                "--outcome",
                "dead_end",
                "--recorded-at",
                "100",
            ])
            .command,
            Some(Command::Research(ResearchCommand {
                target: ResearchTarget::SaveResult(ResearchSaveResultArgs {
                    path: PathBuf::from("."),
                    question: "where?".to_owned(),
                    answer: "here".to_owned(),
                    cited_node_ids: vec!["b".to_owned(), "a".to_owned()],
                    outcome: ResearchOutcomeArg::DeadEnd,
                    correction: None,
                    recorded_at: Some(100),
                }),
            }))
        );
        assert_eq!(
            Cli::parse_from_args(["lithograph", "research", "reflect", ".", "--now", "200",])
                .command,
            Some(Command::Research(ResearchCommand {
                target: ResearchTarget::Reflect(ResearchReflectArgs {
                    path: PathBuf::from("."),
                    now: Some(200),
                }),
            }))
        );
    }
}
