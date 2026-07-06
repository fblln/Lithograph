//! Integration coverage for `lithograph inspect graph`.

use assert_cmd::Command;
use serde_json::Value;
use std::error::Error;

#[test]
fn inspect_graph_table_lists_node_and_relation_counts() -> Result<(), Box<dyn Error>> {
    let output = inspect_graph(["inspect", "graph", "fixtures/polyglot"])?;

    assert!(output.contains("nodes:"));
    assert!(output.contains("relations:"));
    assert!(output.contains("artifact"));

    Ok(())
}

#[test]
fn inspect_graph_json_is_deterministic_and_valid() -> Result<(), Box<dyn Error>> {
    let first = inspect_graph(["inspect", "graph", "fixtures/polyglot", "--format", "json"])?;
    let second = inspect_graph(["inspect", "graph", "fixtures/polyglot", "--format", "json"])?;
    let parsed: Value = serde_json::from_str(&first)?;

    assert_eq!(first, second);
    let nodes = parsed["nodes"].as_array().ok_or("nodes array")?;
    let relations = parsed["relations"].as_array().ok_or("relations array")?;
    assert!(!nodes.is_empty());
    assert!(!relations.is_empty());
    assert!(nodes.iter().any(|node| node["node_type"] == "Artifact"));

    Ok(())
}

fn inspect_graph<const N: usize>(args: [&str; N]) -> Result<String, Box<dyn Error>> {
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
