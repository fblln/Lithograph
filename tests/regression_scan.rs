//! Scan-only regression coverage over real reference repositories checked
//! out alongside Lithograph, plus `/Users/fabio/Workspace/ridgeline`.
//!
//! These repos are not part of this checkout and are not guaranteed to
//! exist on every machine or CI runner, so this test is `#[ignore]`d by
//! default. Run it explicitly:
//!
//! ```sh
//! cargo test --test regression_scan -- --ignored --nocapture
//! ```
//!
//! It only walks the filesystem (no analyzers, no graph, no model calls),
//! so it never needs live model credentials and stays fast even on large
//! real repositories.

// Diagnostic output (which repos were found/skipped, artifact counts) is
// this test's whole point when run manually with `--nocapture`.
#![allow(clippy::print_stderr)]

use lithograph::domain::Artifact;
use lithograph::inventory::{RepositoryWalker, WalkOptions};
use std::error::Error;
use std::path::{Path, PathBuf};

const RIDGELINE_PATH: &str = "/Users/fabio/Workspace/ridgeline";

#[test]
#[ignore = "scans real external repos on this machine; run explicitly with `cargo test --test regression_scan -- --ignored --nocapture`"]
fn regression_scan_reference_repos_and_ridgeline() -> Result<(), Box<dyn Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let parent = manifest_dir
        .parent()
        .ok_or("Lithograph has no parent directory")?;

    let candidates: [(&str, PathBuf); 5] = [
        ("CodeWiki", parent.join("CodeWiki")),
        ("deepwiki-rs", parent.join("deepwiki-rs")),
        ("deepwiki-open", parent.join("deepwiki-open")),
        ("openwiki", parent.join("openwiki")),
        ("ridgeline", PathBuf::from(RIDGELINE_PATH)),
    ];

    let mut scanned = 0usize;
    for (name, path) in &candidates {
        if !path.is_dir() {
            eprintln!(
                "regression scan: skipping {name} ({} not found on this machine)",
                path.display()
            );
            continue;
        }

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(path)?;
        eprintln!("regression scan: {name}: {} artifacts", artifacts.len());
        assert!(!artifacts.is_empty(), "{name} scanned zero artifacts");
        scanned += 1;

        if *name == "ridgeline" {
            assert_ridgeline_coverage(&artifacts)?;
        }
    }

    if scanned == 0 {
        eprintln!(
            "regression scan: none of CodeWiki/deepwiki-rs/deepwiki-open/openwiki/ridgeline \
             were found on this machine; nothing to verify"
        );
    }

    Ok(())
}

fn assert_ridgeline_coverage(artifacts: &[Artifact]) -> Result<(), Box<dyn Error>> {
    let has_format = |wanted: &str| {
        artifacts
            .iter()
            .any(|artifact| artifact.detected_format.as_deref() == Some(wanted))
    };
    let has_extension = |wanted: &str| {
        artifacts
            .iter()
            .any(|artifact| artifact.path.as_str().ends_with(wanted))
    };
    let has_category = |wanted: lithograph::domain::ArtifactCategory| {
        artifacts.iter().any(|artifact| artifact.category == wanted)
    };

    let checks: [(&str, bool); 10] = [
        ("Python", has_format("python")),
        ("Rust", has_format("rust")),
        (
            "TypeScript/TSX",
            has_format("typescript") || has_format("tsx"),
        ),
        ("HTML", has_format("html")),
        (
            "Docker/Compose",
            has_format("dockerfile") || has_format("docker-compose"),
        ),
        ("Markdown", has_format("markdown")),
        // ponytail: classify.rs has no dedicated GPX support yet, so this
        // checks the raw extension rather than detected_format; upgrade
        // once/if a GPX profile is added.
        ("GPX", has_extension(".gpx")),
        (
            "image",
            has_format("png")
                || has_format("jpg")
                || has_format("jpeg")
                || has_format("gif")
                || has_format("svg"),
        ),
        (
            "lockfile",
            has_category(lithograph::domain::ArtifactCategory::DependencyLockfile),
        ),
        (
            "vendored",
            artifacts.iter().any(|artifact| artifact.vendored_score > 0),
        ),
    ];

    let missing: Vec<&str> = checks
        .into_iter()
        .filter(|(_, present)| !present)
        .map(|(name, _)| name)
        .collect();
    if !missing.is_empty() {
        return Err(
            format!("ridgeline scan is missing expected artifact kinds: {missing:?}").into(),
        );
    }

    Ok(())
}
