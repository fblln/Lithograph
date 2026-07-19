use super::execute;
use super::generate::select_model;
use super::inspect::{render_artifacts_json, render_artifacts_table, render_graph_diagnostics};
use crate::cli::{
    AdrCommand, AdrCreateArgs, AdrDeleteArgs, AdrGetArgs, AdrListArgs, AdrStatusArg, AdrTarget,
    AdrUpdateArgs, AskArgs, Cli, Command, DriftArgs, GraphCommand, GraphExportArgs,
    GraphImportArgs, GraphReportArgs, GraphTarget, InitArgs, InspectArtifactsArgs, InspectCommand,
    InspectGraphArgs, InspectMetricsArgs, InspectModulesArgs, InspectTarget, IntegrateAgentsArgs,
    IntegrateMcpArgs, McpExportArgs, OutputFormat, ResearchCommand, ResearchOutcomeArg,
    ResearchReflectArgs, ResearchSaveResultArgs, ResearchTarget, ValidateMermaidArgs, WatchArgs,
};
use crate::graph::{GraphIssue, GraphIssueKind, GraphStore};
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
            include_tests: false,
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
            include_tests: false,
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
fn research_cli_saves_results_and_reflects_corroborated_lessons()
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
                include_tests: false,
            })),
        },
        &mut Vec::new(),
    )?;
    let node_id = GraphStore::new(temp.path()).load()?.graph.nodes[0]
        .id()
        .as_str()
        .to_owned();
    for question in ["where?", "which?"] {
        execute(
            Cli {
                command: Some(Command::Research(ResearchCommand {
                    target: ResearchTarget::SaveResult(ResearchSaveResultArgs {
                        path: temp.path().to_path_buf(),
                        question: question.to_owned(),
                        answer: "here".to_owned(),
                        cited_node_ids: vec![node_id.clone()],
                        outcome: ResearchOutcomeArg::Useful,
                        correction: None,
                        recorded_at: Some(100),
                    }),
                })),
            },
            &mut Vec::new(),
        )?;
    }
    let mut output = Vec::new();
    execute(
        Cli {
            command: Some(Command::Research(ResearchCommand {
                target: ResearchTarget::Reflect(ResearchReflectArgs {
                    path: temp.path().to_path_buf(),
                    now: Some(100),
                }),
            })),
        },
        &mut output,
    )?;
    let lessons: crate::research_feedback::ResearchLessons = serde_json::from_slice(&output)?;
    assert_eq!(lessons.preferred_sources[0].node_id, node_id);
    assert!(
        temp.path()
            .join(".lithograph/research/lessons.json")
            .exists()
    );
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
fn execute_watch_once_reports_up_to_date_after_init() -> Result<(), Box<dyn std::error::Error>> {
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
                include_tests: false,
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
fn execute_watch_without_auto_index_never_writes_docs() -> Result<(), Box<dyn std::error::Error>> {
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
fn execute_watch_with_auto_index_runs_update_when_stale() -> Result<(), Box<dyn std::error::Error>>
{
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
fn execute_graph_export_and_import_round_trips_artifact() -> Result<(), Box<dyn std::error::Error>>
{
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
                include_tests: false,
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
    let mut report_output = Vec::new();
    execute(
        Cli {
            command: Some(Command::Graph(GraphCommand {
                target: GraphTarget::Report(GraphReportArgs {
                    path: source.path().to_path_buf(),
                    hide_unresolved: false,
                }),
            })),
        },
        &mut report_output,
    )?;

    assert!(artifact_path.exists());
    assert!(export_output.contains("graph artifact exported"));
    assert!(import_output.contains("graph artifact imported"));
    assert!(
        destination
            .path()
            .join(".lithograph/graph/current.json")
            .exists()
    );
    assert_eq!(
        report_output,
        std::fs::read(source.path().join(".lithograph/GRAPH_REPORT.md"))?
    );
    assert!(String::from_utf8(report_output)?.contains("## Suggested audit questions"));

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
                include_tests: false,
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
fn execute_integrate_agents_creates_then_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
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
fn execute_integrate_mcp_without_target_only_detects() -> Result<(), Box<dyn std::error::Error>> {
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
fn execute_integrate_mcp_apply_writes_then_is_idempotent() -> Result<(), Box<dyn std::error::Error>>
{
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
    let artifacts = RepositoryWalker::new(WalkOptions {
        include_hidden_directories: true,
        ..WalkOptions::default()
    })
    .walk(&root)?;
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
fn execute_drift_reports_no_drift_on_the_clean_fixture() -> Result<(), Box<dyn std::error::Error>> {
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
                include_tests: false,
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
                    hide_unresolved: false,
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
fn execute_adr_create_get_update_list_delete_round_trips() -> Result<(), Box<dyn std::error::Error>>
{
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
                hide_unresolved: false,
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
                hide_unresolved: false,
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
fn execute_inspect_metrics_reports_persisted_run_metadata() -> Result<(), Box<dyn std::error::Error>>
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
                include_tests: false,
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
                include_tests: false,
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
            message: "relation:1 has target symbol:missing which is not a graph node".to_owned(),
        },
        GraphIssue {
            kind: GraphIssueKind::InvalidSourceSpan,
            message: "evidence for src/lib.rs spans lines 1-100 but the artifact has only 5 lines"
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
fn validate_mermaid_fix_flag_rewrites_ids_then_passes() -> Result<(), Box<dyn std::error::Error>> {
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
