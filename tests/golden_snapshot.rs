//! Golden graph/docs/research snapshot coverage for the polyglot fixture
//! (LIT-22.2.5 AC2/AC3/AC4), running through the existing `golden` module
//! rather than a separate mechanism. To regenerate the committed
//! snapshot after an intentional output change, run:
//!
//! ```sh
//! cargo test --test golden_snapshot -- --ignored --nocapture
//! ```
//!
//! which updates `tests/golden/polyglot/`; review the diff, then commit
//! it alongside the change that caused it.

use lithograph::generation::MockModel;
use lithograph::golden::check_or_update;
use lithograph::orchestrate::run_init;
use std::path::Path;

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
}

fn golden_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden/polyglot")
}

#[test]
fn polyglot_docs_manifest_and_graph_match_committed_golden_snapshot()
-> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    copy_dir(&fixture_root(), temp.path())?;
    run_init(temp.path(), &MockModel, "mock", "v1")?;

    let report = check_or_update(temp.path(), &golden_dir(), false)?;

    assert!(report.is_clean());
    assert!(
        report
            .entries
            .iter()
            .any(|entry| entry.source.ends_with(".lithograph/graph/current.json"))
    );

    Ok(())
}

/// Regenerates the committed golden snapshot. Never run by `cargo test`
/// (AC4's `make check-all` only runs non-`#[ignore]` tests); run
/// explicitly (see module docs) after an intentional, reviewed output
/// change.
#[test]
#[ignore = "regenerates tests/golden/polyglot -- run explicitly, then review and commit the diff"]
fn update_golden_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    copy_dir(&fixture_root(), temp.path())?;
    run_init(temp.path(), &MockModel, "mock", "v1")?;

    check_or_update(temp.path(), &golden_dir(), true)?;

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
