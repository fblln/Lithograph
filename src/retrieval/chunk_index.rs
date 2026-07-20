//! Persistent, incrementally reconciled raw-code vector index (LIT-86.3).
//!
//! Chunk embeddings live in one JSON file under `.lithograph/` instead of being
//! rebuilt on every search. An `update` reconciles the persisted index against
//! the current chunk set: unchanged chunk identities reuse their vector,
//! identical content anywhere reuses one vector (dedup), only genuinely new or
//! changed content is embedded, and chunk ids that no longer exist are dropped.
//!
//! Reconciliation is owned entirely by this component (the LIT-86.1 ownership
//! contract): the desired keyset is recomputed from the current chunks, and any
//! persisted record absent from it is deleted. Writes are staged to a temp file
//! and atomically renamed, so an interrupted update leaves the previous index
//! fully intact -- never a half-written or mixed-dimension file.

// ponytail: consumed by the search surface (LIT-86.6) and graph-constrained
// ranking (LIT-86.5). Drop this allow at first production wiring.
#![allow(dead_code)]

use crate::retrieval::semantic_search::{EmbeddingError, EmbeddingProvider};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::io;
use std::path::Path;

/// On-disk schema version. A bump forces a full rebuild (AC#4): old files are
/// treated as incompatible and every vector is recomputed.
pub(crate) const CHUNK_INDEX_SCHEMA_VERSION: u32 = 1;

/// Everything about the embedding pipeline whose change must invalidate cached
/// vectors (AC#4): provider/model, dimension, normalization, and the document
/// vs query embedding purposes (LIT-86.15). Folded into one digest that is
/// stored with the index and compared on every reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ProviderIdentity {
    /// Provider/model identity string, e.g. `MockEmbeddingProvider`'s tag.
    pub model: String,
    /// Vector dimensionality.
    pub dimensions: usize,
    /// Whether vectors are L2-normalized.
    pub normalized: bool,
    /// Document embedding purpose/prompt identity.
    pub document_prompt: String,
    /// Query embedding purpose/prompt identity.
    pub query_prompt: String,
}

impl ProviderIdentity {
    /// Stable digest over every invalidating field; any change yields a new
    /// digest and therefore a full rebuild.
    pub(crate) fn digest(&self) -> String {
        let joined = format!(
            "{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
            self.model, self.dimensions, self.normalized, self.document_prompt, self.query_prompt
        );
        blake3::hash(joined.as_bytes()).to_hex().to_string()
    }
}

/// One persisted chunk embedding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ChunkVectorRecord {
    /// Stable chunk identity (LIT-86.1); the reconciliation key.
    pub chunk_id: String,
    /// `blake3` of the chunk bytes; the embedding-reuse key (identical content
    /// shares one vector even across different chunk ids).
    pub content_hash: String,
    // ponytail: vectors are stored as a plain JSON f32 array -- serde+ryu is an
    // exact, deterministic round-trip and needs no codec. Pack to bytes only if
    // a real repo's index size or parse time is measured to matter.
    /// Embedding vector.
    pub vector: Vec<f32>,
}

/// The persisted index: schema/provider identity plus the vector records,
/// sorted by chunk id for a deterministic file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct ChunkIndex {
    /// On-disk schema version.
    pub schema_version: u32,
    /// Digest of the provider identity that produced these vectors.
    pub provider_digest: String,
    /// Vector dimensionality (guards against mixed-dimension reuse).
    pub dimensions: usize,
    /// Records, sorted by `chunk_id`.
    pub records: Vec<ChunkVectorRecord>,
}

impl ChunkIndex {
    /// An empty index for `identity` (used as the "previous" state on a cold
    /// build or after a corrupt-file reset).
    pub(crate) fn empty(identity: &ProviderIdentity) -> Self {
        Self {
            schema_version: CHUNK_INDEX_SCHEMA_VERSION,
            provider_digest: identity.digest(),
            dimensions: identity.dimensions,
            records: Vec::new(),
        }
    }

    /// Loads the index from `path`, returning an empty index for `identity`
    /// when the file is missing or corrupt (miss-is-safe, AC#8): a bad index is
    /// never a correctness hazard, only a rebuild.
    pub(crate) fn load(path: &Path, identity: &ProviderIdentity) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|_| Self::empty(identity)),
            Err(_) => Self::empty(identity),
        }
    }

    /// Serializes and atomically writes the index to `path`: staged to a
    /// sibling `*.tmp` then renamed, so a crash leaves either the old file or
    /// the new file, never a partial one (AC#5). Returns the byte size written.
    pub(crate) fn save(&self, path: &Path) -> io::Result<usize> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string(self).map_err(io::Error::other)?;
        json.push('\n');
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, path)?;
        Ok(json.len())
    }

    /// Cosine-ranks the persisted vectors against `query_vector` without
    /// re-embedding any document (AC#1); returns the top `limit` by score,
    /// breaking ties by chunk id for determinism.
    pub(crate) fn search(&self, query_vector: &[f32], limit: usize) -> Vec<ScoredChunk> {
        let mut scored: Vec<ScoredChunk> = self
            .records
            .iter()
            .map(|record| ScoredChunk {
                chunk_id: record.chunk_id.clone(),
                score: cosine(query_vector, &record.vector),
            })
            .collect();
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.chunk_id.cmp(&b.chunk_id))
        });
        scored.truncate(limit);
        scored
    }
}

/// One search result: a chunk id and its cosine score.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ScoredChunk {
    /// The chunk's stable identity.
    pub chunk_id: String,
    /// Cosine similarity to the query.
    pub score: f64,
}

/// The current chunk set to reconcile the index against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DesiredChunk {
    /// Stable chunk identity.
    pub chunk_id: String,
    /// `blake3` of the chunk bytes.
    pub content_hash: String,
    /// Chunk text to embed on a miss.
    pub text: String,
}

/// What one reconcile did, for status and run metrics (AC#7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReconcileMetrics {
    /// Distinct chunk ids in the new index.
    pub chunk_count: usize,
    /// Vector records in the new index (equals `chunk_count`).
    pub vector_count: usize,
    /// Vectors reused from the previous index (unchanged or dedup).
    pub reused_vectors: usize,
    /// Vectors freshly embedded this reconcile.
    pub recomputed_vectors: usize,
    /// Records dropped because their chunk id no longer exists (or a full
    /// rebuild discarded the whole previous index).
    pub deleted_rows: usize,
    /// Whether the previous index was incompatible and fully rebuilt.
    pub rebuilt: bool,
    /// Provider digest of the new index.
    pub provider_digest: String,
    /// Schema version of the new index.
    pub schema_version: u32,
}

/// Reconciles `previous` against `desired`, embedding only new or changed
/// content. A schema or provider-identity mismatch forces a full rebuild
/// (AC#4). Returns the new index and the metrics describing the work done.
pub(crate) fn reconcile(
    previous: &ChunkIndex,
    desired: &[DesiredChunk],
    provider: &dyn EmbeddingProvider,
    identity: &ProviderIdentity,
) -> Result<(ChunkIndex, ReconcileMetrics), EmbeddingError> {
    let current_digest = identity.digest();
    let compatible = previous.schema_version == CHUNK_INDEX_SCHEMA_VERSION
        && previous.provider_digest == current_digest;

    // One content-hash -> vector map drives all reuse. Seeded from the previous
    // index when compatible (unchanged chunks and cross-id dedup) and extended
    // as vectors are computed this pass, so duplicate content embeds exactly
    // once even on a cold build. A vector is a pure function of its content
    // hash, so reuse is always correct regardless of which chunk id produced it.
    let mut computed: HashMap<String, Vec<f32>> = HashMap::new();
    if compatible {
        for record in &previous.records {
            computed
                .entry(record.content_hash.clone())
                .or_insert_with(|| record.vector.clone());
        }
    }

    let mut records = Vec::with_capacity(desired.len());
    let mut reused = 0usize;
    let mut recomputed = 0usize;
    for chunk in desired {
        let vector = match computed.get(&chunk.content_hash) {
            Some(vector) => {
                reused += 1;
                vector.clone()
            }
            None => {
                recomputed += 1;
                let vector = provider.embed(&chunk.text)?;
                computed.insert(chunk.content_hash.clone(), vector.clone());
                vector
            }
        };
        records.push(ChunkVectorRecord {
            chunk_id: chunk.chunk_id.clone(),
            content_hash: chunk.content_hash.clone(),
            vector,
        });
    }
    records.sort_by(|a, b| a.chunk_id.cmp(&b.chunk_id));

    let desired_ids: BTreeSet<&str> = desired.iter().map(|c| c.chunk_id.as_str()).collect();
    let deleted_rows = if compatible {
        previous
            .records
            .iter()
            .filter(|record| !desired_ids.contains(record.chunk_id.as_str()))
            .count()
    } else {
        previous.records.len()
    };

    let index = ChunkIndex {
        schema_version: CHUNK_INDEX_SCHEMA_VERSION,
        provider_digest: current_digest.clone(),
        dimensions: identity.dimensions,
        records,
    };
    let metrics = ReconcileMetrics {
        chunk_count: index.records.len(),
        vector_count: index.records.len(),
        reused_vectors: reused,
        recomputed_vectors: recomputed,
        deleted_rows,
        rebuilt: !compatible,
        provider_digest: current_digest,
        schema_version: CHUNK_INDEX_SCHEMA_VERSION,
    };
    Ok((index, metrics))
}

/// Cosine similarity of two equal-length vectors (both L2-normalized in
/// practice, so this is their dot product); zero when either is empty.
fn cosine(a: &[f32], b: &[f32]) -> f64 {
    if a.len() != b.len() {
        return 0.0;
    }
    f64::from(a.iter().zip(b).map(|(x, y)| x * y).sum::<f32>())
}

#[cfg(test)]
mod tests {
    use super::{
        CHUNK_INDEX_SCHEMA_VERSION, ChunkIndex, DesiredChunk, ProviderIdentity, reconcile,
    };
    use crate::retrieval::semantic_search::MockEmbeddingProvider;

    fn identity() -> ProviderIdentity {
        ProviderIdentity {
            model: "mock-hash-v1".to_owned(),
            dimensions: 64,
            normalized: true,
            document_prompt: "doc-v1".to_owned(),
            query_prompt: "query-v1".to_owned(),
        }
    }

    fn desired(id: &str, text: &str) -> DesiredChunk {
        DesiredChunk {
            chunk_id: id.to_owned(),
            content_hash: blake3::hash(text.as_bytes()).to_hex().to_string(),
            text: text.to_owned(),
        }
    }

    /// AC#8 cold build: every chunk is embedded, nothing reused, nothing deleted.
    #[test]
    fn cold_build_embeds_everything() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let prev = ChunkIndex::empty(&id);
        let chunks = vec![desired("chunk:a#0", "fn a"), desired("chunk:a#1", "fn b")];
        let (index, metrics) = reconcile(&prev, &chunks, &MockEmbeddingProvider, &id)?;
        assert_eq!(metrics.recomputed_vectors, 2);
        assert_eq!(metrics.reused_vectors, 0);
        assert_eq!(metrics.deleted_rows, 0);
        assert_eq!(index.records.len(), 2);
        Ok(())
    }

    /// AC#8 warm no-op: reconciling the same chunks reuses every vector.
    #[test]
    fn warm_rebuild_reuses_all() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let chunks = vec![desired("chunk:a#0", "fn a"), desired("chunk:a#1", "fn b")];
        let (first, _) = reconcile(
            &ChunkIndex::empty(&id),
            &chunks,
            &MockEmbeddingProvider,
            &id,
        )?;
        let (second, metrics) = reconcile(&first, &chunks, &MockEmbeddingProvider, &id)?;
        assert_eq!(metrics.reused_vectors, 2);
        assert_eq!(metrics.recomputed_vectors, 0);
        assert_eq!(first.records, second.records);
        Ok(())
    }

    /// AC#2/#8 one-chunk edit: only the changed chunk is re-embedded.
    #[test]
    fn one_chunk_edit_recomputes_only_that_chunk() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let before = vec![desired("chunk:a#0", "fn a"), desired("chunk:a#1", "fn b")];
        let (idx, _) = reconcile(
            &ChunkIndex::empty(&id),
            &before,
            &MockEmbeddingProvider,
            &id,
        )?;
        let after = vec![
            desired("chunk:a#0", "fn a"),
            desired("chunk:a#1", "fn b CHANGED"),
        ];
        let (_, metrics) = reconcile(&idx, &after, &MockEmbeddingProvider, &id)?;
        assert_eq!(metrics.recomputed_vectors, 1);
        assert_eq!(metrics.reused_vectors, 1);
        assert_eq!(metrics.deleted_rows, 0);
        Ok(())
    }

    /// AC#8 duplicate chunks: identical content in two chunk ids embeds once.
    #[test]
    fn duplicate_content_embeds_once() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let chunks = vec![desired("chunk:a#0", "same"), desired("chunk:b#0", "same")];
        let (_, metrics) = reconcile(
            &ChunkIndex::empty(&id),
            &chunks,
            &MockEmbeddingProvider,
            &id,
        )?;
        assert_eq!(
            metrics.recomputed_vectors, 1,
            "second identical chunk reuses"
        );
        assert_eq!(metrics.reused_vectors, 1);
        Ok(())
    }

    /// AC#3/#8 file rename: old chunk ids are deleted, content-equal vectors are
    /// reused for the new ids (no re-embed).
    #[test]
    fn file_rename_deletes_old_ids_and_reuses_vectors() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let before = vec![desired("chunk:old.rs#0", "fn a")];
        let (idx, _) = reconcile(
            &ChunkIndex::empty(&id),
            &before,
            &MockEmbeddingProvider,
            &id,
        )?;
        let after = vec![desired("chunk:new.rs#0", "fn a")];
        let (index, metrics) = reconcile(&idx, &after, &MockEmbeddingProvider, &id)?;
        assert_eq!(metrics.reused_vectors, 1, "same content reused via dedup");
        assert_eq!(metrics.recomputed_vectors, 0);
        assert_eq!(metrics.deleted_rows, 1, "old id dropped");
        assert_eq!(index.records.len(), 1);
        assert_eq!(index.records[0].chunk_id, "chunk:new.rs#0");
        Ok(())
    }

    /// AC#3/#8 file delete: removed chunks drop their records.
    #[test]
    fn file_delete_removes_records() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let before = vec![desired("chunk:a#0", "fn a"), desired("chunk:b#0", "fn b")];
        let (idx, _) = reconcile(
            &ChunkIndex::empty(&id),
            &before,
            &MockEmbeddingProvider,
            &id,
        )?;
        let after = vec![desired("chunk:a#0", "fn a")];
        let (index, metrics) = reconcile(&idx, &after, &MockEmbeddingProvider, &id)?;
        assert_eq!(metrics.deleted_rows, 1);
        assert_eq!(index.records.len(), 1);
        Ok(())
    }

    /// AC#4/#8 model change: a different provider digest forces a full rebuild.
    #[test]
    fn model_change_forces_full_rebuild() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let chunks = vec![desired("chunk:a#0", "fn a")];
        let (idx, _) = reconcile(
            &ChunkIndex::empty(&id),
            &chunks,
            &MockEmbeddingProvider,
            &id,
        )?;
        let changed = ProviderIdentity {
            model: "different-model".to_owned(),
            ..identity()
        };
        let (index, metrics) = reconcile(&idx, &chunks, &MockEmbeddingProvider, &changed)?;
        assert!(metrics.rebuilt);
        assert_eq!(metrics.recomputed_vectors, 1);
        assert_eq!(metrics.reused_vectors, 0);
        assert_eq!(metrics.deleted_rows, 1);
        assert_eq!(index.provider_digest, changed.digest());
        Ok(())
    }

    /// AC#4 schema change: a version mismatch forces a full rebuild.
    #[test]
    fn schema_change_forces_full_rebuild() -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let chunks = vec![desired("chunk:a#0", "fn a")];
        let mut stale = ChunkIndex::empty(&id);
        stale.schema_version = CHUNK_INDEX_SCHEMA_VERSION + 1;
        let (_, metrics) = reconcile(&stale, &chunks, &MockEmbeddingProvider, &id)?;
        assert!(metrics.rebuilt);
        assert_eq!(metrics.recomputed_vectors, 1);
        Ok(())
    }

    /// AC#8 corrupt index recovery: a garbage file loads as empty, so the next
    /// reconcile is a clean cold build.
    #[test]
    fn corrupt_index_loads_as_empty() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("chunk-index.json");
        std::fs::write(&path, "{ not json")?;
        let loaded = ChunkIndex::load(&path, &identity());
        assert!(loaded.records.is_empty());
        Ok(())
    }

    /// AC#1/#5/#8 persistence + query after interrupted update: an atomic save
    /// round-trips, a leftover `.tmp` from an interrupted write does not affect
    /// the committed index, and search runs off the persisted vectors.
    #[test]
    fn atomic_save_survives_interrupted_tmp_and_search_works()
    -> Result<(), Box<dyn std::error::Error>> {
        let id = identity();
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("chunk-index.json");
        let chunks = vec![
            desired("chunk:a#0", "route service"),
            desired("chunk:a#1", "banana"),
        ];
        let (index, _) = reconcile(
            &ChunkIndex::empty(&id),
            &chunks,
            &MockEmbeddingProvider,
            &id,
        )?;
        index.save(&path)?;

        // Simulate an interrupted update: a stray temp file must not be read.
        std::fs::write(path.with_extension("tmp"), "garbage")?;
        let reloaded = ChunkIndex::load(&path, &id);
        assert_eq!(reloaded, index, "committed index intact despite stray tmp");

        // Search runs off persisted vectors without re-embedding documents.
        use crate::retrieval::semantic_search::EmbeddingProvider;
        let query = MockEmbeddingProvider.embed("route service")?;
        let results = reloaded.search(&query, 1);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "chunk:a#0");
        Ok(())
    }
}
