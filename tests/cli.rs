//! CLI integration tests.

use assert_cmd::Command;
use predicates::prelude::*;
use std::error::Error;

#[test]
fn help_output_describes_lithograph() -> Result<(), Box<dyn Error>> {
    let mut command = Command::cargo_bin("lithograph")?;

    command
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("Compile repository knowledge"))
        .stdout(predicate::str::contains("--version"));

    Ok(())
}

#[test]
fn version_output_uses_package_version() -> Result<(), Box<dyn Error>> {
    let mut command = Command::cargo_bin("lithograph")?;

    command
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains(env!("CARGO_PKG_VERSION")));

    Ok(())
}
