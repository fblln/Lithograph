//! End-to-end coverage for the baseline lab development binary.

use assert_cmd::Command;
use predicates::prelude::*;
use std::error::Error;

#[test]
fn help_exposes_complete_diagnostic_workflow() -> Result<(), Box<dyn Error>> {
    Command::cargo_bin("lithograph-lab")?
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("corpus"))
        .stdout(predicate::str::contains("replay"))
        .stdout(predicate::str::contains("accept"))
        .stdout(predicate::str::contains("benchmark"))
        .stdout(predicate::str::contains("mcp"));
    Ok(())
}

#[test]
fn pr_baseline_check_is_hermetic_and_clean() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    Command::cargo_bin("lithograph-lab")?
        .args(["--root", temp.path().to_str().ok_or("non-UTF-8 temp path")?])
        .args(["check", "--suite", "pr"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"changes\": []"));
    Ok(())
}

#[test]
fn mcp_surface_is_read_only_and_discoverable() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    Command::cargo_bin("lithograph-lab")?
        .args(["--root", temp.path().to_str().ok_or("non-UTF-8 temp path")?])
        .arg("mcp")
        .write_stdin(concat!(
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{}}\n",
            "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\",\"params\":{}}\n"
        ))
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "\"protocolVersion\":\"2025-06-18\"",
        ))
        .stdout(predicate::str::contains("inspect_run"))
        .stdout(predicate::str::contains("explain_assertion"))
        .stdout(predicate::str::contains("\"name\":\"accept\"").not());
    Ok(())
}

#[test]
fn benchmark_preserves_append_only_samples_and_mode() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    let root = temp.path().to_str().ok_or("non-UTF-8 temp path")?;
    Command::cargo_bin("lithograph-lab")?
        .args(["--root", root])
        .args([
            "benchmark",
            "--suite",
            "pr",
            "--case",
            "diagnostic",
            "--samples",
            "3",
            "--mode",
            "no-op",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"mode\": \"no_op\""))
        .stdout(predicate::str::contains("\"sample_files\""));
    let sample_root = temp.path().join("performance/samples/diagnostic/no_op");
    assert_eq!(std::fs::read_dir(&sample_root)?.count(), 3);

    Command::cargo_bin("lithograph-lab")?
        .args(["--root", root])
        .args([
            "benchmark",
            "--suite",
            "pr",
            "--case",
            "diagnostic",
            "--samples",
            "3",
            "--mode",
            "no-op",
        ])
        .assert()
        .success();
    assert_eq!(std::fs::read_dir(sample_root)?.count(), 6);
    Ok(())
}
