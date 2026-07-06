//! Integration coverage for `lithograph integrate-agents`.

use assert_cmd::Command;
use std::error::Error;

#[test]
fn integrate_agents_creates_refreshes_and_then_no_ops() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    std::fs::write(
        temp.path().join("AGENTS.md"),
        "# Agents\n\nExisting rules.\n",
    )?;
    let repo = temp.path().display().to_string();

    let created = Command::cargo_bin("lithograph")?
        .args(["integrate-agents", &repo])
        .assert()
        .success();
    let created_output = String::from_utf8(created.get_output().stdout.clone())?;
    assert!(created_output.contains("created"));
    let after_create = std::fs::read_to_string(temp.path().join("AGENTS.md"))?;
    assert!(after_create.starts_with("# Agents\n\nExisting rules.\n"));
    assert!(after_create.contains("docs/lithograph/quickstart.md"));

    let no_op = Command::cargo_bin("lithograph")?
        .args(["integrate-agents", &repo])
        .assert()
        .success();
    let no_op_output = String::from_utf8(no_op.get_output().stdout.clone())?;
    assert!(no_op_output.contains("unchanged"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("AGENTS.md"))?,
        after_create
    );

    let mut mutated = after_create.replace("docs/lithograph/quickstart.md", "stale/path.md");
    mutated = mutated.replace(
        "<!-- lithograph:end -->",
        "extra stale line\n<!-- lithograph:end -->",
    );
    std::fs::write(temp.path().join("AGENTS.md"), &mutated)?;

    let refreshed = Command::cargo_bin("lithograph")?
        .args(["integrate-agents", &repo])
        .assert()
        .success();
    let refreshed_output = String::from_utf8(refreshed.get_output().stdout.clone())?;
    assert!(refreshed_output.contains("refreshed"));
    let after_refresh = std::fs::read_to_string(temp.path().join("AGENTS.md"))?;
    assert!(after_refresh.contains("docs/lithograph/quickstart.md"));
    assert!(!after_refresh.contains("stale/path.md"));
    assert!(!after_refresh.contains("extra stale line"));
    assert!(after_refresh.starts_with("# Agents\n\nExisting rules.\n"));

    Ok(())
}

#[test]
fn integrate_agents_reports_when_no_files_exist() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    let repo = temp.path().display().to_string();

    let output = Command::cargo_bin("lithograph")?
        .args(["integrate-agents", &repo])
        .assert()
        .success();
    let output = String::from_utf8(output.get_output().stdout.clone())?;

    assert!(output.contains("nothing to do"));

    Ok(())
}
