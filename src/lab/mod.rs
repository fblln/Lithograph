//! Deterministic baseline, corpus, trace, and replay tooling.

pub mod corpus;
pub mod metrics;
pub mod model;
pub mod runner;

pub use corpus::{Corpus, CorpusError};
pub use model::*;
pub use runner::{Lab, LabError};
