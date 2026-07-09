//! Command-line argument definitions.

use clap::{Args, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

/// Repository knowledge compiler that builds evidence-backed documentation.
#[derive(Debug, Parser)]
#[command(name = "lithograph")]
#[command(version)]
#[command(about = "Compile repository knowledge into evidence-backed documentation.")]
#[command(long_about = None)]
pub struct Cli {
    /// Command to run.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Top-level Lithograph commands.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum Command {
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
    /// Export or import team-shareable graph artifacts.
    Graph(GraphCommand),
    /// Create, read, update, delete, and list architecture decision records.
    Adr(AdrCommand),
    /// Poll a repository for staleness against its last recorded snapshot.
    /// Disabled by default beyond this explicit command: reports staleness
    /// only, unless `--auto-index` is passed.
    Watch(WatchArgs),
    /// Detect, preview, or apply per-agent MCP server integration (Codex,
    /// Claude, Gemini, Zed). Without `--target`, only detects and reports;
    /// `--apply` requires `--target` and is the only way anything is written.
    IntegrateMcp(IntegrateMcpArgs),
}

/// Arguments for `integrate-mcp`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct IntegrateMcpArgs {
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
pub struct WatchArgs {
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
pub struct AdrCommand {
    /// ADR operation.
    #[command(subcommand)]
    pub target: AdrTarget,
}

/// ADR operations.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum AdrTarget {
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
pub struct AdrCreateArgs {
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
pub struct AdrGetArgs {
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
pub struct AdrUpdateArgs {
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
pub struct AdrDeleteArgs {
    /// Repository path.
    pub path: PathBuf,
    /// ADR id, e.g. `ADR-0001`.
    pub id: String,
}

/// Arguments for `adr list`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AdrListArgs {
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
pub struct GraphCommand {
    /// Graph artifact operation.
    #[command(subcommand)]
    pub target: GraphTarget,
}

/// Graph artifact operations.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum GraphTarget {
    /// Export the current graph snapshot as a compressed artifact.
    Export(GraphExportArgs),
    /// Import a compressed graph artifact into this repository's graph store.
    Import(GraphImportArgs),
}

/// Arguments for `graph export`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct GraphExportArgs {
    /// Repository path with a generated Lithograph graph store.
    pub path: PathBuf,
    /// Output compressed artifact path.
    #[arg(long)]
    pub output: PathBuf,
}

/// Arguments for `graph import`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct GraphImportArgs {
    /// Repository path whose graph store should receive the artifact.
    pub path: PathBuf,
    /// Compressed artifact path to import.
    pub artifact: PathBuf,
}

/// Arguments for `golden`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct GoldenArgs {
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
pub struct QualityArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `validate-mermaid`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ValidateMermaidArgs {
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
pub struct McpServerArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
}

/// Arguments for `viewer`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct ViewerArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Output directory for the static viewer.
    #[arg(long, default_value = ".lithograph/viewer")]
    pub output_dir: PathBuf,
}

/// Arguments for `drift`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct DriftArgs {
    /// Repository path to scan.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `ask`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct AskArgs {
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
pub struct McpExportArgs {
    /// Repository path with generated Lithograph docs.
    pub path: PathBuf,
    /// Optional question to answer in the export payload.
    #[arg(long)]
    pub question: Option<String>,
}

/// Arguments for `init` and `update`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InitArgs {
    /// Repository path to compile documentation for.
    pub path: PathBuf,
    /// Prompt template version stamped on generated pages.
    #[arg(long, default_value = "v1")]
    pub prompt_version: String,
    /// Use deterministic semantic grouping when planning documentation modules.
    #[arg(long)]
    pub semantic_grouping: bool,
}

/// Arguments for `integrate-agents`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct IntegrateAgentsArgs {
    /// Repository path whose top-level `AGENTS.md`/`CLAUDE.md` should be updated.
    pub path: PathBuf,
}

/// Inspect command namespace.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InspectCommand {
    /// Inspect target.
    #[command(subcommand)]
    pub target: InspectTarget,
}

/// Inspectable repository data.
#[derive(Debug, Clone, PartialEq, Eq, Subcommand)]
pub enum InspectTarget {
    /// Print artifact inventory.
    Artifacts(InspectArtifactsArgs),
    /// Print the semantic graph.
    Graph(InspectGraphArgs),
    /// Print the deterministic module plan.
    Modules(InspectModulesArgs),
    /// Print the last recorded run's metrics: index/generation time, graph
    /// size, cache hit rate, and token estimate. Optionally checks them
    /// against explicit budget thresholds.
    Metrics(InspectMetricsArgs),
}

/// Arguments for `inspect modules`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InspectModulesArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Use deterministic semantic grouping when planning modules.
    #[arg(long)]
    pub semantic_grouping: bool,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect artifacts`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InspectArtifactsArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect graph`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InspectGraphArgs {
    /// Repository path to inspect.
    pub path: PathBuf,
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub format: OutputFormat,
}

/// Arguments for `inspect metrics`.
#[derive(Debug, Clone, PartialEq, Eq, Args)]
pub struct InspectMetricsArgs {
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
pub enum OutputFormat {
    /// Human-readable table.
    Table,
    /// Deterministic JSON.
    Json,
}

impl Cli {
    /// Parses command-line arguments from the current process.
    pub fn parse_args() -> Self {
        Self::parse()
    }

    /// Parses command-line arguments from an explicit iterator.
    ///
    /// Tests use this path to verify the CLI definition without spawning a
    /// process. User-facing process behavior is covered by integration tests.
    pub fn parse_from_args<I, T>(args: I) -> Self
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
        GraphTarget, InitArgs, InspectArtifactsArgs, InspectCommand, InspectGraphArgs,
        InspectModulesArgs, InspectTarget, IntegrateAgentsArgs, McpExportArgs, McpServerArgs,
        OutputFormat, QualityArgs, ValidateMermaidArgs, ViewerArgs,
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
    fn parses_init_defaults_prompt_version() {
        let cli = Cli::parse_from_args(["lithograph", "init", "fixtures/polyglot"]);

        assert_eq!(
            cli.command,
            Some(Command::Init(InitArgs {
                path: PathBuf::from("fixtures/polyglot"),
                prompt_version: "v1".to_owned(),
                semantic_grouping: false,
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
}
