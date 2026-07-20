//! Command execution and output rendering.
//!
//! Each CLI command group has its own submodule; this module only wires a
//! parsed [`Command`] to its implementation, so it stays a thin dispatcher.

mod adr;
mod ask;
mod drift;
mod generate;
mod graph;
mod inspect;
mod integrate;
mod quality;
mod query;
mod research;
mod serve;

#[cfg(test)]
mod tests;

use crate::cli::{Cli, Command};
use std::io::Write;

/// Runs parsed CLI arguments and writes command output.
pub(crate) fn execute<W>(cli: Cli, writer: &mut W) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match cli.command {
        Some(Command::Init(args)) => generate::execute_init(args, writer),
        Some(Command::Update(args)) => generate::execute_update(args, writer),
        Some(Command::Inspect(command)) => inspect::execute_inspect(command, writer),
        Some(Command::IntegrateAgents(args)) => integrate::execute_integrate_agents(args, writer),
        Some(Command::Drift(args)) => drift::execute_drift(args, writer),
        Some(Command::Ask(args)) => ask::execute_ask(args, writer),
        Some(Command::McpExport(args)) => ask::execute_mcp_export(args, writer),
        Some(Command::Golden(args)) => quality::execute_golden(args, writer),
        Some(Command::Quality(args)) => quality::execute_quality(args, writer),
        Some(Command::ValidateMermaid(args)) => quality::execute_validate_mermaid(args, writer),
        Some(Command::McpServer(args)) => serve::execute_mcp_server(args, writer),
        Some(Command::Viewer(args)) => serve::execute_viewer(args, writer),
        Some(Command::Serve(args)) => serve::execute_serve(args, writer),
        Some(Command::Graph(args)) => graph::execute_graph(args, writer),
        Some(Command::Path(args)) => query::execute_path(args, writer),
        Some(Command::Explain(args)) => query::execute_explain(args, writer),
        Some(Command::Affected(args)) => query::execute_affected(args, writer),
        Some(Command::SearchCode(args)) => query::execute_search_code(args, writer),
        Some(Command::Adr(command)) => adr::execute_adr(command, writer),
        Some(Command::Research(command)) => research::execute_research(command, writer),
        Some(Command::Watch(args)) => generate::execute_watch(args, writer),
        Some(Command::IntegrateMcp(args)) => integrate::execute_integrate_mcp(args, writer),
        None => Ok(()),
    }
}
