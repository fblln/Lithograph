//! Graph-bound derived attachments (LIT-86.16).
//!
//! An *attachment* is a derived sidecar bound to a parent target and the
//! canonical graph it was computed from: the FTS index, the vector index, the
//! graph report, research summaries, the layout, the generation manifest, and
//! the structural index are all attachments. Each [`AttachmentRecord`] records
//! its owner component, parent target, source graph hash, schema version, and
//! input/logic fingerprints, plus the teardown key needed to remove it. An
//! attachment answers queries only while its source graph hash and schema match
//! the active canonical graph; a stale sidecar is rejected and rebuilt, never
//! silently served, and publishing a new canonical graph stays atomic (an
//! attachment can never promote a partially-resolved graph, because it is only
//! active against a hash the validated graph actually has).

// ponytail: this generalizes the code-search sidecar (LIT-86.6) and the FTS
// store (LIT-86.11) into one typed attachment model; migrating each existing
// sidecar onto it is staged across follow-ons (AC#3). Drop this allow as they
// migrate.
#![allow(dead_code)]

use crate::reconcile::ComponentPath;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Schema version of the attachment tracking state.
pub(crate) const ATTACHMENT_SCHEMA_VERSION: u32 = 1;

/// The typed kinds of graph-bound attachment (AC#3). Each has documented
/// ownership and transaction boundaries even if its migration onto this model
/// is staged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum AttachmentKind {
    /// BM25 full-text index (owner: the FTS projection; committed atomically).
    Fts,
    /// Raw-code vector index (owner: the embedding pipeline; provider-tagged).
    VectorIndex,
    /// Evidence-linked architecture/operations report.
    GraphReport,
    /// Research summaries and lessons.
    Research,
    /// Graph layout.
    Layout,
    /// Documentation generation manifest.
    GenerationManifest,
    /// AST structural index (LIT-86.12, if built).
    StructuralIndex,
}

/// One attachment instance and every fact needed to validate, reconcile, and
/// tear it down (AC#2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AttachmentRecord {
    /// Kind of attachment.
    pub kind: AttachmentKind,
    /// Owning component.
    pub owner: ComponentPath,
    /// Parent target key this attachment hangs off.
    pub parent_target: String,
    /// Typed namespace within the parent (AC#1).
    pub namespace: String,
    /// Stable instance id within the namespace.
    pub instance: String,
    /// Hash of the canonical graph (or manifest) this attachment was built from.
    pub source_graph_hash: String,
    /// Attachment payload schema version.
    pub schema_version: u32,
    /// Canonical input fingerprint.
    pub input_fingerprint: String,
    /// Logic fingerprint of the pass that produced it.
    pub logic_fingerprint: String,
    /// Key/path to remove when tearing this attachment down (AC#2).
    pub teardown_key: String,
}

impl AttachmentRecord {
    /// The namespaced record key, prefixed so it can never collide with an
    /// ordinary child record key (AC#1).
    pub(crate) fn key(&self) -> String {
        format!(
            "@{}/{}/{}",
            self.namespace, self.parent_target, self.instance
        )
    }

    /// Whether this attachment is compatible with the active canonical graph
    /// (AC#4): same source graph hash and schema version. An incompatible
    /// attachment must be rejected/rebuilt, never served.
    pub(crate) fn is_compatible(&self, active_graph_hash: &str, active_schema: u32) -> bool {
        self.source_graph_hash == active_graph_hash && self.schema_version == active_schema
    }
}

/// The persisted attachment tracking state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AttachmentState {
    version: u32,
    records: BTreeMap<String, AttachmentRecord>,
}

impl Default for AttachmentState {
    fn default() -> Self {
        Self {
            version: ATTACHMENT_SCHEMA_VERSION,
            records: BTreeMap::new(),
        }
    }
}

impl AttachmentState {
    /// Loads state, returning empty when missing, corrupt, or a different schema
    /// version. A leftover `*.pending` from an interrupted commit is removed
    /// (AC#7 recoverable, AC#10 corrupt/missing).
    pub(crate) fn load(path: &Path) -> Self {
        let pending = path.with_extension("pending");
        if pending.exists() {
            let _ = std::fs::remove_file(&pending);
        }
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok())
            .filter(|state| state.version == ATTACHMENT_SCHEMA_VERSION)
            .unwrap_or_default()
    }

    /// Atomically commits state (staged `*.pending`, then renamed) so an
    /// interrupted commit leaves the previous committed file intact (AC#7).
    pub(crate) fn commit(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string(self).map_err(io::Error::other)?;
        json.push('\n');
        let pending = path.with_extension("pending");
        std::fs::write(&pending, &json)?;
        std::fs::rename(&pending, path)
    }

    /// The compatible attachments that may answer queries against the active
    /// graph (AC#4): stale sidecars are excluded.
    pub(crate) fn active(
        &self,
        active_graph_hash: &str,
        active_schema: u32,
    ) -> Vec<&AttachmentRecord> {
        self.records
            .values()
            .filter(|record| record.is_compatible(active_graph_hash, active_schema))
            .collect()
    }
}

/// What one reconcile did (AC#9).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct AttachmentMetrics {
    /// Attachments reused unchanged (no physical rewrite).
    pub kept: usize,
    /// Attachments rebuilt because their fingerprint changed.
    pub rebuilt: usize,
    /// Attachments rejected as incompatible with the active graph.
    pub rejected: usize,
    /// Attachments removed because their owner/parent disappeared.
    pub removed: usize,
}

/// Reconciles `previous` attachment state against the `desired` set for the
/// active graph. An unchanged attachment (same key + fingerprints + graph hash)
/// is kept with no rewrite (AC#8); a changed one is rebuilt; a previously
/// tracked attachment absent from `desired` is removed (orphan cleanup, AC#5);
/// a desired attachment whose source hash does not match the active graph is
/// rejected (AC#4). Pure -- the caller commits on success (AC#7).
pub(crate) fn reconcile_attachments(
    previous: &AttachmentState,
    desired: &[AttachmentRecord],
    active_graph_hash: &str,
    active_schema: u32,
) -> (AttachmentState, AttachmentMetrics) {
    let mut records = BTreeMap::new();
    let mut metrics = AttachmentMetrics::default();

    for attachment in desired {
        // Reject a desired attachment that is not bound to the active graph:
        // it must be (re)built against the current graph before it can serve.
        if !attachment.is_compatible(active_graph_hash, active_schema) {
            metrics.rejected += 1;
            continue;
        }
        let key = attachment.key();
        match previous.records.get(&key) {
            Some(existing)
                if existing.input_fingerprint == attachment.input_fingerprint
                    && existing.logic_fingerprint == attachment.logic_fingerprint
                    && existing.source_graph_hash == attachment.source_graph_hash =>
            {
                metrics.kept += 1;
            }
            _ => metrics.rebuilt += 1,
        }
        records.insert(key, attachment.clone());
    }

    metrics.removed = previous
        .records
        .keys()
        .filter(|key| !records.contains_key(*key))
        .count();

    let state = AttachmentState {
        version: ATTACHMENT_SCHEMA_VERSION,
        records,
    };
    (state, metrics)
}

#[cfg(test)]
mod tests {
    use super::{AttachmentKind, AttachmentRecord, AttachmentState, reconcile_attachments};
    use crate::reconcile::ComponentPath;

    fn record(instance: &str, graph_hash: &str, input_fp: &str) -> AttachmentRecord {
        AttachmentRecord {
            kind: AttachmentKind::VectorIndex,
            owner: ComponentPath::new(["search-index"]),
            parent_target: "graph".to_owned(),
            namespace: "vector".to_owned(),
            instance: instance.to_owned(),
            source_graph_hash: graph_hash.to_owned(),
            schema_version: 1,
            input_fingerprint: input_fp.to_owned(),
            logic_fingerprint: "logic-1".to_owned(),
            teardown_key: format!(".lithograph/derived/{instance}.json"),
        }
    }

    /// AC#1: attachment keys are namespaced and never collide with an ordinary
    /// child record key.
    #[test]
    fn attachment_key_is_namespaced() {
        let key = record("main", "g1", "i1").key();
        assert!(key.starts_with('@'));
        assert_ne!(key, "graph");
    }

    /// AC#4: an attachment bound to a stale graph hash is not active.
    #[test]
    fn stale_graph_hash_is_not_active() {
        let mut state = AttachmentState::default();
        let attachment = record("main", "old-graph", "i1");
        state.records.insert(attachment.key(), attachment);
        assert!(state.active("old-graph", 1).len() == 1);
        assert!(
            state.active("new-graph", 1).is_empty(),
            "stale sidecar never serves"
        );
    }

    /// AC#8 no-op: an unchanged attachment is kept with no rebuild.
    #[test]
    fn no_op_keeps_attachment() {
        let attachment = record("main", "g1", "i1");
        let (state, _) = reconcile_attachments(
            &AttachmentState::default(),
            std::slice::from_ref(&attachment),
            "g1",
            1,
        );
        let (_, metrics) =
            reconcile_attachments(&state, std::slice::from_ref(&attachment), "g1", 1);
        assert_eq!(metrics.kept, 1);
        assert_eq!(metrics.rebuilt, 0);
        assert_eq!(metrics.removed, 0);
    }

    /// AC#8 localized change: a changed input fingerprint rebuilds only that
    /// attachment.
    #[test]
    fn changed_fingerprint_rebuilds() {
        let before = record("main", "g1", "i1");
        let (state, _) = reconcile_attachments(
            &AttachmentState::default(),
            std::slice::from_ref(&before),
            "g1",
            1,
        );
        let after = record("main", "g1", "i2");
        let (_, metrics) = reconcile_attachments(&state, std::slice::from_ref(&after), "g1", 1);
        assert_eq!(metrics.rebuilt, 1);
        assert_eq!(metrics.kept, 0);
    }

    /// AC#4: a desired attachment bound to a different graph than the active one
    /// is rejected, not stored.
    #[test]
    fn incompatible_desired_attachment_is_rejected() {
        let attachment = record("main", "some-other-graph", "i1");
        let (state, metrics) = reconcile_attachments(
            &AttachmentState::default(),
            std::slice::from_ref(&attachment),
            "active-graph",
            1,
        );
        assert_eq!(metrics.rejected, 1);
        assert!(state.records.is_empty());
    }

    /// AC#5 orphan cleanup: an attachment absent from the desired set (parent
    /// or owner removed) is removed.
    #[test]
    fn orphaned_attachment_is_removed() {
        let attachment = record("main", "g1", "i1");
        let (state, _) = reconcile_attachments(
            &AttachmentState::default(),
            std::slice::from_ref(&attachment),
            "g1",
            1,
        );
        let (next, metrics) = reconcile_attachments(&state, &[], "g1", 1);
        assert_eq!(metrics.removed, 1);
        assert!(next.records.is_empty());
    }

    /// AC#7/#10: persistence round-trips, a stray `*.pending` from an
    /// interrupted commit is ignored, and corrupt state loads empty.
    #[test]
    fn persistence_is_recoverable() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("attachments.json");
        let attachment = record("main", "g1", "i1");
        let (state, _) = reconcile_attachments(
            &AttachmentState::default(),
            std::slice::from_ref(&attachment),
            "g1",
            1,
        );
        state.commit(&path)?;

        std::fs::write(path.with_extension("pending"), "garbage")?;
        assert_eq!(AttachmentState::load(&path), state);
        assert!(!path.with_extension("pending").exists());

        std::fs::write(&path, "{ not json")?;
        assert!(AttachmentState::load(&path).records.is_empty());
        Ok(())
    }
}
