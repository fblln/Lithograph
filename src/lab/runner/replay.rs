//! Deterministic replay: re-executes the exact case and suite a prior run
//! recorded, refusing when the graph pipeline version or the corpus-pinned
//! source revision has moved on since.

use super::{Lab, LabError};
use crate::lab::model::{CorpusSource, RunManifest};
use std::path::PathBuf;

impl Lab {
    /// Replays the exact case and suite identified by a prior run.
    pub fn replay(&self, run: &RunManifest) -> Result<PathBuf, LabError> {
        if run.graph_pipeline_version != crate::graph::GRAPH_BUILD_PIPELINE_VERSION {
            return Err(LabError::Invalid(format!(
                "run graph pipeline version {} is incompatible with current version {}; migrate mechanical schema fields or regenerate and semantically review the baseline",
                run.graph_pipeline_version,
                crate::graph::GRAPH_BUILD_PIPELINE_VERSION
            )));
        }
        let case = self.corpus.case(&run.case_id)?.clone();
        let current_revision = match &case.source {
            CorpusSource::Fixture { .. } => None,
            CorpusSource::Git { commit, .. } => Some(commit.as_str()),
        };
        if current_revision.is_some_and(|revision| revision != run.source_revision) {
            return Err(LabError::Invalid(format!(
                "replay source changed: run used {}, corpus pins {}",
                run.source_revision,
                current_revision.unwrap_or_default()
            )));
        }
        self.run_case(&case, run.suite)
    }
}
