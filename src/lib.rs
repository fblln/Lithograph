//! Lithograph CLI library surface.

// Public crate API: only modules consumed by the binaries, integration tests,
// and examples stay `pub`. Everything else is `pub(crate)` so each module's
// internals stop being part of the crate's external interface (LIT-85.1).
pub mod domain;
pub mod generation;
pub mod golden;
pub mod inventory;
pub mod lab;
pub mod manifest;
pub mod orchestrate;

pub(crate) mod adr;
pub(crate) mod agents;
pub(crate) mod analysis;
pub(crate) mod architecture;
pub(crate) mod ask;
pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod docs_model;
pub(crate) mod documentation_claims;
pub(crate) mod drift;
pub(crate) mod fts;
pub(crate) mod graph;
pub(crate) mod graph_docs;
pub(crate) mod knowledge_agent;
pub(crate) mod mcp;
pub(crate) mod mcp_targets;
pub(crate) mod mermaid;
pub(crate) mod plan;
pub(crate) mod quality;
pub(crate) mod query;
pub(crate) mod research;
pub(crate) mod research_feedback;
pub(crate) mod resolve;
pub(crate) mod run;
pub(crate) mod search;
pub(crate) mod semantic_search;
pub(crate) mod serve;
pub(crate) mod storage;
pub(crate) mod subsystem_docs;
pub(crate) mod viewer;
pub(crate) mod watch;

/// Runs the Lithograph command-line interface.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let command = cli::Cli::parse_args();
    let mut stdout = std::io::stdout().lock();
    commands::execute(command, &mut stdout)
}
