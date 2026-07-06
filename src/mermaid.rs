//! Markdown Mermaid fence discovery and optional local Node validation.

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
    use super::validate;

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
}
