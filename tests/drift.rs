//! Integration coverage for `lithograph drift`.

use assert_cmd::Command;
use serde_json::Value;
use std::error::Error;
use std::path::Path;

#[test]
fn drift_table_reports_no_drift_on_the_clean_fixture() -> Result<(), Box<dyn Error>> {
    let output = drift(["drift", "fixtures/polyglot"])?;

    assert_eq!(output, "no drift detected\n");

    Ok(())
}

#[test]
fn drift_json_reports_a_broken_link_on_a_repo_with_drift() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
        temp.path(),
    )?;
    std::fs::write(
        temp.path().join("docs/broken.md"),
        "See [missing](./does-not-exist.md) for details.\n",
    )?;

    let mut command = Command::cargo_bin("lithograph")?;
    let output = command
        .args([
            "drift",
            &temp.path().display().to_string(),
            "--format",
            "json",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let output = String::from_utf8(output)?;
    let parsed: Value = serde_json::from_str(&output)?;

    let findings = parsed["findings"].as_array().ok_or("findings array")?;
    assert!(findings.iter().any(
        |finding| finding["kind"] == "BrokenLink" && finding["detail"] == "./does-not-exist.md"
    ));

    Ok(())
}

fn drift<const N: usize>(args: [&str; N]) -> Result<String, Box<dyn Error>> {
    let mut command = Command::cargo_bin("lithograph")?;
    let output = command
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    Ok(String::from_utf8(output)?)
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn Error>> {
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

fn walk_files(root: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn Error>> {
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
