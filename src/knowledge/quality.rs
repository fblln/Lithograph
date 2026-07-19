//! Deterministic quality inspection for generated Lithograph docs.

use crate::agent::ask::WikiSearch;
use crate::manifest::PageManifest;
use crate::run::RepositorySnapshot;
use crate::storage::JsonStore;
use serde::Serialize;
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One quality finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct QualityFinding {
    /// Stable finding category.
    pub kind: QualityFindingKind,
    /// Repository-relative generated file or metadata path.
    pub path: String,
    /// Actionable detail.
    pub detail: String,
}

/// Quality finding categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub(crate) enum QualityFindingKind {
    /// A generated page has no recorded evidence refs.
    PageWithoutEvidence,
    /// A generated page still contains an unresolved-question section.
    UnresolvedQuestions,
    /// A generated page has an empty Mermaid block.
    EmptyMermaid,
    /// A module page has too little source coverage.
    WeakModuleCoverage,
    /// A source evidence line does not link or cite a source reference.
    MissingSourceLink,
    /// A generated Markdown link points at a missing generated doc.
    BrokenGeneratedDocLink,
    /// A cited evidence ref's source artifact no longer exists, or was
    /// never part of the last recorded repository snapshot (LIT-22.7.5 AC3).
    InvalidSourceRef,
    /// A cited evidence ref's source artifact exists but its content has
    /// changed since the page was generated (LIT-22.7.5 AC3): the citation
    /// may no longer accurately support the claim it was backing.
    StaleEvidence,
}

/// Quality inspection report.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct QualityReport {
    /// Findings sorted by path and kind.
    pub findings: Vec<QualityFinding>,
}

impl QualityReport {
    /// True when no findings were emitted.
    pub(crate) fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }
}

/// Runs deterministic quality checks over generated docs and metadata.
pub(crate) fn inspect(repo_root: &Path) -> Result<QualityReport, Box<dyn std::error::Error>> {
    let manifest_path = repo_root.join(".lithograph/manifest.json");
    let manifest = PageManifest::from_json(&std::fs::read_to_string(manifest_path)?)?;
    let pages = WikiSearch.load_pages(repo_root)?;
    let generated_paths: BTreeSet<String> = manifest
        .pages
        .iter()
        .map(|page| page.path.clone())
        .collect();
    let snapshot: Option<RepositorySnapshot> =
        JsonStore.read(&repo_root.join(".lithograph/snapshot.json"))?;
    let mut findings = Vec::new();

    for page in manifest.pages {
        let body = pages
            .iter()
            .find(|loaded| loaded.id == page.id)
            .map(|loaded| loaded.body.as_str())
            .unwrap_or("");
        if page.evidence.is_empty() {
            findings.push(QualityFinding {
                kind: QualityFindingKind::PageWithoutEvidence,
                path: page.path.clone(),
                detail: "manifest records no cited source evidence".to_owned(),
            });
        }
        if body.to_lowercase().contains("unresolved question") {
            findings.push(QualityFinding {
                kind: QualityFindingKind::UnresolvedQuestions,
                path: page.path.clone(),
                detail: "page still contains unresolved questions".to_owned(),
            });
        }
        for (line, detail) in empty_mermaid_blocks(body) {
            findings.push(QualityFinding {
                kind: QualityFindingKind::EmptyMermaid,
                path: page.path.clone(),
                detail: format!("{detail} at line {line}"),
            });
        }
        if page.module_id.is_some() && page.dependencies.len() < 2 {
            findings.push(QualityFinding {
                kind: QualityFindingKind::WeakModuleCoverage,
                path: page.path.clone(),
                detail: "module page depends on fewer than two graph nodes".to_owned(),
            });
        }
        if body.contains("## Source Evidence") && !source_evidence_has_links(body) {
            findings.push(QualityFinding {
                kind: QualityFindingKind::MissingSourceLink,
                path: page.path.clone(),
                detail: "source evidence section has no link or inline source reference".to_owned(),
            });
        }
        for broken in broken_generated_links(&page.path, body, &generated_paths) {
            findings.push(QualityFinding {
                kind: QualityFindingKind::BrokenGeneratedDocLink,
                path: page.path.clone(),
                detail: format!("generated-doc link `{broken}` does not resolve"),
            });
        }
        findings.extend(stale_or_invalid_evidence(
            repo_root,
            &page,
            snapshot.as_ref(),
        ));
    }

    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(format!("{:?}", a.kind).cmp(&format!("{:?}", b.kind)))
            .then(a.detail.cmp(&b.detail))
    });
    Ok(QualityReport { findings })
}

fn empty_mermaid_blocks(body: &str) -> Vec<(usize, String)> {
    let mut findings = Vec::new();
    let mut in_mermaid = false;
    let mut start = 0usize;
    let mut saw_content = false;
    for (index, line) in body.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if !in_mermaid && trimmed.eq_ignore_ascii_case("```mermaid") {
            in_mermaid = true;
            start = line_number;
            saw_content = false;
            continue;
        }
        if in_mermaid && trimmed == "```" {
            if !saw_content {
                findings.push((start, "empty Mermaid block".to_owned()));
            }
            in_mermaid = false;
            continue;
        }
        if in_mermaid && !trimmed.is_empty() {
            saw_content = true;
        }
    }
    findings
}

fn source_evidence_has_links(body: &str) -> bool {
    body.lines()
        .skip_while(|line| *line != "## Source Evidence")
        .skip(1)
        .take_while(|line| !line.starts_with("## "))
        .any(|line| line.contains("](") || line.contains("#L") || line.contains("`"))
}

fn broken_generated_links(
    page_path: &str,
    body: &str,
    generated_paths: &BTreeSet<String>,
) -> Vec<String> {
    let mut broken = Vec::new();
    for target in markdown_links(body) {
        if target.starts_with("http://")
            || target.starts_with("https://")
            || target.starts_with('#')
            || !target.ends_with(".md")
        {
            continue;
        }
        let resolved = resolve_doc_link(page_path, &target);
        if resolved.starts_with("docs/lithograph/") && !generated_paths.contains(&resolved) {
            broken.push(target);
        }
    }
    broken.sort();
    broken.dedup();
    broken
}

/// Checks each of `page`'s cited evidence refs against `snapshot` (the
/// last recorded repository state) and the current filesystem
/// (LIT-22.7.5 AC3): a citation whose source no longer exists, or was
/// never in the last snapshot, is `InvalidSourceRef`; one whose source
/// still exists but now hashes differently is `StaleEvidence`. Skips
/// entirely (no findings) when no snapshot has been recorded yet, since
/// there is nothing to compare against.
fn stale_or_invalid_evidence(
    repo_root: &Path,
    page: &crate::manifest::DocumentationPage,
    snapshot: Option<&RepositorySnapshot>,
) -> Vec<QualityFinding> {
    let Some(snapshot) = snapshot else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for evidence in &page.evidence {
        let relative = evidence.path.as_str();
        let absolute = repo_root.join(relative);
        if !absolute.is_file() {
            findings.push(QualityFinding {
                kind: QualityFindingKind::InvalidSourceRef,
                path: page.path.clone(),
                detail: format!("cited evidence artifact `{relative}` no longer exists"),
            });
            continue;
        }
        let Some(recorded_hash) = snapshot.artifact_hashes.get(relative) else {
            findings.push(QualityFinding {
                kind: QualityFindingKind::InvalidSourceRef,
                path: page.path.clone(),
                detail: format!(
                    "cited evidence artifact `{relative}` was not part of the last recorded repository snapshot"
                ),
            });
            continue;
        };
        let Ok(bytes) = std::fs::read(&absolute) else {
            continue;
        };
        let current_hash = blake3::hash(&bytes).to_hex().to_string();
        if &current_hash != recorded_hash {
            findings.push(QualityFinding {
                kind: QualityFindingKind::StaleEvidence,
                path: page.path.clone(),
                detail: format!(
                    "cited evidence artifact `{relative}` has changed since this page was generated"
                ),
            });
        }
    }
    findings
}

fn markdown_links(body: &str) -> Vec<String> {
    let mut links = Vec::new();
    for part in body.split("](").skip(1) {
        if let Some((target, _)) = part.split_once(')') {
            links.push(target.split('#').next().unwrap_or(target).to_owned());
        }
    }
    links
}

fn resolve_doc_link(page_path: &str, target: &str) -> String {
    let base = PathBuf::from(page_path)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    normalize_path(&base.join(target))
}

fn normalize_path(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        let text = component.as_os_str().to_string_lossy();
        if text == "." {
            continue;
        }
        if text == ".." {
            let _ = parts.pop();
        } else {
            parts.push(text.into_owned());
        }
    }
    parts.join("/")
}

/// Renders a table-like quality report.
pub(crate) fn render_table(report: &QualityReport) -> String {
    if report.findings.is_empty() {
        return "quality: clean\n".to_owned();
    }
    let mut output = format!("{} quality finding(s):\n", report.findings.len());
    for finding in &report.findings {
        output.push_str(&format!(
            "  [{:?}] {}: {}\n",
            finding.kind, finding.path, finding.detail
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{QualityFindingKind, inspect};
    use crate::generation::MockModel;
    use crate::orchestrate::run_init;
    use std::path::Path;

    #[test]
    fn clean_generated_fixture_has_no_quality_findings() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::create_dir_all(temp.path().join("docs/lithograph"))?;
        std::fs::write(
            temp.path().join("docs/lithograph/overview.md"),
            "# Overview\n\nBody.\n\n## Source Evidence\n- `src/lib.rs#L1-L2`\n",
        )?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            r#"{
  "pages": [
    {
      "id": "page:overview",
      "path": "docs/lithograph/overview.md",
      "module_id": null,
      "dependencies": ["artifact:src/lib.rs"],
      "evidence": [
        {
          "artifact_id": "artifact:src/lib.rs",
          "path": "src/lib.rs",
          "span": { "start_line": 1, "end_line": 2 },
          "structured_path": null
        }
      ],
      "input_hash": "hash",
      "output_hash": "hash",
      "prompt_version": "v1",
      "context_schema_version": "overview-context-v1"
    }
  ],
  "tasks": []
}
"#,
        )?;

        let report = inspect(temp.path())?;

        assert!(report.is_clean());
        Ok(())
    }

    #[test]
    fn reports_broken_generated_doc_links_and_empty_mermaid()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        std::fs::write(
            temp.path().join("docs/lithograph/overview.md"),
            "# Overview\n\n[missing](./missing.md)\n\n```mermaid\n```\n",
        )?;

        let report = inspect(temp.path())?;

        assert!(report.findings.iter().any(|finding| {
            finding.kind == QualityFindingKind::BrokenGeneratedDocLink
                && finding.detail.contains("missing.md")
        }));
        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::EmptyMermaid)
        );

        Ok(())
    }

    fn first_cited_evidence_path(repo_root: &Path) -> Result<String, Box<dyn std::error::Error>> {
        let manifest = crate::manifest::PageManifest::from_json(&std::fs::read_to_string(
            repo_root.join(".lithograph/manifest.json"),
        )?)?;
        manifest
            .pages
            .iter()
            .find_map(|page| page.evidence.first())
            .map(|evidence| evidence.path.as_str().to_owned())
            .ok_or_else(|| "no page cited any evidence".into())
    }

    /// LIT-22.7.5 AC3/AC4: a cited evidence artifact that no longer exists
    /// on disk fails validation as `InvalidSourceRef`.
    #[test]
    fn invalid_source_ref_reported_when_cited_artifact_is_deleted()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let cited_path = first_cited_evidence_path(temp.path())?;
        std::fs::remove_file(temp.path().join(&cited_path))?;

        let report = inspect(temp.path())?;

        assert!(report.findings.iter().any(|finding| {
            finding.kind == QualityFindingKind::InvalidSourceRef
                && finding.detail.contains(&cited_path)
        }));

        Ok(())
    }

    /// LIT-22.7.5 AC3/AC4: a cited evidence artifact whose content changed
    /// since the page was generated fails validation as `StaleEvidence`.
    #[test]
    fn stale_evidence_reported_when_cited_artifact_content_changes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        copy_dir(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
            temp.path(),
        )?;
        run_init(temp.path(), &MockModel, "mock", "v1")?;
        let cited_path = first_cited_evidence_path(temp.path())?;
        let absolute = temp.path().join(&cited_path);
        let mut content = std::fs::read_to_string(&absolute)?;
        content.push_str("\n# a deliberate drift-inducing change\n");
        std::fs::write(&absolute, content)?;

        let report = inspect(temp.path())?;

        assert!(report.findings.iter().any(|finding| {
            finding.kind == QualityFindingKind::StaleEvidence
                && finding.detail.contains(&cited_path)
        }));

        Ok(())
    }

    /// LIT-22.7.5 AC3/AC4: without a recorded snapshot yet, evidence
    /// freshness cannot be checked, so no `InvalidSourceRef`/`StaleEvidence`
    /// findings are emitted (rather than false positives on a fresh repo).
    #[test]
    fn evidence_freshness_checks_are_skipped_without_a_snapshot()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::create_dir_all(temp.path().join("docs/lithograph"))?;
        std::fs::write(
            temp.path().join("docs/lithograph/overview.md"),
            "# Overview\n\nBody.\n\n## Source Evidence\n- `src/lib.rs#L1-L2`\n",
        )?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            r#"{
  "pages": [
    {
      "id": "page:overview",
      "path": "docs/lithograph/overview.md",
      "module_id": null,
      "dependencies": ["artifact:src/lib.rs"],
      "evidence": [
        {
          "artifact_id": "artifact:src/lib.rs",
          "path": "src/lib.rs",
          "span": { "start_line": 1, "end_line": 2 },
          "structured_path": null
        }
      ],
      "input_hash": "hash",
      "output_hash": "hash",
      "prompt_version": "v1",
      "context_schema_version": "overview-context-v1"
    }
  ],
  "tasks": []
}
"#,
        )?;

        let report = inspect(temp.path())?;

        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::InvalidSourceRef)
        );
        assert!(
            !report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::StaleEvidence)
        );

        Ok(())
    }

    fn manifest_with_one_page(page_json: &str) -> String {
        format!("{{\n  \"pages\": [{page_json}],\n  \"tasks\": []\n}}\n")
    }

    /// LIT-22.7.5 AC1/AC4: a page manifest entry with no cited evidence
    /// fails validation (a factual claim must be backed by evidence).
    #[test]
    fn page_without_evidence_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::create_dir_all(temp.path().join("docs/lithograph"))?;
        std::fs::write(
            temp.path().join("docs/lithograph/overview.md"),
            "# Overview\n\nBody.\n",
        )?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            manifest_with_one_page(
                r#"{
      "id": "page:overview",
      "path": "docs/lithograph/overview.md",
      "module_id": null,
      "dependencies": [],
      "evidence": [],
      "input_hash": "hash",
      "output_hash": "hash",
      "prompt_version": "v1",
      "context_schema_version": "overview-context-v1"
    }"#,
            ),
        )?;

        let report = inspect(temp.path())?;

        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::PageWithoutEvidence)
        );

        Ok(())
    }

    /// LIT-22.7.5 AC2/AC4: a page whose body still contains an unresolved
    /// question is reported rather than silently accepted as settled fact.
    #[test]
    fn unresolved_questions_in_body_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::create_dir_all(temp.path().join("docs/lithograph"))?;
        std::fs::write(
            temp.path().join("docs/lithograph/overview.md"),
            "# Overview\n\nUnresolved question: what does this module actually do?\n",
        )?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            manifest_with_one_page(
                r#"{
      "id": "page:overview",
      "path": "docs/lithograph/overview.md",
      "module_id": null,
      "dependencies": [],
      "evidence": [],
      "input_hash": "hash",
      "output_hash": "hash",
      "prompt_version": "v1",
      "context_schema_version": "overview-context-v1"
    }"#,
            ),
        )?;

        let report = inspect(temp.path())?;

        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::UnresolvedQuestions)
        );

        Ok(())
    }

    /// LIT-22.7.5 AC4: a module page depending on fewer than two graph
    /// nodes is reported as weakly covered.
    #[test]
    fn weak_module_coverage_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::create_dir_all(temp.path().join("docs/lithograph/modules"))?;
        std::fs::write(
            temp.path().join("docs/lithograph/modules/tiny.md"),
            "# Tiny\n\nBody.\n",
        )?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            manifest_with_one_page(
                r#"{
      "id": "page:module:tiny",
      "path": "docs/lithograph/modules/tiny.md",
      "module_id": "module-plan:tiny",
      "dependencies": ["artifact:src/tiny.rs"],
      "evidence": [],
      "input_hash": "hash",
      "output_hash": "hash",
      "prompt_version": "v1",
      "context_schema_version": "module-context-v1"
    }"#,
            ),
        )?;

        let report = inspect(temp.path())?;

        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::WeakModuleCoverage)
        );

        Ok(())
    }

    /// LIT-22.7.5 AC4: a "## Source Evidence" section with no actual link
    /// or inline source reference is reported.
    #[test]
    fn missing_source_link_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join(".lithograph"))?;
        std::fs::create_dir_all(temp.path().join("docs/lithograph"))?;
        std::fs::write(
            temp.path().join("docs/lithograph/overview.md"),
            "# Overview\n\nBody.\n\n## Source Evidence\nSee the source for details.\n",
        )?;
        std::fs::write(
            temp.path().join(".lithograph/manifest.json"),
            manifest_with_one_page(
                r#"{
      "id": "page:overview",
      "path": "docs/lithograph/overview.md",
      "module_id": null,
      "dependencies": [],
      "evidence": [],
      "input_hash": "hash",
      "output_hash": "hash",
      "prompt_version": "v1",
      "context_schema_version": "overview-context-v1"
    }"#,
            ),
        )?;

        let report = inspect(temp.path())?;

        assert!(
            report
                .findings
                .iter()
                .any(|finding| finding.kind == QualityFindingKind::MissingSourceLink)
        );

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
