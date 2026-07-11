//! Integration coverage for `lithograph inspect env`.

use assert_cmd::Command;
use serde_json::Value;
use std::error::Error;

#[test]
fn inspect_env_table_lists_users_and_resolution_reason() -> Result<(), Box<dyn Error>> {
    let output = inspect_env(["inspect", "env", "fixtures/polyglot"])?;

    assert!(output.contains("RIDGELINE_WORKER"));
    assert!(output.contains("code users:"));
    assert!(output.contains("reason:"));
    Ok(())
}

#[test]
fn inspect_env_json_is_deterministic_and_missing_filter_is_empty() -> Result<(), Box<dyn Error>> {
    let first = inspect_env([
        "inspect",
        "env",
        "fixtures/polyglot",
        "--variable",
        "RIDGELINE_WORKER",
        "--format",
        "json",
    ])?;
    let second = inspect_env([
        "inspect",
        "env",
        "fixtures/polyglot",
        "--variable",
        "RIDGELINE_WORKER",
        "--format",
        "json",
    ])?;
    assert_eq!(first, second);
    let parsed: Value = serde_json::from_str(&first)?;
    assert_eq!(parsed["variables"].as_array().map(Vec::len), Some(1));

    let missing = inspect_env([
        "inspect",
        "env",
        "fixtures/polyglot",
        "--variable",
        "DOES_NOT_EXIST",
        "--format",
        "json",
    ])?;
    let missing: Value = serde_json::from_str(&missing)?;
    assert_eq!(missing["variables"].as_array().map(Vec::len), Some(0));
    Ok(())
}

fn inspect_env<const N: usize>(args: [&str; N]) -> Result<String, Box<dyn Error>> {
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
