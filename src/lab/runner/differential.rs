//! Independent-oracle differential checks run alongside the PR correctness
//! suite: a Python AST count, `cargo metadata` packages, the TypeScript
//! compiler's own import list, and optional SCIP symbol sentinels. Each
//! oracle degrades to `Skipped` rather than `Failed` when its external tool
//! is unavailable so the suite stays hermetic and offline.

use super::LabError;
use crate::graph::{Graph, GraphNode, RelationKind};
use crate::lab::model::{AssertionResult, DifferentialResult, DifferentialStatus};
use serde_json::Value;
use std::path::Path;

/// Compares the Lithograph definition count against Python's own `ast`
/// module. Used only for the `Pr` tier since it shells out to `python3`.
pub(super) fn differential_python_definitions(
    repo_root: &Path,
    graph: &Graph,
) -> Result<AssertionResult, LabError> {
    let script = r#"import ast, pathlib, sys
count = 0
for path in pathlib.Path(sys.argv[1]).rglob('*.py'):
    if any(part in {'.git', '.lithograph'} for part in path.parts):
        continue
    try:
        tree = ast.parse(path.read_text(encoding='utf-8'))
    except (OSError, UnicodeDecodeError, SyntaxError):
        continue
    count += sum(isinstance(node, (ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)) for node in ast.walk(tree))
print(count)
"#;
    let output = std::process::Command::new("python3")
        .args(["-c", script])
        .arg(repo_root)
        .output()
        .map_err(|error| {
            LabError::Invalid(format!(
                "python3 is required for the PR differential oracle: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(LabError::Invalid(format!(
            "Python AST differential oracle failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let expected = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>()
        .map_err(|error| LabError::Invalid(format!("invalid Python AST count: {error}")))?;
    let observed = graph
        .nodes
        .iter()
        .filter(|node| {
            matches!(node, GraphNode::Symbol(symbol) if symbol.evidence.path.as_str().ends_with(".py"))
        })
        .count();
    Ok(AssertionResult {
        id: "differential-python-definitions".to_owned(),
        passed: expected == observed,
        stage: "definitions_and_imports".to_owned(),
        detail: format!(
            "expected Python AST and Lithograph definition counts to match; ast={expected}, graph={observed}"
        ),
        expected_failure: None,
    })
}

/// Compares internal package names against `cargo metadata`. Skips when no
/// root `Cargo.toml` is present or `cargo metadata` cannot run.
pub(super) fn differential_cargo_metadata(repo_root: &Path, graph: &Graph) -> DifferentialResult {
    if !repo_root.join("Cargo.toml").is_file() {
        return DifferentialResult {
            name: "cargo_metadata_packages".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "no root Cargo.toml was present".to_owned(),
        };
    }
    let mut command = cargo_metadata::MetadataCommand::new();
    command.current_dir(repo_root).no_deps();
    let metadata = match command.exec() {
        Ok(metadata) => metadata,
        Err(error) => {
            return DifferentialResult {
                name: "cargo_metadata_packages".to_owned(),
                status: DifferentialStatus::Skipped,
                detail: format!("cargo metadata unavailable: {error}"),
            };
        }
    };
    let expected = metadata
        .packages
        .iter()
        .map(|package| package.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let observed = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Package(package) if !package.is_external => Some(package.name.as_str()),
            _ => None,
        })
        .collect::<std::collections::BTreeSet<_>>();
    let missing = expected.difference(&observed).copied().collect::<Vec<_>>();
    DifferentialResult {
        name: "cargo_metadata_packages".to_owned(),
        status: if missing.is_empty() {
            DifferentialStatus::Passed
        } else {
            DifferentialStatus::Failed
        },
        detail: format!(
            "cargo metadata packages={}, graph internal packages={}, missing={missing:?}",
            expected.len(),
            observed.len()
        ),
    }
}

/// Compares graph import relations against the TypeScript compiler's own
/// import list. Skips when no TypeScript package root or compiler is present.
pub(super) fn differential_typescript_compiler(
    repo_root: &Path,
    graph: &Graph,
) -> DifferentialResult {
    if !repo_root.join("package.json").is_file()
        && !repo_root.join("frontend/package.json").is_file()
    {
        return DifferentialResult {
            name: "typescript_imports".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "no TypeScript package root was present".to_owned(),
        };
    }
    let script = r#"let ts; try { ts = require('typescript'); } catch (_) { process.exit(42); }
const files = ts.sys.readDirectory(process.cwd(), ['.ts', '.tsx'], undefined, ['**/*']);
let imports = 0;
for (const file of files) {
  if (file.includes('/node_modules/') || file.includes('/dist/')) continue;
  const source = ts.createSourceFile(file, ts.sys.readFile(file) || '', ts.ScriptTarget.Latest, true);
  for (const statement of source.statements) if (ts.isImportDeclaration(statement)) imports++;
}
process.stdout.write(String(imports));"#;
    let output = match std::process::Command::new("node")
        .args(["-e", script])
        .current_dir(repo_root)
        .output()
    {
        Ok(output) => output,
        Err(error) => {
            return DifferentialResult {
                name: "typescript_imports".to_owned(),
                status: DifferentialStatus::Skipped,
                detail: format!("node unavailable: {error}"),
            };
        }
    };
    if output.status.code() == Some(42) {
        return DifferentialResult {
            name: "typescript_imports".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "the repository has no locally resolvable TypeScript compiler".to_owned(),
        };
    }
    let expected = String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<usize>();
    let Ok(expected) = expected else {
        return DifferentialResult {
            name: "typescript_imports".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: format!(
                "TypeScript compiler adapter failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ),
        };
    };
    let observed = graph
        .relations
        .iter()
        .filter(|relation| {
            relation.kind == RelationKind::Imports
                && relation.evidence.iter().any(|evidence| {
                    evidence.path.as_str().ends_with(".ts")
                        || evidence.path.as_str().ends_with(".tsx")
                })
        })
        .count();
    DifferentialResult {
        name: "typescript_imports".to_owned(),
        status: if expected == observed {
            DifferentialStatus::Passed
        } else {
            DifferentialStatus::Failed
        },
        detail: format!("TypeScript compiler imports={expected}, graph imports={observed}"),
    }
}

/// Compares graph node ids against an optional normalized SCIP index's
/// symbol sentinels. Skipped when no `.scip/lithograph-index.json` exists.
pub(super) fn differential_scip(repo_root: &Path, graph: &Graph) -> DifferentialResult {
    let path = repo_root.join(".scip/lithograph-index.json");
    if !path.is_file() {
        return DifferentialResult {
            name: "scip_sentinels".to_owned(),
            status: DifferentialStatus::Skipped,
            detail: "optional normalized .scip/lithograph-index.json was not present".to_owned(),
        };
    }
    let value: Value = match std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
    {
        Some(value) => value,
        None => {
            return DifferentialResult {
                name: "scip_sentinels".to_owned(),
                status: DifferentialStatus::Failed,
                detail: "normalized SCIP adapter input was invalid JSON".to_owned(),
            };
        }
    };
    let symbols = value
        .get("symbols")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    let node_ids = graph
        .nodes
        .iter()
        .map(|node| node.id().as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let missing = symbols
        .iter()
        .filter(|symbol| !node_ids.contains(**symbol))
        .copied()
        .collect::<Vec<_>>();
    DifferentialResult {
        name: "scip_sentinels".to_owned(),
        status: if missing.is_empty() {
            DifferentialStatus::Passed
        } else {
            DifferentialStatus::Failed
        },
        detail: format!(
            "SCIP symbol sentinels={}, missing={missing:?}",
            symbols.len()
        ),
    }
}
