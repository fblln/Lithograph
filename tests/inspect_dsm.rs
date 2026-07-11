//! Integration coverage for `lithograph inspect dsm`.
use assert_cmd::Command;
use serde_json::Value;

#[test]
fn inspect_dsm_table_and_json_are_deterministic() -> Result<(), Box<dyn std::error::Error>> {
    let table = run(["inspect", "dsm", "fixtures/polyglot"])?;
    assert!(table.contains("modules:"));
    assert!(table.contains("cycles:"));
    let first = run(["inspect", "dsm", "fixtures/polyglot", "--format", "json"])?;
    let second = run(["inspect", "dsm", "fixtures/polyglot", "--format", "json"])?;
    assert_eq!(first, second);
    let value: Value = serde_json::from_str(&first)?;
    assert!(value["modules"].is_array());
    assert!(value["cells"].is_array());
    Ok(())
}

fn run<const N: usize>(args: [&str; N]) -> Result<String, Box<dyn std::error::Error>> {
    let output = Command::cargo_bin("lithograph")?
        .args(args)
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    Ok(String::from_utf8(output)?)
}
