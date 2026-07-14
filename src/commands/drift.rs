//! `drift`: scan Markdown documentation for likely drift against the
//! current repository and graph. Deterministic: never calls a language
//! model.

use crate::cli::{DriftArgs, OutputFormat};
use crate::drift::{DriftDetector, DriftReport};
use crate::graph::GraphBuilder;
use crate::inventory::{RepositoryWalker, WalkOptions};
use std::io::Write;

pub(crate) fn execute_drift<W>(
    args: DriftArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let walk_options = WalkOptions {
        exclude_globs: crate::orchestrate::cache_exclude_globs(),
        ..WalkOptions::default()
    };
    let artifacts = RepositoryWalker::new(walk_options).walk(&args.path)?;
    let graph = GraphBuilder.build(&args.path, &artifacts);
    let report = DriftDetector.scan(&artifacts, &graph, &args.path);

    let output = match args.format {
        OutputFormat::Table => render_drift_table(&report),
        OutputFormat::Json => {
            let mut json = serde_json::to_string_pretty(&report)?;
            json.push('\n');
            json
        }
    };
    writer.write_all(output.as_bytes())?;
    Ok(())
}

/// Renders a deterministic, human-readable drift report: one line per
/// finding (kind, artifact path, one-based line when evidence has a span,
/// and the stale detail text), or a clear "no drift" message when empty.
fn render_drift_table(report: &DriftReport) -> String {
    if report.findings.is_empty() {
        return "no drift detected\n".to_owned();
    }
    let mut output = format!("{} drift finding(s):\n", report.findings.len());
    for finding in &report.findings {
        let line = finding
            .evidence
            .span
            .as_ref()
            .map(|span| span.start_line.to_string())
            .unwrap_or_else(|| "-".to_owned());
        output.push_str(&format!(
            "  [{:?}] {}:{line} {}\n",
            finding.kind, finding.artifact_path, finding.detail
        ));
    }
    output
}
