//! `integrate-agents` and `integrate-mcp`: wire generated Lithograph
//! knowledge into external agent tooling (`AGENTS.md`/`CLAUDE.md`, and
//! per-agent MCP server configuration).

use crate::agents::{AgentFileOutcome, IntegrateAgentsReport, integrate_agents};
use crate::cli::{IntegrateAgentsArgs, IntegrateMcpArgs, OutputFormat};
use crate::mcp_targets::{
    AgentTarget, IntegrationOutcome, TargetDetection, apply, detect, preview,
};
use std::io::Write;

pub(crate) fn execute_integrate_agents<W>(
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
fn render_integrate_agents_report(report: &IntegrateAgentsReport) -> String {
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
pub(crate) fn execute_integrate_mcp<W>(
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
