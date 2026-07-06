//! Static generated-wiki viewer output.

use crate::ask::WikiSearch;
use serde_json::json;
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

/// Generates a lightweight static viewer with navigation, search, and
/// browser-side Mermaid rendering when Mermaid is available from a local or
/// configured browser runtime.
pub fn generate(
    repo_root: &Path,
    output_dir: &Path,
) -> Result<ViewerReport, Box<dyn std::error::Error>> {
    let pages = WikiSearch.load_pages(repo_root)?;
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
    use super::generate;
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
        assert_eq!(report.page_count, 17);

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
