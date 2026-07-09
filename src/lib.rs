//! Lithograph CLI library surface.

pub mod adr;
pub mod agents;
pub mod analysis;
pub mod architecture;
pub mod ask;
pub mod cli;
pub mod commands;
pub mod domain;
pub mod drift;
pub mod editor_agent;
pub mod generation;
pub mod golden;
pub mod graph;
pub mod inventory;
pub mod knowledge_agent;
pub mod manifest;
pub mod mcp;
pub mod mermaid;
pub mod orchestrate;
pub mod plan;
pub mod quality;
pub mod research;
pub mod resolve;
pub mod run;
pub mod storage;
pub mod viewer;

/// Runs the Lithograph command-line interface.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let command = cli::Cli::parse_args();
    let mut stdout = std::io::stdout().lock();
    commands::execute(command, &mut stdout)
}
