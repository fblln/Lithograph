//! `path`, `explain`, `affected`: graph queries at the terminal.
//!
//! These render engines the MCP server already exposes to agents
//! (`trace_path`, `get_node_detail`, `impact_analysis`). Reaching them
//! previously meant standing up an MCP client, so a human with a shell could
//! not ask the graph the questions it answers best (LIT-47). The queries
//! themselves live in [`KnowledgeIndex`]; this module only chooses words.

use crate::cli::{AffectedArgs, ExplainArgs, OutputFormat, PathArgs, SearchCodeArgs};
use crate::graph::{
    Graph, GraphNodeId, GraphStore, KnowledgeIndex, NodeExplanation, PathResult, TraceDirection,
    TraceParams, TraceResult,
};
use serde::Serialize;
use std::io::{BufRead, Write};
use std::path::Path;

pub(crate) fn execute_path<W>(
    args: PathArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let graph = load_graph(&args.path)?;
    let result = KnowledgeIndex::new(&graph)
        .shortest_path(&args.from, &args.to)
        .ok_or_else(|| no_path_error(&graph, &args.from, &args.to))?;
    match args.format {
        OutputFormat::Json => write_json(writer, &result)?,
        OutputFormat::Table => writer.write_all(render_path(&result).as_bytes())?,
    }
    Ok(())
}

pub(crate) fn execute_explain<W>(
    args: ExplainArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let graph = load_graph(&args.path)?;
    let explanation = KnowledgeIndex::new(&graph)
        .explain(&args.node)
        .ok_or_else(|| format!("no graph node matched `{}`", args.node))?;
    match args.format {
        OutputFormat::Json => write_json(writer, &explanation)?,
        OutputFormat::Table => writer.write_all(render_explanation(&explanation).as_bytes())?,
    }
    Ok(())
}

pub(crate) fn execute_affected<W>(
    args: AffectedArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    let targets = collect_targets(&args, &mut std::io::stdin().lock())?;
    if targets.is_empty() {
        return Err(
            "affected requires at least one target, or --stdin with a non-empty list".into(),
        );
    }
    let graph = load_graph(&args.path)?;
    let index = KnowledgeIndex::new(&graph);

    // Every target is reported, including ones the graph does not know, so a
    // piped changed-file list never silently analyzes fewer files than it was
    // handed -- "no dependents" and "never looked" must not read alike.
    let reports: Vec<AffectedReport> = targets
        .iter()
        .map(|target| {
            let trace = index.impact_analysis(&TraceParams {
                query: target.clone(),
                depth: args.depth,
                direction: TraceDirection::Inbound,
            });
            AffectedReport {
                target: target.clone(),
                matched: trace.is_some(),
                dependents: trace.map(dependents_of).unwrap_or_default(),
            }
        })
        .collect();

    match args.format {
        OutputFormat::Json => write_json(writer, &reports)?,
        OutputFormat::Table => writer.write_all(render_affected(&reports).as_bytes())?,
    }
    Ok(())
}

/// Runs enriched raw-code semantic search from the terminal (LIT-86.6). Uses
/// the deterministic offline mock embedding provider: no network call, no live
/// model, and it builds/refreshes its own cached index from the repository, so
/// it does not require a prior `init`.
pub(crate) fn execute_search_code<W>(
    args: SearchCodeArgs,
    writer: &mut W,
) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
{
    use crate::graph::GraphNodeId;
    use crate::retrieval::chunk_rank::{Expansion, RankFilters, TagExpr};
    use crate::retrieval::code_index::{CodeSearchRequest, provider_identity, refresh, search};
    use crate::retrieval::semantic_search::MockEmbeddingProvider;

    let provider = MockEmbeddingProvider;
    let identity = provider_identity(&provider);

    // An explicit `--refresh` rebuilds the cache up front; the search below then
    // reads that fresh cache.
    if args.refresh {
        refresh(&args.path, &provider, &identity)?;
    }

    let request = CodeSearchRequest {
        query: args.query,
        filters: RankFilters {
            path_glob: args.path_glob,
            language: args.language,
            module_id: args.module_id.map(GraphNodeId::new),
            package_id: args.package_id.map(GraphNodeId::new),
            service: args.service,
            node_id: args.node_id.map(GraphNodeId::new),
            layer: args.layer,
            tags: TagExpr {
                all: args.tags,
                any: Vec::new(),
            },
        },
        expansion: Expansion {
            max_hops: args.expand_hops,
            relations: Vec::new(),
            direction: None,
        },
        limit: args.limit,
        offset: args.offset,
        refresh: false,
    };
    let response = search(&args.path, &request, &provider, &identity)?;

    match args.format {
        OutputFormat::Json => write_json(writer, &response)?,
        OutputFormat::Table => {
            writer.write_all(render_code_search(&response, args.explain).as_bytes())?;
        }
    }
    Ok(())
}

/// Renders a code-search response as a terminal table, with a freshness banner
/// and an optional per-result explanation.
fn render_code_search(
    response: &crate::retrieval::code_index::CodeSearchResponse,
    explain: bool,
) -> String {
    let mut out = String::new();
    let freshness = if response.freshness.is_fresh {
        "index: fresh"
    } else {
        "index: STALE (run with --refresh to rebuild)"
    };
    out.push_str(&format!(
        "{freshness} | provider {} | schema v{} | scoring v{} | {} matched\n",
        response.provider_model,
        response.index_schema_version,
        response.scoring_version,
        response.total_matched,
    ));
    for warning in &response.diagnostics.warnings {
        out.push_str(&format!("! {warning}\n"));
    }
    if response.results.is_empty() {
        out.push_str("(no results)\n");
        return out;
    }
    for result in &response.results {
        let span = result
            .evidence
            .span
            .as_ref()
            .map(|span| format!(":{}-{}", span.start_line, span.end_line))
            .unwrap_or_default();
        out.push_str(&format!(
            "{:.4}  {}{}  [{}]\n",
            result.features.final_score,
            result.evidence.path.as_str(),
            span,
            result.chunk_id,
        ));
        if explain {
            for step in &result.explanation {
                out.push_str(&format!("        {step}\n"));
            }
        }
    }
    out
}

/// Loads the repository's graph, or explains how to create one.
///
/// A missing store is the likeliest first-run error for these commands, so it
/// names the fix rather than surfacing a bare file-not-found (LIT-47 AC4).
fn load_graph(path: &Path) -> Result<Graph, Box<dyn std::error::Error>> {
    if !path.join(".lithograph/graph").exists() {
        return Err(format!(
            "no graph store in {}: run `lithograph init {}` first",
            path.display(),
            path.display()
        )
        .into());
    }
    Ok(GraphStore::new(path).load()?.graph)
}

/// Distinguishes "that node does not exist" from "those nodes do not
/// connect": the fix differs, so the message must too.
fn no_path_error(graph: &Graph, from: &str, to: &str) -> String {
    let index = KnowledgeIndex::new(graph);
    match (index.find_root(from), index.find_root(to)) {
        (None, _) => format!("no graph node matched `{from}`"),
        (_, None) => format!("no graph node matched `{to}`"),
        _ => format!("no path connects `{from}` and `{to}`"),
    }
}

fn collect_targets(
    args: &AffectedArgs,
    stdin: &mut impl BufRead,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut targets = args.targets.clone();
    if args.stdin {
        for line in stdin.lines() {
            let line = line?;
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                targets.push(trimmed.to_owned());
            }
        }
    }
    targets.dedup();
    Ok(targets)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct AffectedReport {
    target: String,
    /// False when the graph knows nothing by this name -- reported, never
    /// silently dropped.
    matched: bool,
    dependents: Vec<Dependent>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
struct Dependent {
    id: GraphNodeId,
    name: String,
    hop: usize,
}

fn dependents_of(trace: TraceResult) -> Vec<Dependent> {
    let mut dependents: Vec<Dependent> = trace
        .visited
        .into_iter()
        .filter(|hop| hop.hop > 0)
        .map(|hop| Dependent {
            id: hop.node.id,
            name: hop.node.name,
            hop: hop.hop,
        })
        .collect();
    dependents.sort_by(|a, b| a.hop.cmp(&b.hop).then(a.id.cmp(&b.id)));
    dependents
}

fn write_json<W, T>(writer: &mut W, value: &T) -> Result<(), Box<dyn std::error::Error>>
where
    W: Write,
    T: Serialize,
{
    writeln!(writer, "{}", serde_json::to_string_pretty(value)?)?;
    Ok(())
}

fn render_path(result: &PathResult) -> String {
    let mut output = format!("{}\n", result.start.name);
    if result.hops.is_empty() {
        output.push_str("  (both ends resolve to the same node)\n");
        return output;
    }
    for hop in &result.hops {
        // The arrow shows which way the underlying relation points, since a
        // path may legitimately traverse one backwards.
        let arrow = if hop.forward { "-->" } else { "<--" };
        let resolution = hop
            .resolution
            .map_or_else(String::new, |resolution| format!(" {resolution:?}"));
        output.push_str(&format!(
            "  {arrow} {} [{:?}{resolution}]\n",
            hop.node.name, hop.kind
        ));
    }
    output.push_str(&format!("\n{} hop(s)\n", result.hops.len()));
    output
}

fn render_explanation(explanation: &NodeExplanation) -> String {
    let mut output = format!("{}\n", explanation.node.name);
    output.push_str(&format!("  id:     {}\n", explanation.node.id.as_str()));
    output.push_str(&format!("  kind:   {}\n", explanation.node.label));
    output.push_str(&format!(
        "  degree: {} in / {} out\n",
        explanation.node.in_degree, explanation.node.out_degree
    ));
    if explanation.evidence.is_empty() {
        output.push_str("  source: (none recorded)\n");
    } else {
        for evidence in &explanation.evidence {
            let span = evidence
                .span
                .as_ref()
                .map_or_else(String::new, |span| format!(":{span}"));
            output.push_str(&format!("  source: {}{span}\n", evidence.path));
        }
    }
    for (heading, groups) in [
        ("outbound", &explanation.outbound),
        ("inbound", &explanation.inbound),
    ] {
        if groups.is_empty() {
            continue;
        }
        output.push_str(&format!("\n{heading}:\n"));
        for (kind, neighbors) in groups {
            output.push_str(&format!("  {kind} ({})\n", neighbors.len()));
            for neighbor in neighbors {
                let resolution = neighbor
                    .resolution
                    .map_or_else(String::new, |resolution| format!(" [{resolution:?}]"));
                output.push_str(&format!("    {}{resolution}\n", neighbor.node.name));
            }
        }
    }
    output
}

fn render_affected(reports: &[AffectedReport]) -> String {
    let mut output = String::new();
    for report in reports {
        if !report.matched {
            output.push_str(&format!("{}: no graph node matched\n", report.target));
            continue;
        }
        output.push_str(&format!(
            "{}: {} dependent(s)\n",
            report.target,
            report.dependents.len()
        ));
        for dependent in &report.dependents {
            output.push_str(&format!("  {} (hop {})\n", dependent.name, dependent.hop));
        }
    }
    output
}
