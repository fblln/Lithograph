//! Static generated-wiki viewer output.

use crate::architecture::LayerKind;
use crate::ask::{WikiPage, WikiSearch};
use crate::graph::{ArchitectureSummary, GraphSchema, GraphStore, KnowledgeIndex};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Viewer generation report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerReport {
    /// Output directory.
    pub output_dir: PathBuf,
    /// Main HTML file.
    pub index_path: PathBuf,
    /// Number of generated pages indexed.
    pub page_count: usize,
}

/// Wiki page id for the deterministic graph summary pane (LIT-22.7.4 AC2).
pub const GRAPH_SUMMARY_PAGE_ID: &str = "viewer-graph-summary";
/// Wiki page id for the deterministic architecture summary pane (LIT-22.7.4 AC2).
pub const ARCHITECTURE_SUMMARY_PAGE_ID: &str = "viewer-architecture-summary";

/// Generates a lightweight static viewer with navigation, search, and
/// browser-side Mermaid rendering when Mermaid is available from a local or
/// configured browser runtime.
///
/// Two synthetic pages are appended to the generated-doc pages loaded from
/// the manifest: a graph summary and an architecture summary, both computed
/// directly from the persisted `.lithograph/graph` snapshot rather than from
/// LLM-authored prose, so they stay accurate even when the wiki pages
/// covering the same ground are stale relative to the graph (AC2). They
/// participate in the same nav list, in-browser search, and Markdown
/// rendering as every other page (AC1, AC3): no new UI machinery.
pub fn generate(
    repo_root: &Path,
    output_dir: &Path,
) -> Result<ViewerReport, Box<dyn std::error::Error>> {
    let mut pages = WikiSearch.load_pages(repo_root)?;
    pages.extend(graph_summary_pages(repo_root));
    std::fs::create_dir_all(output_dir)?;
    let data = serde_json::to_string(&json!({ "pages": pages }))?;
    let html = viewer_html(&data);
    let index_path = output_dir.join("index.html");
    std::fs::write(&index_path, html)?;
    Ok(ViewerReport {
        output_dir: output_dir.to_path_buf(),
        index_path,
        page_count: pages.len(),
    })
}

/// Builds the graph and architecture summary pages from the persisted graph
/// snapshot. Falls back to an empty graph -- rendered as explicit zero
/// counts, never fabricated content -- when no snapshot has been written yet.
fn graph_summary_pages(repo_root: &Path) -> [WikiPage; 2] {
    let graph = GraphStore::new(repo_root)
        .load()
        .map(|snapshot| snapshot.graph)
        .unwrap_or_default();
    let index = KnowledgeIndex::new(&graph);
    [
        WikiPage {
            id: GRAPH_SUMMARY_PAGE_ID.to_owned(),
            path: "(generated) graph summary".to_owned(),
            title: "Graph Summary".to_owned(),
            body: graph_summary_markdown(&index.schema()),
        },
        WikiPage {
            id: ARCHITECTURE_SUMMARY_PAGE_ID.to_owned(),
            path: "(generated) architecture summary".to_owned(),
            title: "Architecture Summary".to_owned(),
            body: architecture_summary_markdown(&index.architecture(None)),
        },
    ]
}

/// Renders deterministic Markdown for the graph summary pane: node and
/// relation counts by kind, sourced entirely from [`GraphSchema`].
fn graph_summary_markdown(schema: &GraphSchema) -> String {
    let mut body = String::from("# Graph Summary\n\n## Node counts\n\n");
    if schema.node_labels.is_empty() {
        body.push_str("No graph nodes found.\n");
    }
    for entry in &schema.node_labels {
        body.push_str(&format!("- {}: {}\n", entry.label, entry.count));
    }
    body.push_str("\n## Relation counts\n\n");
    if schema.edge_types.is_empty() {
        body.push_str("No graph relations found.\n");
    }
    for entry in &schema.edge_types {
        body.push_str(&format!("- {}: {}\n", entry.edge_type, entry.count));
    }
    body
}

/// Renders deterministic Markdown for the architecture summary pane: layer
/// distribution, cluster counts, and top entry points/hotspots, sourced
/// entirely from [`ArchitectureSummary`] (LIT-22.5.1/LIT-22.5.2 output).
fn architecture_summary_markdown(architecture: &ArchitectureSummary) -> String {
    let mut layer_counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for layer in &architecture.layers {
        *layer_counts
            .entry(layer_kind_label(layer.layer))
            .or_default() += 1;
    }

    let mut body = String::from("# Architecture Summary\n\n## Layers\n\n");
    if layer_counts.is_empty() {
        body.push_str("No architecture layer classification available.\n");
    }
    for (label, count) in &layer_counts {
        body.push_str(&format!("- {label}: {count}\n"));
    }

    body.push_str("\n## Clusters\n\n");
    if architecture.clusters.is_empty() {
        body.push_str("No functional clusters detected.\n");
    }
    for cluster in &architecture.clusters {
        body.push_str(&format!(
            "- {} ({} members, cohesion {:.2})\n",
            cluster.id,
            cluster.members.len(),
            cluster.cohesion
        ));
    }

    body.push_str("\n## Entry points\n\n");
    if architecture.entry_points.is_empty() {
        body.push_str("No entry points detected.\n");
    }
    for entry in &architecture.entry_points {
        body.push_str(&format!("- {} ({})\n", entry.name, entry.label));
    }

    body.push_str("\n## Hotspots\n\n");
    if architecture.hotspots.is_empty() {
        body.push_str("No high-degree nodes detected.\n");
    }
    for hotspot in &architecture.hotspots {
        body.push_str(&format!(
            "- {} ({}, in {} / out {})\n",
            hotspot.name, hotspot.label, hotspot.in_degree, hotspot.out_degree
        ));
    }

    body
}

fn layer_kind_label(kind: LayerKind) -> &'static str {
    match kind {
        LayerKind::Ui => "UI",
        LayerKind::Api => "API",
        LayerKind::Domain => "Domain",
        LayerKind::Data => "Data",
        LayerKind::Infra => "Infra",
        LayerKind::Test => "Test",
        LayerKind::Unknown => "Unknown",
    }
}

fn viewer_html(data: &str) -> String {
    format!(
        r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Lithograph Wiki</title>
  <style>
    :root {{ color-scheme: light dark; font-family: Inter, ui-sans-serif, system-ui, sans-serif; }}
    body {{ margin: 0; display: grid; grid-template-columns: minmax(220px, 320px) 1fr; min-height: 100vh; }}
    nav {{ border-right: 1px solid #8884; padding: 16px; overflow: auto; }}
    main {{ padding: 24px; max-width: 980px; }}
    input {{ width: 100%; box-sizing: border-box; padding: 8px; margin-bottom: 12px; }}
    button {{ display: block; width: 100%; text-align: left; padding: 7px 8px; border: 0; background: transparent; cursor: pointer; }}
    button[aria-current="page"] {{ background: #0f766e22; }}
    pre {{ overflow: auto; padding: 12px; background: #8882; }}
    code {{ font-family: ui-monospace, SFMono-Regular, Menlo, monospace; }}
    @media (max-width: 760px) {{ body {{ grid-template-columns: 1fr; }} nav {{ border-right: 0; border-bottom: 1px solid #8884; }} }}
  </style>
</head>
<body>
  <nav>
    <input id="search" type="search" aria-label="Search pages" placeholder="Search">
    <div id="pages"></div>
  </nav>
  <main id="content"></main>
  <script type="application/json" id="wiki-data">{data}</script>
  <script type="module">
    const data = JSON.parse(document.getElementById('wiki-data').textContent);
    const list = document.getElementById('pages');
    const content = document.getElementById('content');
    const search = document.getElementById('search');
    let active = data.pages[0]?.id;

    function escapeHtml(value) {{
      return value.replace(/[&<>"']/g, ch => ({{'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}}[ch]));
    }}
    function renderMarkdown(markdown) {{
      return escapeHtml(markdown)
        .replace(/^# (.*)$/gm, '<h1>$1</h1>')
        .replace(/^## (.*)$/gm, '<h2>$1</h2>')
        .replace(/```mermaid\n([\s\S]*?)```/g, '<pre class="mermaid">$1</pre>')
        .replace(/```([\s\S]*?)```/g, '<pre><code>$1</code></pre>')
        .replace(/\n/g, '<br>');
    }}
    async function renderContent() {{
      const page = data.pages.find(page => page.id === active) || data.pages[0];
      if (!page) {{ content.textContent = ''; return; }}
      content.innerHTML = renderMarkdown(page.body);
      if (globalThis.mermaid) {{
        globalThis.mermaid.initialize({{ startOnLoad: false }});
        await globalThis.mermaid.run({{ nodes: content.querySelectorAll('.mermaid') }});
      }}
    }}
    function renderList() {{
      const query = search.value.toLowerCase();
      list.innerHTML = '';
      for (const page of data.pages) {{
        if (query && !(page.title + ' ' + page.path + ' ' + page.body).toLowerCase().includes(query)) continue;
        const button = document.createElement('button');
        button.textContent = page.title;
        button.title = page.path;
        button.setAttribute('aria-current', page.id === active ? 'page' : 'false');
        button.addEventListener('click', () => {{ active = page.id; renderList(); renderContent(); }});
        list.append(button);
      }}
    }}
    search.addEventListener('input', renderList);
    renderList();
    renderContent();
  </script>
</body>
</html>
"#
    )
}

/// Renders a compact viewer report.
pub fn render_report(report: &ViewerReport) -> String {
    format!(
        "viewer wrote {} page(s) to {}\n",
        report.page_count,
        report.index_path.display()
    )
}

#[cfg(test)]
mod tests {
    use super::{ARCHITECTURE_SUMMARY_PAGE_ID, GRAPH_SUMMARY_PAGE_ID, generate};
    use crate::generation::MockModel;
    use crate::orchestrate::run_init;
    use std::path::Path;

    #[test]
    fn generates_static_viewer_index() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        let report = generate(temp.path(), &temp.path().join(".lithograph/viewer"))?;

        let html = std::fs::read_to_string(&report.index_path)?;
        assert!(html.contains("Search"));
        assert!(html.contains("mermaid"));
        assert_eq!(report.page_count, 22);

        Ok(())
    }

    #[test]
    fn viewer_exposes_graph_and_architecture_panes_from_persisted_data()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;

        let report = generate(temp.path(), &temp.path().join(".lithograph/viewer"))?;
        let html = std::fs::read_to_string(&report.index_path)?;

        // The graph and architecture panes are embedded as ordinary pages in
        // the same JSON payload the in-browser search operates over (AC1),
        // with content derived from the persisted graph snapshot rather than
        // any LLM-authored page (AC2).
        assert!(html.contains(GRAPH_SUMMARY_PAGE_ID));
        assert!(html.contains(ARCHITECTURE_SUMMARY_PAGE_ID));
        assert!(html.contains("Graph Summary"));
        assert!(html.contains("Architecture Summary"));
        assert!(html.contains("Node counts"));
        assert!(html.contains("Artifact:"));

        Ok(())
    }

    #[test]
    fn viewer_degrades_gracefully_with_no_graph_snapshot() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            r#"{"pages":[],"tasks":[]}"#,
        )?;

        let report = generate(temp.path(), &temp.path().join(".lithograph/viewer"))?;
        let html = std::fs::read_to_string(&report.index_path)?;

        assert_eq!(report.page_count, 2);
        assert!(html.contains("No graph nodes found."));
        assert!(html.contains("No architecture layer classification available."));

        Ok(())
    }

    fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn std::error::Error>> {
        for entry in walk_files(from)? {
            let relative = entry.strip_prefix(from)?;
            let destination = to.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&entry, &destination)?;
        }
        Ok(())
    }

    fn walk_files(root: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn std::error::Error>> {
        let mut files = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    files.push(path);
                }
            }
        }
        Ok(files)
    }
}
