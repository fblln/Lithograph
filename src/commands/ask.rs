//! `ask` and `mcp-export`: deterministic local question answering and
//! MCP-style JSON export over generated Lithograph docs.

use crate::agent::ask::{McpExport, WikiSearch, render_ask_table};
use crate::cli::{AskArgs, McpExportArgs, OutputFormat};
use std::io::Write;

pub(crate) fn execute_ask<W>(
    args: AskArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
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

pub(crate) fn execute_mcp_export<W>(
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
