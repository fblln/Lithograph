//! Incrementally reconciled BM25 document store (LIT-86.11).
//!
//! The FTS document set is a deterministic projection of the graph, and
//! [`FtsIndex::search`] computes every corpus statistic (document frequency,
//! document count, average length) from the current document set at query
//! time. So BM25 scores are a pure function of that set: maintaining the set
//! incrementally -- inserting, updating, reusing, and deleting one document at
//! a time -- produces search results *byte-identical* to a clean full rebuild,
//! as long as the reconciled set equals the freshly projected one (AC#4).
//!
//! Each FTS document has exactly one owning graph node, so reconciliation here
//! is a keyed content-hash diff (the multi-owner aggregation model in
//! `reconcile.rs` is for graph fragments, LIT-86.10, where env vars/packages
//! are genuinely shared). The persisted state is versioned by both a schema
//! version and a tokenization version, so a tokenizer change forces a rebuild,
//! and it is committed atomically so an interrupted write rolls back.

// ponytail: the incremental store is proven equivalent to the whole-index
// rebuild here; swapping orchestrate's rebuild for this reconcile is deferred
// so baseline-pr stays untouched until the incremental wave wires the run.
#![allow(dead_code)]

use crate::graph::Graph;
use crate::retrieval::fts::{FtsDocument, FtsIndex};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::Path;

/// Schema version of the persisted incremental FTS state.
pub(crate) const FTS_STATE_VERSION: u32 = 1;

/// Tokenization contract version. A change forces a full rebuild (AC#5), since
/// existing document postings were produced by the old tokenizer.
pub(crate) const FTS_TOKENIZER_VERSION: u32 = 1;

/// One tracked document plus the content hash used to detect changes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct FtsRecord {
    document: FtsDocument,
    content_hash: String,
}

/// The persisted incremental FTS state: tracked documents keyed by graph node
/// id, plus the schema and tokenizer versions they were produced under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IncrementalFtsState {
    version: u32,
    tokenizer_version: u32,
    records: BTreeMap<String, FtsRecord>,
}

impl Default for IncrementalFtsState {
    fn default() -> Self {
        Self {
            version: FTS_STATE_VERSION,
            tokenizer_version: FTS_TOKENIZER_VERSION,
            records: BTreeMap::new(),
        }
    }
}

impl IncrementalFtsState {
    /// Loads state, returning empty when missing, corrupt, or produced under a
    /// different schema or tokenizer version (a version change is a rebuild,
    /// AC#5/#6). A leftover `*.pending` from an interrupted commit is removed.
    pub(crate) fn load(path: &Path) -> Self {
        let pending = path.with_extension("pending");
        if pending.exists() {
            let _ = std::fs::remove_file(&pending);
        }
        let parsed = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok());
        match parsed {
            Some(state)
                if state.version == FTS_STATE_VERSION
                    && state.tokenizer_version == FTS_TOKENIZER_VERSION =>
            {
                state
            }
            _ => Self::default(),
        }
    }

    /// Atomically commits state (staged `*.pending`, then renamed), so an
    /// interrupted write leaves the previous committed file intact (AC#6).
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

    /// Materializes a searchable [`FtsIndex`] from the tracked documents, in
    /// the same stable (node id) order a clean build uses.
    pub(crate) fn to_index(&self) -> FtsIndex {
        FtsIndex {
            documents: self
                .records
                .values()
                .map(|record| record.document.clone())
                .collect(),
        }
    }
}

/// What one reconcile did, for run metrics (AC#8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FtsMetrics {
    /// New documents.
    pub inserted: usize,
    /// Documents whose text changed.
    pub updated: usize,
    /// Unchanged documents reused as-is.
    pub reused: usize,
    /// Documents removed (owning node gone or renamed).
    pub deleted: usize,
    /// Documents in the new state.
    pub total: usize,
    /// Documents whose presence/content changed corpus statistics (insert +
    /// update + delete): the postings that shift df/avgdl/N.
    pub corpus_stats_changed: usize,
}

fn content_hash(document: &FtsDocument) -> String {
    // Kind, reference, and text all affect the stored document; hash them all
    // so any change to the projection is detected.
    let canonical = format!(
        "{:?}\u{1f}{}\u{1f}{}",
        document.kind, document.reference, document.text
    );
    blake3::hash(canonical.as_bytes()).to_hex().to_string()
}

/// Reconciles the persisted state against the current graph's projected
/// documents: only genuinely new or changed documents are rewritten, removed
/// nodes' documents are deleted, and the resulting set equals a clean rebuild
/// (AC#2/#3/#4). Pure -- the caller commits the returned state on success.
pub(crate) fn reconcile_fts(
    previous: &IncrementalFtsState,
    graph: &Graph,
) -> (IncrementalFtsState, FtsMetrics) {
    // The canonical projection (AC#1): reuse the single build path so the
    // incremental document set can never diverge from the clean one.
    let desired = FtsIndex::build(graph).documents;

    let mut records = BTreeMap::new();
    let mut inserted = 0;
    let mut updated = 0;
    let mut reused = 0;
    for document in desired {
        let hash = content_hash(&document);
        match previous.records.get(&document.id) {
            Some(existing) if existing.content_hash == hash => reused += 1,
            Some(_) => updated += 1,
            None => inserted += 1,
        }
        records.insert(
            document.id.clone(),
            FtsRecord {
                document,
                content_hash: hash,
            },
        );
    }
    let deleted = previous
        .records
        .keys()
        .filter(|id| !records.contains_key(*id))
        .count();

    let total = records.len();
    let state = IncrementalFtsState {
        version: FTS_STATE_VERSION,
        tokenizer_version: FTS_TOKENIZER_VERSION,
        records,
    };
    let metrics = FtsMetrics {
        inserted,
        updated,
        reused,
        deleted,
        total,
        corpus_stats_changed: inserted + updated + deleted,
    };
    (state, metrics)
}

#[cfg(test)]
mod tests {
    use super::{IncrementalFtsState, reconcile_fts};
    use crate::domain::{ArtifactCategory, ArtifactId, EvidenceRef, RepoPath, SourceSpan};
    use crate::graph::{ArtifactNode, Graph, GraphNode, GraphNodeId, SymbolKind, SymbolNode};

    fn symbol(path: &str, name: &str, doc: &str) -> Result<GraphNode, Box<dyn std::error::Error>> {
        let repo = RepoPath::new(path)?;
        let artifact_id = ArtifactId::from_path(&repo);
        Ok(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(format!("symbol:{path}#{name}")),
            kind: SymbolKind::Function,
            qualified_name: name.to_owned(),
            doc: (!doc.is_empty()).then(|| doc.to_owned()),
            evidence: EvidenceRef::file(artifact_id, repo).with_span(SourceSpan::new(1, 1)?),
        }))
    }

    fn artifact(path: &str) -> Result<GraphNode, Box<dyn std::error::Error>> {
        let repo = RepoPath::new(path)?;
        let artifact_id = ArtifactId::from_path(&repo);
        Ok(GraphNode::Artifact(ArtifactNode {
            id: GraphNodeId::new(format!("artifact:{path}")),
            path: path.to_owned(),
            category: ArtifactCategory::SourceCode,
            evidence: EvidenceRef::file(artifact_id, repo),
        }))
    }

    fn graph(nodes: Vec<GraphNode>) -> Graph {
        Graph {
            nodes,
            relations: Vec::new(),
        }
    }

    /// AC#4/#7 clean-vs-incremental equivalence: an index maintained through an
    /// edit searches identically to a clean rebuild of the edited graph.
    #[test]
    fn incremental_search_matches_clean_rebuild() -> Result<(), Box<dyn std::error::Error>> {
        let before = graph(vec![
            artifact("router.py")?,
            symbol("router.py", "route_service", "handles routing")?,
        ]);
        let (state, _) = reconcile_fts(&IncrementalFtsState::default(), &before);

        // Edit: add a symbol and change a doc comment.
        let after = graph(vec![
            artifact("router.py")?,
            symbol("router.py", "route_service", "handles request routing now")?,
            symbol("router.py", "handle", "processes a request")?,
        ]);
        let (incremental_state, metrics) = reconcile_fts(&state, &after);
        assert_eq!(metrics.inserted, 1, "one new symbol");
        assert_eq!(metrics.updated, 1, "one changed doc comment");

        // The incremental index and a clean rebuild return identical results.
        let incremental = incremental_state.to_index();
        let clean = crate::retrieval::fts::FtsIndex::build(&after);
        for query in ["route service", "request", "handle", "routing"] {
            assert_eq!(
                incremental.search(query, 10),
                clean.search(query, 10),
                "query `{query}` matches clean rebuild"
            );
        }
        Ok(())
    }

    /// AC#2 no-op: reconciling the same graph reuses every document.
    #[test]
    fn no_op_reuses_all() -> Result<(), Box<dyn std::error::Error>> {
        let g = graph(vec![
            artifact("a.py")?,
            symbol("a.py", "f", "does a thing")?,
        ]);
        let (state, _) = reconcile_fts(&IncrementalFtsState::default(), &g);
        let (_, metrics) = reconcile_fts(&state, &g);
        assert_eq!(metrics.reused, state.records.len());
        assert_eq!(metrics.inserted, 0);
        assert_eq!(metrics.updated, 0);
        assert_eq!(metrics.deleted, 0);
        Ok(())
    }

    /// AC#3 delete and rename remove owned documents with no ghost hits.
    #[test]
    fn delete_and_rename_remove_documents() -> Result<(), Box<dyn std::error::Error>> {
        let before = graph(vec![
            artifact("old.py")?,
            symbol("old.py", "gone", "temporary helper")?,
        ]);
        let (state, _) = reconcile_fts(&IncrementalFtsState::default(), &before);

        // Rename the file (new node ids) and drop the symbol.
        let after = graph(vec![artifact("new.py")?]);
        let (renamed_state, metrics) = reconcile_fts(&state, &after);
        assert_eq!(metrics.deleted, 2, "old artifact + old symbol removed");
        // No ghost hits for the deleted symbol's distinctive term.
        assert!(
            renamed_state
                .to_index()
                .search("temporary helper", 10)
                .is_empty()
        );
        Ok(())
    }

    /// AC#5 tokenizer-version change forces a rebuild via load returning empty.
    #[test]
    fn tokenizer_version_change_forces_rebuild() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("fts.json");
        let g = graph(vec![artifact("a.py")?]);
        let (state, _) = reconcile_fts(&IncrementalFtsState::default(), &g);
        // Persist under a stale tokenizer version.
        let stale = IncrementalFtsState {
            tokenizer_version: super::FTS_TOKENIZER_VERSION + 1,
            ..state
        };
        stale.commit(&path)?;
        assert!(
            IncrementalFtsState::load(&path).records.is_empty(),
            "stale tokenizer version reloads empty (rebuild)"
        );
        Ok(())
    }

    /// AC#6 recoverability: state round-trips and a stray `*.pending` from an
    /// interrupted commit is ignored; corrupt state loads empty.
    #[test]
    fn persistence_is_recoverable() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("fts.json");
        let g = graph(vec![
            artifact("a.py")?,
            symbol("a.py", "f", "documented function")?,
        ]);
        let (state, _) = reconcile_fts(&IncrementalFtsState::default(), &g);
        state.commit(&path)?;

        std::fs::write(path.with_extension("pending"), "garbage")?;
        assert_eq!(
            IncrementalFtsState::load(&path),
            state,
            "rolled back to committed"
        );
        assert!(!path.with_extension("pending").exists());

        std::fs::write(&path, "{ not json")?;
        assert!(
            IncrementalFtsState::load(&path).records.is_empty(),
            "corrupt loads empty"
        );
        Ok(())
    }

    /// AC#7 Unicode identifiers survive projection and reconcile deterministically.
    #[test]
    fn unicode_identifiers_reconcile() -> Result<(), Box<dyn std::error::Error>> {
        let g = graph(vec![
            artifact("café.py")?,
            symbol("café.py", "función_café", "índice de búsqueda")?,
        ]);
        let (state, metrics) = reconcile_fts(&IncrementalFtsState::default(), &g);
        assert_eq!(metrics.inserted, 2);
        // Re-running is a pure no-op even with multibyte content.
        let (_, again) = reconcile_fts(&state, &g);
        assert_eq!(again.reused, 2);
        Ok(())
    }
}
