//! Fixture integrity tests.

use std::fs;
use std::path::{Path, PathBuf};

const FIXTURE_ROOT: &str = "fixtures/polyglot";

#[test]
fn polyglot_fixture_contains_required_artifact_types() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let required_paths = [
        "README.md",
        "docs/architecture.md",
        "src/python_app/__init__.py",
        "src/python_app/service.py",
        "rust/Cargo.toml",
        "rust/src/lib.rs",
        "rust/src/bin/worker.rs",
        "config/settings.yaml",
        "config/schema.json",
        "pyproject.toml",
        "requirements.txt",
        "Dockerfile",
        "docker-compose.yml",
        ".github/workflows/ci.yml",
        "Makefile",
        "web/package.json",
        "web/index.html",
        "web/src/App.tsx",
        "assets/logo.svg",
        "data/sample.bin",
        "generated/client.py",
        "vendor/example/lib.rs",
        "LICENSE",
    ];

    for relative_path in required_paths {
        assert!(
            root.join(relative_path).is_file(),
            "missing fixture artifact: {relative_path}"
        );
    }
}

#[test]
fn fixture_documentation_names_expected_categories() -> Result<(), Box<dyn std::error::Error>> {
    let readme = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE_ROOT)
            .join("README.md"),
    )?;

    for category in [
        "SourceCode",
        "Configuration",
        "Documentation",
        "PackageManifest",
        "ContainerDefinition",
        "ContinuousIntegration",
        "BuildDefinition",
        "StaticAsset",
        "BinaryAsset",
        "GeneratedSource",
    ] {
        assert!(
            readme.contains(category),
            "fixture README should document expected category {category}"
        );
    }

    Ok(())
}

#[test]
fn binary_fixture_contains_nul_byte() -> Result<(), Box<dyn std::error::Error>> {
    let bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(FIXTURE_ROOT)
            .join("data/sample.bin"),
    )?;

    assert!(
        bytes.contains(&0),
        "binary fixture should contain at least one NUL byte"
    );

    Ok(())
}

#[test]
fn fixture_paths_are_local_and_relative() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(FIXTURE_ROOT);
    let mut stack = vec![root.clone()];

    while let Some(path) = stack.pop() {
        for entry in fs::read_dir(&path)? {
            let entry = entry?;
            let entry_path = entry.path();
            let relative = entry_path.strip_prefix(&root)?;

            assert_is_relative_fixture_path(relative);

            if entry_path.is_dir() {
                stack.push(entry_path);
            }
        }
    }

    Ok(())
}

fn assert_is_relative_fixture_path(path: &Path) {
    let copy = PathBuf::from(path);

    assert!(
        copy.is_relative(),
        "fixture path should be relative: {}",
        copy.display()
    );
}
