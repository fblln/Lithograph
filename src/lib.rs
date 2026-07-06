//! Lithograph CLI library surface.

pub mod agents;
pub mod analysis;
pub mod cli;
pub mod commands;
pub mod domain;
pub mod drift;
pub mod generation;
pub mod graph;
pub mod inventory;
pub mod manifest;
pub mod orchestrate;
pub mod plan;
pub mod run;
pub mod storage;

/// Runs the Lithograph command-line interface.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let command = cli::Cli::parse_args();
    let mut stdout = std::io::stdout().lock();
    commands::execute(command, &mut stdout)
}
