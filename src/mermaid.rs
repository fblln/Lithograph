//! Markdown Mermaid fence discovery, structural/id validation, and an
//! optional local Node validation pass.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// One Mermaid validation finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidIssue {
    /// Markdown file path.
    pub path: PathBuf,
    /// One-based fence index within the file.
    pub fence_index: usize,
    /// One-based start line.
    pub line: usize,
    /// Parser or structural error.
    pub error: String,
}

/// Validation report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MermaidReport {
    /// Issues sorted by file and line.
    pub issues: Vec<MermaidIssue>,
    /// Number of Mermaid fences checked.
    pub fences_checked: usize,
}

impl MermaidReport {
    /// True when every Mermaid fence validated.
    pub fn is_clean(&self) -> bool {
        self.issues.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MermaidFence {
    path: PathBuf,
    index: usize,
    line: usize,
    body: String,
    closed: bool,
}

/// Validates Mermaid fences under `path`.
pub fn validate(
    path: &Path,
    node_validator: Option<&Path>,
) -> Result<MermaidReport, Box<dyn std::error::Error>> {
    let markdown_files = markdown_files(path)?;
    let mut issues = Vec::new();
    let mut fences_checked = 0usize;
    for file in markdown_files {
        for fence in fences_in_file(&file)? {
            fences_checked += 1;
            if !fence.closed {
                issues.push(issue(&fence, "Mermaid fence is not closed"));
                continue;
            }
            if fence.body.trim().is_empty() {
                issues.push(issue(&fence, "Mermaid fence is empty"));
                continue;
            }
            for detail in validate_node_ids(&fence.body) {
                issues.push(issue(&fence, &detail));
            }
            if let Some(validator) = node_validator
                && let Err(error) = run_node_validator(validator, &fence.body)
            {
                issues.push(issue(&fence, &error));
            }
        }
    }
    issues.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.fence_index.cmp(&b.fence_index))
    });
    Ok(MermaidReport {
        issues,
        fences_checked,
    })
}

fn issue(fence: &MermaidFence, error: &str) -> MermaidIssue {
    MermaidIssue {
        path: fence.path.clone(),
        fence_index: fence.index,
        line: fence.line,
        error: error.to_owned(),
    }
}

fn markdown_files(path: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    if path.is_file() {
        return Ok(vec![path.to_path_buf()]);
    }
    let mut files = Vec::new();
    collect_markdown(path, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_markdown(path: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    if !path.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(path)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_markdown(&path, files)?;
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("md") {
            files.push(path);
        }
    }
    Ok(())
}

fn fences_in_file(path: &Path) -> Result<Vec<MermaidFence>, std::io::Error> {
    let body = std::fs::read_to_string(path)?;
    let mut fences = Vec::new();
    let mut in_mermaid = false;
    let mut start_line = 0usize;
    let mut current = String::new();
    let mut index = 0usize;
    for (line_index, line) in body.lines().enumerate() {
        let line_number = line_index + 1;
        let trimmed = line.trim();
        if !in_mermaid && trimmed.eq_ignore_ascii_case("```mermaid") {
            in_mermaid = true;
            start_line = line_number;
            current.clear();
            index += 1;
            continue;
        }
        if in_mermaid && trimmed == "```" {
            fences.push(MermaidFence {
                path: path.to_path_buf(),
                index,
                line: start_line,
                body: current.clone(),
                closed: true,
            });
            in_mermaid = false;
            continue;
        }
        if in_mermaid {
            current.push_str(line);
            current.push('\n');
        }
    }
    if in_mermaid {
        fences.push(MermaidFence {
            path: path.to_path_buf(),
            index,
            line: start_line,
            body: current,
            closed: false,
        });
    }
    Ok(fences)
}

fn run_node_validator(validator: &Path, body: &str) -> Result<(), String> {
    let mut child = Command::new("node")
        .arg(validator)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("failed to start Node validator: {error}"))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "failed to open Node validator stdin".to_owned())?;
    stdin
        .write_all(body.as_bytes())
        .map_err(|error| format!("failed to write Mermaid to validator: {error}"))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .map_err(|error| format!("Node validator failed: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if stderr.is_empty() {
            format!("Node validator exited with {}", output.status)
        } else {
            stderr
        })
    }
}

/// Checks that every node declaration (`id[Label]`, `id(Label)`, or
/// `id{Label}`) in `fence_body` uses an ASCII, identifier-shaped id kept
/// separate from its bracketed label (LIT-22.7.2 AC1). Only the token
/// immediately before the bracket is checked -- the id -- not the label
/// text inside the brackets, so a label containing spaces or punctuation
/// never triggers a finding.
pub fn validate_node_ids(fence_body: &str) -> Vec<String> {
    let mut issues = Vec::new();
    for (line_index, line) in fence_body.lines().enumerate() {
        let Some(id) = declared_node_id(line) else {
            continue;
        };
        if !is_safe_id(id) {
            issues.push(format!(
                "line {}: node id `{id}` must be ASCII letters, digits, `_`, or `-`, with its label kept inside brackets separate from the id",
                line_index + 1
            ));
        }
    }
    issues
}

/// Returns the id token immediately preceding a node's opening bracket on
/// `line`, if the line declares one. `None` for lines with no bracket
/// (plain edges, directives) or an empty id (an anonymous/malformed
/// bracket the caller isn't expected to name).
fn declared_node_id(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let bracket_index = trimmed.find(['[', '(', '{'])?;
    let before = &trimmed[..bracket_index];
    let id = before
        .char_indices()
        .rev()
        .find(|&(_, ch)| ch.is_whitespace())
        .map_or(before, |(index, ch)| &before[index + ch.len_utf8()..]);
    (!id.is_empty()).then_some(id)
}

fn is_safe_id(id: &str) -> bool {
    id.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

/// Rewrites `fence_body` so every unsafe node id (AC1) is replaced with a
/// deterministic, sequential ASCII id (`N1`, `N2`, ...), consistently
/// across its declaration and every edge/reference that uses the same
/// token, so the diagram still renders correctly rather than dangling
/// (LIT-22.7.2 AC3). Never called by [`validate`] or by any normal test
/// path -- only when a caller explicitly opts in.
pub fn fix_node_ids(fence_body: &str) -> String {
    let mut renames: BTreeMap<String, String> = BTreeMap::new();
    let mut next = 1usize;
    for line in fence_body.lines() {
        let Some(id) = declared_node_id(line) else {
            continue;
        };
        if !is_safe_id(id) && !renames.contains_key(id) {
            renames.insert(id.to_owned(), format!("N{next}"));
            next += 1;
        }
    }
    if renames.is_empty() {
        return fence_body.to_owned();
    }
    let mut fixed: String = fence_body
        .lines()
        .map(|line| replace_id_tokens(line, &renames))
        .collect::<Vec<_>>()
        .join("\n");
    fixed.push('\n');
    fixed
}

/// Replaces every whole occurrence of an id-shaped token in `line` that
/// matches a key in `renames`, leaving everything else (arrows, brackets,
/// quoted label text) untouched.
fn replace_id_tokens(line: &str, renames: &BTreeMap<String, String>) -> String {
    let mut output = String::with_capacity(line.len());
    let mut token = String::new();
    for ch in line.chars() {
        // A token boundary is whitespace or a Mermaid structural
        // character -- everything else (including non-ASCII bytes, so an
        // id like `caf\u{e9}` stays one token matching what
        // `declared_node_id` extracted) belongs to the token itself.
        if ch.is_whitespace() || matches!(ch, '[' | ']' | '(' | ')' | '{' | '}' | '|' | '"') {
            push_token(&mut output, &mut token, renames);
            output.push(ch);
        } else {
            token.push(ch);
        }
    }
    push_token(&mut output, &mut token, renames);
    output
}

fn push_token(output: &mut String, token: &mut String, renames: &BTreeMap<String, String>) {
    if !token.is_empty() {
        output.push_str(renames.get(token.as_str()).map_or(token.as_str(), |v| v));
        token.clear();
    }
}

/// Applies [`fix_node_ids`] to every Mermaid fence under `path`, rewriting
/// each Markdown file only when a fence actually changed. Returns the
/// number of files rewritten. Only invoked when a caller explicitly asks
/// for it (CLI `--fix`); never part of [`validate`] or normal tests.
pub fn fix_path(path: &Path) -> Result<usize, std::io::Error> {
    let mut files_changed = 0usize;
    for file in markdown_files(path)? {
        let original = std::fs::read_to_string(&file)?;
        let mut rewritten = String::with_capacity(original.len());
        let mut in_mermaid = false;
        let mut fence_body = String::new();
        for line in original.lines() {
            let trimmed = line.trim();
            if !in_mermaid && trimmed.eq_ignore_ascii_case("```mermaid") {
                in_mermaid = true;
                fence_body.clear();
                rewritten.push_str(line);
                rewritten.push('\n');
                continue;
            }
            if in_mermaid && trimmed == "```" {
                rewritten.push_str(&fix_node_ids(&fence_body));
                rewritten.push_str(line);
                rewritten.push('\n');
                in_mermaid = false;
                continue;
            }
            if in_mermaid {
                fence_body.push_str(line);
                fence_body.push('\n');
                continue;
            }
            rewritten.push_str(line);
            rewritten.push('\n');
        }
        if rewritten != original {
            std::fs::write(&file, rewritten)?;
            files_changed += 1;
        }
    }
    Ok(files_changed)
}

/// Renders validation results.
pub fn render_report(report: &MermaidReport) -> String {
    if report.issues.is_empty() {
        return format!("validated {} Mermaid fence(s)\n", report.fences_checked);
    }
    let mut output = format!("{} Mermaid validation issue(s):\n", report.issues.len());
    for issue in &report.issues {
        output.push_str(&format!(
            "  {} fence #{} line {}: {}\n",
            issue.path.display(),
            issue.fence_index,
            issue.line,
            issue.error
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{fix_node_ids, fix_path, validate, validate_node_ids};

    #[test]
    fn reports_file_fence_line_and_node_parser_error() -> Result<(), Box<dyn std::error::Error>> {
        if std::process::Command::new("node")
            .arg("--version")
            .output()
            .is_err()
        {
            return Ok(());
        }
        let temp = tempfile::TempDir::new()?;
        let markdown = temp.path().join("diagram.md");
        let validator = temp.path().join("validator.js");
        std::fs::write(
            &markdown,
            "# Diagram\n\n```mermaid\nflowchart TD\nA --> B\n```\n",
        )?;
        std::fs::write(
            &validator,
            "process.stdin.resume(); console.error('parse exploded'); process.exit(2);\n",
        )?;

        let report = validate(&markdown, Some(&validator))?;

        assert_eq!(report.issues.len(), 1);
        assert_eq!(report.issues[0].fence_index, 1);
        assert_eq!(report.issues[0].line, 3);
        assert!(report.issues[0].error.contains("parse exploded"));

        Ok(())
    }

    #[test]
    fn structural_validation_catches_empty_fences() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let markdown = temp.path().join("diagram.md");
        std::fs::write(&markdown, "```mermaid\n```\n")?;

        let report = validate(&markdown, None)?;

        assert_eq!(report.issues.len(), 1);
        assert!(report.issues[0].error.contains("empty"));

        Ok(())
    }

    /// LIT-22.7.2 AC1/AC4: a diagram with ASCII ids and bracket-separated
    /// labels validates clean.
    #[test]
    fn valid_ascii_ids_pass_validation() {
        let body = "flowchart TD\n  M1[\"Overview module\"]\n  M2[\"Config module\"]\n  M1 -->|Imports| M2\n";

        assert!(validate_node_ids(body).is_empty());
    }

    /// LIT-22.7.2 AC1/AC4: a node id containing a space or non-ASCII
    /// character is reported.
    #[test]
    fn invalid_node_ids_are_reported() {
        let body = "flowchart TD\n  caf\u{e9}[\"Unicode id\"]\n  data.point[\"Dotted id\"]\n";

        let issues = validate_node_ids(body);

        assert_eq!(issues.len(), 2);
        assert!(issues[0].contains("caf"));
        assert!(issues[1].contains("data.point"));
    }

    /// LIT-22.7.2 AC1/AC4: an end-to-end diagram with an unsafe node id
    /// fails full validation through the public `validate` entry point.
    #[test]
    fn diagram_with_invalid_id_fails_full_validation() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let markdown = temp.path().join("diagram.md");
        std::fs::write(
            &markdown,
            "```mermaid\nflowchart TD\n  caf\u{e9}[\"Overview\"]\n```\n",
        )?;

        let report = validate(&markdown, None)?;

        assert!(!report.is_clean());
        assert!(report.issues[0].error.contains("caf"));

        Ok(())
    }

    /// LIT-22.7.2 AC3: `validate` never mutates the file it inspects --
    /// the fixer is a separate, explicit path, never invoked implicitly.
    #[test]
    fn validate_never_modifies_the_file_it_checks() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let markdown = temp.path().join("diagram.md");
        let original = "```mermaid\nflowchart TD\n  caf\u{e9}[\"Overview\"]\n```\n";
        std::fs::write(&markdown, original)?;

        let report = validate(&markdown, None)?;

        assert!(!report.is_clean());
        assert_eq!(std::fs::read_to_string(&markdown)?, original);

        Ok(())
    }

    /// LIT-22.7.2 AC3: the explicit fixer replaces an unsafe id with a
    /// deterministic ASCII id everywhere it's used -- both the
    /// declaration and any edge referencing it -- so the diagram still
    /// renders correctly rather than dangling.
    #[test]
    fn fix_node_ids_renames_declaration_and_every_reference() {
        let body =
            "flowchart TD\n  caf\u{e9}[\"Overview\"]\n  Other[\"Other\"]\n  caf\u{e9} --> Other\n";

        let fixed = fix_node_ids(body);

        assert!(validate_node_ids(&fixed).is_empty());
        assert!(fixed.contains("N1[\"Overview\"]"));
        assert!(fixed.contains("N1 --> Other"));
        assert!(!fixed.contains('\u{e9}'));
    }

    /// LIT-22.7.2 AC3/AC4: `fix_node_ids` is a no-op on an already-safe
    /// diagram, and is only reachable through the explicit `fix_path`
    /// entry point -- never through `validate`.
    #[test]
    fn fix_node_ids_is_a_no_op_when_ids_are_already_safe() {
        let body = "flowchart TD\n  M1[\"Overview\"]\n  M1 --> M2\n";

        assert_eq!(fix_node_ids(body), body);
    }

    /// LIT-22.7.2 AC3/AC4: `fix_path` rewrites only the files that
    /// actually change, and the fixed file then validates clean.
    #[test]
    fn fix_path_rewrites_only_changed_files_and_result_validates_clean()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let broken = temp.path().join("broken.md");
        let clean = temp.path().join("clean.md");
        std::fs::write(
            &broken,
            "```mermaid\nflowchart TD\n  caf\u{e9}[\"Overview\"]\n```\n",
        )?;
        std::fs::write(
            &clean,
            "```mermaid\nflowchart TD\n  M1[\"Overview\"]\n```\n",
        )?;
        let clean_before = std::fs::read_to_string(&clean)?;

        let files_changed = fix_path(temp.path())?;

        assert_eq!(files_changed, 1);
        assert_eq!(std::fs::read_to_string(&clean)?, clean_before);
        let report = validate(temp.path(), None)?;
        assert!(report.is_clean());

        Ok(())
    }
}
