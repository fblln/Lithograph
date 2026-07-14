//! Offline lab execution, expectation evaluation, run storage, and replay.
//!
//! This module is a thin dispatcher: it owns the [`Lab`] handle and
//! [`LabError`], plus the handful of generic operations (loading a run,
//! explaining an assertion, listing runs) that are not specific to any one
//! lab mode. Each lab mode's orchestration lives in its own submodule and
//! contributes additional `impl Lab` blocks, which Rust merges with this
//! one since they share a crate:
//!
//! - [`correctness`]: the correctness suite (`run`/`run_case`), persistence,
//!   and known-failure suppression -- what `just baseline-pr` exercises.
//! - [`evaluation`]: expectation evaluation and metric derivation shared by
//!   the correctness suite, mutation scenarios, and minimization.
//! - [`differential`]: independent-oracle checks (Python `ast`, `cargo
//!   metadata`, the TypeScript compiler, optional SCIP) run for the PR tier.
//! - [`mutation`]: expectation-preserving repository scenarios and bounded
//!   generated-Python parser fuzzing with failure shrinking.
//! - [`acceptance`]: baseline governance -- comparing a run to its accepted
//!   baseline and the reviewed, token-bound acceptance flow.
//! - [`replay`]: deterministic replay of a prior run's exact case and suite.
//! - [`migration`]: purely mechanical lab JSON schema migration.
//! - [`minimize`]: source-free failure minimization by artifact bisection.
//! - [`performance`]: warm-sample benchmarking reduced to robust
//!   median/MAD summaries, plus the separate `community-only` replay mode.
//! - [`support`]: shared JSON I/O, content-addressing, schema-compatible
//!   reads, and process telemetry used across the modes above.

use crate::lab::corpus::{Corpus, CorpusError};
use crate::lab::model::*;
use serde_json::{Value, json};
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

mod acceptance;
mod correctness;
mod differential;
mod evaluation;
mod migration;
mod minimize;
mod mutation;
mod performance;
mod replay;
mod support;
use correctness::apply_known_failures;
use differential::{
    differential_cargo_metadata, differential_python_definitions, differential_scip,
    differential_typescript_compiler,
};
use evaluation::{
    compare_community_scopes, derive_metrics, evaluate, failed_trace_selectors, millionths,
};
use support::{
    atomic_json_write, hash_json, migrate_value, process_rss_kib, read_compatible,
    read_optional_compatible, read_required, schema_version,
};

/// Lab operation failure.
#[derive(Debug)]
pub enum LabError {
    /// Filesystem failure.
    Io(std::io::Error),
    /// JSON failure.
    Json(serde_json::Error),
    /// Corpus failure.
    Corpus(CorpusError),
    /// Repository walk failure.
    Walk(crate::inventory::WalkError),
    /// Invalid request or failed baseline check.
    Invalid(String),
}

impl Display for LabError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(error) => Display::fmt(error, formatter),
            Self::Json(error) => Display::fmt(error, formatter),
            Self::Corpus(error) => Display::fmt(error, formatter),
            Self::Walk(error) => Display::fmt(error, formatter),
            Self::Invalid(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for LabError {}

impl From<std::io::Error> for LabError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for LabError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<CorpusError> for LabError {
    fn from(value: CorpusError) -> Self {
        Self::Corpus(value)
    }
}

impl From<crate::inventory::WalkError> for LabError {
    fn from(value: crate::inventory::WalkError) -> Self {
        Self::Walk(value)
    }
}

/// Baseline lab bound to one corpus and artifact root.
#[derive(Debug, Clone)]
pub struct Lab {
    /// Loaded corpus.
    pub corpus: Corpus,
    /// Content-addressed run and committed-baseline root.
    pub root: PathBuf,
}

impl Lab {
    /// Creates a lab.
    pub fn new(corpus: Corpus, root: PathBuf) -> Self {
        Self { corpus, root }
    }

    /// Loads a run by directory or content id.
    pub fn load_run(&self, run: &Path) -> Result<RunManifest, LabError> {
        let path = if run.is_dir() {
            run.join("manifest.json")
        } else {
            self.root.join("runs").join(run).join("manifest.json")
        };
        read_compatible(&path)
    }

    /// Returns a focused explanation for one assertion.
    pub fn explain(&self, run: &RunManifest, id: &str) -> Result<Value, LabError> {
        let assertion = run
            .assertions
            .iter()
            .find(|result| result.id == id)
            .ok_or_else(|| LabError::Invalid(format!("run has no assertion `{id}`")))?;
        Ok(json!({
            "run_id": run.run_id,
            "case_id": run.case_id,
            "assertion": assertion,
            "replay": run.reproduce,
            "inspect_stage": format!("cargo run --bin lithograph-lab -- inspect {} --stage {}", run.run_id, assertion.stage),
        }))
    }

    /// Lists persisted run ids.
    pub fn list_runs(&self) -> Result<Vec<String>, LabError> {
        let root = self.root.join("runs");
        if !root.is_dir() {
            return Ok(Vec::new());
        }
        let mut runs = std::fs::read_dir(root)?
            .filter_map(Result::ok)
            .filter(|entry| entry.path().join("manifest.json").is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .collect::<Vec<_>>();
        runs.sort();
        Ok(runs)
    }
}
