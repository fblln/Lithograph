//! Integration coverage for `lithograph inspect modules`.

use assert_cmd::Command;
use serde_json::Value;
use std::error::Error;

const TABLE_SNAPSHOT: &str = "\
kind            name            members  input_hash                                                        tokens
Configuration   Configuration        23  3ff68e804254ecdf6acd55f7e90933d940df4ef0e9f453d6a1f4f755c11c8b9a     295
Directory       assets                2  3029d94afd11745b29441c5ecd57daf06d7140c928d35bc44a3911eb472f769d      64
Directory       data                  1  6b99ef6840a59c785d9f18965146b65ed88f2f225487b77007c2ef5e5cbb0227       9
Directory       generated             3  af7c6864a5f8b357a9d86e0806a7739ea426ccbcceebf15fd1f4a462e06a3c40      37
Directory       root                  6  d9b071b9e6f27ac2909415dc4c2590b2ba32e0fcab686075d2b79db574615771      78
Directory       vendor                3  031399bb81b35a133503a3e11bf0af08d46a4064e5f34c019380e97d73812819      18
Directory       web                  14  684e2e68e5213a8ace17ed654a9a177835e7045586aebf6808961cbcddf9379c     151
Documentation   Documentation        13  072a11ff87434dd925fa9e1516a3e2271ed75b8cca6b7049bc82f65ac1584f09    1081
Infrastructure  Infrastructure       19  b504fade04dddb8d546b8a78b5e1a16e0692f96f8e34c926def164672be19873     384
PythonPackage   python_app           16  e6b2ad1a1b9b13f6bb619843c76721d3af8d54ad49b5007add9f863090c72157     333
RustCrate       fixture-worker       14  3326ff9d7b5b8c213d8e5329e3931e01e5fb4ad896fad50c7ca87ece04d4c69b     365
";

#[test]
fn inspect_modules_table_fixture_snapshot() -> Result<(), Box<dyn Error>> {
    let output = inspect_modules(["inspect", "modules", "fixtures/polyglot"])?;

    assert_eq!(output, TABLE_SNAPSHOT);

    Ok(())
}

#[test]
fn inspect_modules_json_is_deterministic_and_valid() -> Result<(), Box<dyn Error>> {
    let first = inspect_modules([
        "inspect",
        "modules",
        "fixtures/polyglot",
        "--format",
        "json",
    ])?;
    let second = inspect_modules([
        "inspect",
        "modules",
        "fixtures/polyglot",
        "--format",
        "json",
    ])?;
    let parsed: Value = serde_json::from_str(&first)?;

    assert_eq!(first, second);
    let modules = parsed.as_array().ok_or("modules array")?;
    assert_eq!(modules.len(), 11);
    assert!(
        modules
            .iter()
            .any(|module| module["name"] == "fixture-worker")
    );

    Ok(())
}

fn inspect_modules<const N: usize>(args: [&str; N]) -> Result<String, Box<dyn Error>> {
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
