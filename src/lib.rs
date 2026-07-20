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

pub(crate) mod agent;
pub(crate) mod analysis;
pub(crate) mod attachment;
pub(crate) mod cli;
pub(crate) mod commands;
pub(crate) mod docs;
pub(crate) mod explain;
pub(crate) mod fingerprint;
pub(crate) mod graph;
pub(crate) mod knowledge;
pub(crate) mod plan;
pub(crate) mod reconcile;
pub(crate) mod resolve;
pub(crate) mod retrieval;
pub(crate) mod run;
pub(crate) mod serve;
pub(crate) mod storage;
pub(crate) mod viewer;
pub(crate) mod watch;

/// Runs the Lithograph command-line interface.
pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let command = cli::Cli::parse_args();
    let mut stdout = std::io::stdout().lock();
    commands::execute(command, &mut stdout)
}
