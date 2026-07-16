//! Integration coverage for `lithograph path`, `explain`, and `affected`
//! (LIT-47).
//!
//! Each runs against a real graph built by `init` on the polyglot fixture,
//! rather than a hand-written graph, so the commands are exercised against
//! the node ids and relation shapes the pipeline actually produces.

use assert_cmd::Command;
use serde_json::Value;
use std::error::Error;
use std::path::Path;
use std::sync::OnceLock;

#[test]
fn path_reports_each_hop_with_kind_and_resolution() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    // `service.py` contains `RouteService`, so a short path exists. Both
    // ends are named loosely: resolution is by substring.
    let output = run(&["path", "--path", repo, "service.py", "RouteService"])?;

    assert!(
        output.contains("-->") || output.contains("<--"),
        "expected a directional hop, got:\n{output}"
    );
    assert!(
        output.contains("hop(s)"),
        "expected a hop count, got:\n{output}"
    );
    assert!(
        output.contains("[Contains"),
        "expected the relation kind on the hop, got:\n{output}"
    );

    Ok(())
}

#[test]
fn path_json_carries_hop_kind_and_resolution() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    let output = run(&[
        "path",
        "--path",
        repo,
        "service.py",
        "RouteService",
        "--format",
        "json",
    ])?;
    let parsed: Value = serde_json::from_str(&output)?;

    assert!(parsed["start"]["id"].is_string());
    let hops = parsed["hops"].as_array().ok_or("hops must be an array")?;
    assert!(!hops.is_empty(), "expected at least one hop");
    assert!(hops[0]["kind"].is_string());
    assert!(hops[0]["forward"].is_boolean());
    assert!(hops[0]["node"]["name"].is_string());

    Ok(())
}

#[test]
fn explain_prints_evidence_and_grouped_neighbors() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    let output = run(&["explain", "--path", repo, "RouteService"])?;

    assert!(output.contains("id:"), "got:\n{output}");
    assert!(output.contains("kind:   Symbol"), "got:\n{output}");
    assert!(output.contains("degree:"), "got:\n{output}");
    // Evidence must name a file and a span, not just exist.
    assert!(output.contains("source: "), "got:\n{output}");
    assert!(
        output.contains(".py:"),
        "expected a source span, got:\n{output}"
    );
    // Neighbors are grouped under a relation-kind heading with a count.
    assert!(
        output.contains("inbound:") || output.contains("outbound:"),
        "got:\n{output}"
    );

    Ok(())
}

#[test]
fn explain_json_groups_neighbors_by_relation_kind() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    let output = run(&[
        "explain",
        "--path",
        repo,
        "RouteService",
        "--format",
        "json",
    ])?;
    let parsed: Value = serde_json::from_str(&output)?;

    assert_eq!(parsed["node"]["label"], "Symbol");
    assert!(parsed["evidence"].as_array().is_some_and(|e| !e.is_empty()));
    let grouped = parsed["inbound"]
        .as_object()
        .into_iter()
        .chain(parsed["outbound"].as_object())
        .any(|group| !group.is_empty());
    assert!(
        grouped,
        "expected neighbors grouped by kind, got:\n{output}"
    );

    Ok(())
}

#[test]
fn affected_accepts_positional_targets_and_honors_depth() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    let output = run(&["affected", "--path", repo, "RouteService", "--depth", "1"])?;

    assert!(output.contains("dependent(s)"), "got:\n{output}");

    Ok(())
}

/// The pre-push use case: `git diff --name-only | lithograph affected --stdin`.
#[test]
fn affected_reads_targets_from_stdin_and_reports_unmatched_ones() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    let mut command = Command::cargo_bin("lithograph")?;
    let assert = command
        .args(["affected", "--path", repo, "--stdin", "--format", "json"])
        .write_stdin("src/python_app/service.py\nnot-a-real-file.py\n")
        .assert()
        .success();
    let output = String::from_utf8(assert.get_output().stdout.clone())?;
    let parsed: Value = serde_json::from_str(&output)?;

    let reports = parsed.as_array().ok_or("expected an array of reports")?;
    assert_eq!(reports.len(), 2, "both piped lines must be reported");
    assert_eq!(reports[0]["target"], "src/python_app/service.py");
    assert_eq!(reports[0]["matched"], true);
    // An unknown path is reported as unmatched rather than dropped: "no
    // dependents" and "never looked" must not read alike.
    assert_eq!(reports[1]["target"], "not-a-real-file.py");
    assert_eq!(reports[1]["matched"], false);

    Ok(())
}

#[test]
fn queries_fail_clearly_without_a_graph_store() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    let empty = temp.path().display().to_string();

    for args in [
        vec!["path", "--path", &empty, "a", "b"],
        vec!["explain", "--path", &empty, "a"],
        vec!["affected", "--path", &empty, "a"],
    ] {
        let mut command = Command::cargo_bin("lithograph")?;
        let assert = command.args(&args).assert().failure();
        let stderr = String::from_utf8(assert.get_output().stderr.clone())?;
        assert!(
            stderr.contains("no graph store") && stderr.contains("lithograph init"),
            "`{}` must name the fix, got: {stderr}",
            args[0]
        );
    }

    Ok(())
}

#[test]
fn unknown_nodes_are_reported_distinctly_from_missing_paths() -> Result<(), Box<dyn Error>> {
    let repo = built_fixture()?;

    let mut command = Command::cargo_bin("lithograph")?;
    let assert = command
        .args(["path", "--path", repo, "RouteService", "no-such-node-xyz"])
        .assert()
        .failure();
    let stderr = String::from_utf8(assert.get_output().stderr.clone())?;
    assert!(
        stderr.contains("no graph node matched `no-such-node-xyz`"),
        "a missing node must say so, got: {stderr}"
    );

    Ok(())
}

/// Builds the polyglot fixture once and shares it: `init` is the slow part of
/// these tests, and every case here is read-only.
fn built_fixture() -> Result<&'static str, Box<dyn Error>> {
    static REPO: OnceLock<Result<(tempfile::TempDir, String), String>> = OnceLock::new();
    let built = REPO.get_or_init(|| build_fixture().map_err(|error| error.to_string()));
    match built {
        Ok((_, path)) => Ok(path.as_str()),
        Err(error) => Err(error.clone().into()),
    }
}

fn build_fixture() -> Result<(tempfile::TempDir, String), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
        temp.path(),
    )?;
    let path = temp.path().display().to_string();
    Command::cargo_bin("lithograph")?
        .args(["init", &path])
        .assert()
        .success();
    Ok((temp, path))
}

fn run(args: &[&str]) -> Result<String, Box<dyn Error>> {
    let mut command = Command::cargo_bin("lithograph")?;
    let output = command.args(args).assert().success().get_output().clone();
    Ok(String::from_utf8(output.stdout)?)
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            std::fs::create_dir_all(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
