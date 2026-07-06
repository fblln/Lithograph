//! Integration coverage for `lithograph update`.

use assert_cmd::Command;
use std::error::Error;
use std::path::Path;

#[test]
fn update_after_init_is_a_no_op_then_regenerates_after_a_change() -> Result<(), Box<dyn Error>> {
    let temp = tempfile::TempDir::new()?;
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot"),
        temp.path(),
    )?;
    let repo = temp.path().display().to_string();

    Command::cargo_bin("lithograph")?
        .args(["init", &repo])
        .assert()
        .success();

    let no_op = Command::cargo_bin("lithograph")?
        .args(["update", &repo])
        .assert()
        .success();
    let no_op_output = String::from_utf8(no_op.get_output().stdout.clone())?;
    assert!(no_op_output.contains("pages regenerated: 0"));
    assert!(no_op_output.contains("changed artifacts: 0"));

    let readme = temp.path().join("README.md");
    let mut content = std::fs::read_to_string(&readme)?;
    content.push_str("\nUpdated by the update integration test.\n");
    std::fs::write(&readme, content)?;

    let changed = Command::cargo_bin("lithograph")?
        .args(["update", &repo])
        .assert()
        .success();
    let changed_output = String::from_utf8(changed.get_output().stdout.clone())?;
    assert!(changed_output.contains("changed artifacts: 1"));
    assert!(!changed_output.contains("pages regenerated: 0"));

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
