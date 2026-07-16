//! `mcp-server`, `viewer`, and `serve`: expose generated Lithograph
//! knowledge over stdio MCP, a static viewer, and the local graph explorer.

use crate::cli::{McpServerArgs, ServeArgs, ViewerArgs};
use crate::mcp::WikiMcpServer;
use crate::viewer::{generate as generate_viewer, render_report as render_viewer_report};
use std::io::Write;

pub(crate) fn execute_mcp_server<W>(
    args: McpServerArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let stdin = std::io::stdin();
    WikiMcpServer::new(&args.path).run(stdin.lock(), writer)
}

pub(crate) fn execute_viewer<W>(
    args: ViewerArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
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

pub(crate) fn execute_serve<W>(
    args: ServeArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let projects = args
        .projects
        .iter()
        .map(|value| {
            let (id, path) = value
                .split_once('=')
                .ok_or_else(|| format!("invalid --project `{value}`; expected ID=PATH"))?;
            if path.is_empty() {
                return Err(format!(
                    "invalid --project `{value}`; PATH must not be empty"
                ));
            }
            Ok(crate::serve::NamedProjectRoot::new(
                id,
                std::path::PathBuf::from(path),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let assets = if args.assets.is_absolute() {
        args.assets
    } else {
        args.path.join(args.assets)
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(crate::serve::run_projects(
        &args.path, projects, &assets, args.port, writer,
    ))?;
    Ok(())
}
