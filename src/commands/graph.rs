//! `graph`: export or import team-shareable graph artifacts.

use crate::cli::{GraphCommand, GraphExportArgs, GraphImportArgs, GraphReportArgs, GraphTarget};
use crate::graph::{GraphArtifactReport, GraphReport, GraphStore};
use std::io::Write;

pub(crate) fn execute_graph<W>(
    command: GraphCommand,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    match command.target {
        GraphTarget::Export(args) => execute_graph_export(args, writer),
        GraphTarget::Import(args) => execute_graph_import(args, writer),
        GraphTarget::Report(args) => execute_graph_report(args, writer),
    }
}

fn execute_graph_report<W>(
    args: GraphReportArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let loaded = GraphStore::new(&args.path).load()?.graph;
    let graph = if args.hide_unresolved {
        loaded.without_unresolved()
    } else {
        loaded
    };
    writer.write_all(GraphReport::build(&graph).render_markdown().as_bytes())?;
    Ok(())
}

fn execute_graph_export<W>(
    args: GraphExportArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = GraphStore::new(&args.path).export_artifact(&args.output)?;
    writer.write_all(render_graph_artifact_report("exported", &report).as_bytes())?;
    Ok(())
}

fn execute_graph_import<W>(
    args: GraphImportArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let report = GraphStore::new(&args.path).import_artifact(&args.artifact)?;
    writer.write_all(render_graph_artifact_report("imported", &report).as_bytes())?;
    Ok(())
}

/// Renders a deterministic graph artifact operation summary.
fn render_graph_artifact_report(action: &str, report: &GraphArtifactReport) -> String {
    format!(
        "graph artifact {action}\n\
         artifact: {}\n\
         snapshot: {}\n\
         format: {}\n\
         compression: {}\n\
         checksum: {}\n\
         schema: {}\n\
         graph model: {}\n\
         nodes: {}\n\
         relations: {}\n",
        report.artifact_path.display(),
        report.snapshot_path.display(),
        report.metadata.artifact_format_version,
        report.metadata.compression,
        report.metadata.snapshot_checksum,
        report.metadata.schema_version,
        report.metadata.graph_model_version,
        report.metadata.node_count,
        report.metadata.relation_count,
    )
}
