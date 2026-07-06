//! Integration coverage for `lithograph init`.

use assert_cmd::Command;
use std::error::Error;
use std::path::Path;

#[test]
fn init_writes_docs_and_manifest_for_the_fixture() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
        temp.path(),
    )?;

    let mut command = Command::cargo_bin("lithograph")?;
    let assert = command
        .args(["init", &temp.path().display().to_string()])
        .assert()
        .success();
    let output = String::from_utf8(assert.get_output().stdout.clone())?;

    assert!(output.contains("artifacts:"));
    assert!(output.contains("modules:"));
    assert!(output.contains("pages written:"));
    assert!(output.contains("changed artifacts:"));
    assert!(temp.path().join("docs/lithograph/quickstart.md").exists());
    assert!(temp.path().join("docs/lithograph/architecture.md").exists());
    assert!(temp.path().join(".lithograph/graph.json").exists());
    assert!(temp.path().join(".lithograph/manifest.json").exists());
    assert!(temp.path().join(".lithograph/snapshot.json").exists());
    assert!(temp.path().join(".lithograph/run.json").exists());

    let run_json = std::fs::read_to_string(temp.path().join(".lithograph/run.json"))?;
    assert!(run_json.contains("\"command\": \"init\""));
    assert!(run_json.contains("\"changed_artifacts\""));

    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn Error>> {
    let mut stack = vec![from.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            let destination = to.join(path.strip_prefix(from)?);
            if path.is_dir() {
                std::fs::create_dir_all(&destination)?;
                stack.push(path);
            } else {
                if let Some(parent) = destination.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&path, &destination)?;
            }
        }
    }
    Ok(())
}
