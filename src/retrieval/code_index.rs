//! Enriched semantic code-search index and query entry point (LIT-86.6).
//!
//! Ties the pieces of the hybrid retrieval stack into one persisted, queryable
//! surface: chunks (LIT-86.2) are embedded and reconciled into a vector index
//! (LIT-86.3), enriched with graph context (LIT-86.4), and ranked under graph
//! constraints (LIT-86.5). The result is exposed identically through the CLI,
//! the MCP server, and the local viewer.
//!
//! The index is built as a first-class step of every `init`/`update` run
//! (`orchestrate::build_code_search_index`) and cached under
//! `.lithograph/derived/`; it is also (re)built lazily on a search if missing
//! or when `--refresh` is passed. A search reports whether that cache is fresh
//! relative to the current graph without silently rebuilding it (freshness is
//! diagnosed, not forced). Real embedding providers stay opt-in; the
//! deterministic mock provider runs in every normal test and in `baseline-pr`,
//! so nothing here makes a network call.

// ponytail: fallback (line) chunking is used for every artifact here so the
// surface can ship without per-artifact adapter wiring. Syntax-boundary
// chunking via the parse-once arena (LIT-86.14) is a quality refinement for the
// pipeline-integration wave (LIT-86.9-86.11); the machinery already exists.

use crate::analysis::chunks::{ChunkConfig, ChunkParse, chunk_source};
use crate::domain::ModelExposurePolicy;
use crate::graph::{Graph, GraphNodeId, GraphStore};
use crate::retrieval::chunk_enrich::{EnrichedChunk, enrich_chunk};
use crate::retrieval::chunk_index::{
    CHUNK_INDEX_SCHEMA_VERSION, ChunkIndex, DesiredChunk, ProviderIdentity, reconcile,
};
use crate::retrieval::chunk_rank::{
    CHUNK_SCORING_VERSION, Candidate, Expansion, RankDiagnostics, RankError, RankFilters,
    RankQuery, RankedResult, rank,
};
use crate::retrieval::semantic_search::{EMBEDDING_DIMENSIONS, EmbeddingProvider};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// Schema version of the persisted code-search cache.
pub(crate) const CODE_INDEX_SCHEMA_VERSION: u32 = 1;

/// Vectors fetched from the index before graph-constrained re-ranking.
const PREFETCH: usize = 200;

/// Bound on typed graph neighbors attached to each chunk.
const MAX_NEIGHBORS: usize = 16;

/// One cataloged chunk: its graph enrichment plus its detected language, joined
/// to a vector by `chunk_id` at query time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CatalogEntry {
    enriched: EnrichedChunk,
    language: String,
}

/// The persisted code-search cache: the vector index, the enrichment catalog,
/// and the graph hash they were built against (for freshness).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CodeIndexFile {
    schema_version: u32,
    scoring_version: u32,
    graph_hash: String,
    vectors: ChunkIndex,
    catalog: Vec<CatalogEntry>,
}

impl CodeIndexFile {
    fn path(root: &Path) -> PathBuf {
        root.join(".lithograph/derived/code-search.json")
    }

    /// Loads the cache, returning `None` when it is missing, corrupt, or a
    /// different schema version (miss-is-safe: a stale cache is rebuilt).
    fn load(root: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(Self::path(root)).ok()?;
        let file: Self = serde_json::from_str(&text).ok()?;
        (file.schema_version == CODE_INDEX_SCHEMA_VERSION).then_some(file)
    }

    /// Atomically writes the cache: staged to `*.tmp`, then renamed.
    fn save(&self, root: &Path) -> Result<(), CodeSearchError> {
        let path = Self::path(root);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string(self)?;
        json.push('\n');
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }
}

/// Whether the cached index reflects the current repository graph (AC#4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Freshness {
    /// Whether a cache existed before this search.
    pub index_present: bool,
    /// Graph hash the cache was built against, if present.
    pub index_graph_hash: Option<String>,
    /// Graph hash of the current repository.
    pub current_graph_hash: String,
    /// True when the two hashes match (index up to date).
    pub is_fresh: bool,
}

/// One search request.
#[derive(Debug, Clone, Default)]
pub(crate) struct CodeSearchRequest {
    /// Natural-language or code query.
    pub query: String,
    /// Hard graph/path/language filters.
    pub filters: RankFilters,
    /// Bounded graph expansion.
    pub expansion: Expansion,
    /// Page size (`0` becomes a bounded default).
    pub limit: usize,
    /// Page offset.
    pub offset: usize,
    /// Force a rebuild of the cache before searching.
    pub refresh: bool,
}

/// A full search response with every field a caller needs to reproduce and
/// explain the result (AC#6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct CodeSearchResponse {
    /// Ranked page of results.
    pub results: Vec<RankedResult>,
    /// Constraint diagnostics.
    pub diagnostics: RankDiagnostics,
    /// Index vs repository freshness.
    pub freshness: Freshness,
    /// Provider/model identity string.
    pub provider_model: String,
    /// Provider identity digest.
    pub provider_digest: String,
    /// Index schema version.
    pub index_schema_version: u32,
    /// Scoring contract version.
    pub scoring_version: u32,
    /// Total results before pagination.
    pub total_matched: usize,
    /// Applied page offset.
    pub offset: usize,
    /// Applied page limit.
    pub limit: usize,
}

/// A code-search failure.
#[derive(Debug)]
pub(crate) enum CodeSearchError {
    /// Repository analysis failed.
    Analyze(String),
    /// Embedding failed.
    Embed(String),
    /// I/O or serialization failure.
    Io(String),
}

impl std::fmt::Display for CodeSearchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Analyze(message) => write!(formatter, "repository analysis failed: {message}"),
            Self::Embed(message) => write!(formatter, "embedding failed: {message}"),
            Self::Io(message) => write!(formatter, "code index i/o failed: {message}"),
        }
    }
}

impl std::error::Error for CodeSearchError {}

impl From<std::io::Error> for CodeSearchError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}

impl From<serde_json::Error> for CodeSearchError {
    fn from(error: serde_json::Error) -> Self {
        Self::Io(error.to_string())
    }
}

/// The provider identity for the deterministic mock provider (or any provider
/// exposing `model_identity`): dimensions probed once, normalized vectors, and
/// the document/query embedding purposes (LIT-86.15).
pub(crate) fn provider_identity(provider: &dyn EmbeddingProvider) -> ProviderIdentity {
    ProviderIdentity {
        model: provider.model_identity(),
        dimensions: EMBEDDING_DIMENSIONS,
        normalized: true,
        document_prompt: "code-document-v1".to_owned(),
        query_prompt: "code-query-v1".to_owned(),
    }
}

fn hash_graph(graph: &Graph) -> Result<String, CodeSearchError> {
    let json = graph
        .to_json()
        .map_err(|error| CodeSearchError::Io(error.to_string()))?;
    Ok(blake3::hash(json.as_bytes()).to_hex().to_string())
}

/// Chunks, enriches, embeds, and reconciles the whole repository into a fresh
/// cache file, reusing unchanged vectors from `previous`.
fn build_file(
    root: &Path,
    artifacts_graph: (&[crate::domain::Artifact], &Graph),
    graph_hash: &str,
    previous: &ChunkIndex,
    provider: &dyn EmbeddingProvider,
    identity: &ProviderIdentity,
) -> Result<CodeIndexFile, CodeSearchError> {
    let (artifacts, graph) = artifacts_graph;
    let layers: BTreeMap<GraphNodeId, String> = BTreeMap::new();
    let config = ChunkConfig::default();
    let parse = ChunkParse::Fallback {
        reason: "pipeline line chunking".to_owned(),
    };

    let mut desired = Vec::new();
    let mut catalog = Vec::new();
    for artifact in artifacts {
        // Safety gate (LIT-86.2 AC#6 / LIT-86.14 AC#5): secret/credential
        // artifacts never reach an embedding, and non-UTF-8 (binary) content is
        // skipped by the failing read below.
        if artifact.model_policy == ModelExposurePolicy::Never {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(artifact.path.as_str())) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        let language = artifact
            .detected_format
            .clone()
            .unwrap_or_else(|| "text".to_owned());
        for chunk in chunk_source(artifact.path.as_str(), &text, &[], &parse, &config) {
            let enriched = enrich_chunk(graph, &artifact.path, &chunk, &layers, MAX_NEIGHBORS);
            desired.push(DesiredChunk {
                chunk_id: chunk.id.clone(),
                content_hash: chunk.content_hash.clone(),
                text: chunk.text.clone(),
            });
            catalog.push(CatalogEntry {
                enriched,
                language: language.clone(),
            });
        }
    }

    let (vectors, _metrics) = reconcile(previous, &desired, provider, identity)
        .map_err(|error| CodeSearchError::Embed(error.to_string()))?;
    let file = CodeIndexFile {
        schema_version: CODE_INDEX_SCHEMA_VERSION,
        scoring_version: CHUNK_SCORING_VERSION,
        graph_hash: graph_hash.to_owned(),
        vectors,
        catalog,
    };
    file.save(root)?;
    Ok(file)
}

/// Builds and persists the code-search index during a pipeline run from the
/// already-analyzed `artifacts` and `graph` (LIT-86.6 first-class step), so a
/// later search finds a warm, reconciled index instead of building lazily.
/// Reuses unchanged vectors from any prior cache. The caller treats this as
/// best-effort: the index is a rebuildable derived sidecar, so a failure here
/// must never fail the run.
pub(crate) fn build_for_run(
    root: &Path,
    artifacts: &[crate::domain::Artifact],
    graph: &Graph,
    provider: &dyn EmbeddingProvider,
    identity: &ProviderIdentity,
) -> Result<(), CodeSearchError> {
    let graph_hash = hash_graph(graph)?;
    let previous = CodeIndexFile::load(root)
        .map(|file| file.vectors)
        .unwrap_or_else(|| ChunkIndex::empty(identity));
    build_file(
        root,
        (artifacts, graph),
        &graph_hash,
        &previous,
        provider,
        identity,
    )?;
    Ok(())
}

/// Produces a dry-run [`ExplainPlan`](crate::explain::ExplainPlan) of what a
/// code-search index refresh *would* do, without writing anything (LIT-86.17
/// AC#4). Each chunk that is new, changed, unchanged, or orphaned maps to a
/// deterministic action + reason code; an incompatible provider/schema makes
/// every entry a provider-model rebuild. Bounded by the chunk count.
pub(crate) fn explain(
    root: &Path,
    identity: &ProviderIdentity,
) -> Result<crate::explain::ExplainPlan, CodeSearchError> {
    use crate::explain::{Action, ExplainEntry, ExplainPlan, ReasonCode};

    let (artifacts, _graph, _) = crate::orchestrate::analyze_repository(root)
        .map_err(|error| CodeSearchError::Analyze(error.to_string()))?;
    let config = ChunkConfig::default();
    let parse = ChunkParse::Fallback {
        reason: "pipeline line chunking".to_owned(),
    };

    // Desired chunk identities (id -> content hash) for the current repository.
    let mut desired: BTreeMap<String, String> = BTreeMap::new();
    for artifact in &artifacts {
        if artifact.model_policy == ModelExposurePolicy::Never {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(artifact.path.as_str())) else {
            continue;
        };
        if text.trim().is_empty() {
            continue;
        }
        for chunk in chunk_source(artifact.path.as_str(), &text, &[], &parse, &config) {
            desired.insert(chunk.id, chunk.content_hash);
        }
    }

    let cached = CodeIndexFile::load(root);
    let provider_compatible = cached
        .as_ref()
        .is_some_and(|file| file.vectors.provider_digest == identity.digest());
    let previous: BTreeMap<String, String> = cached
        .map(|file| {
            file.vectors
                .records
                .into_iter()
                .map(|record| (record.chunk_id, record.content_hash))
                .collect()
        })
        .unwrap_or_default();

    let mut entries = Vec::new();
    for (chunk_id, hash) in &desired {
        let (action, reason) = match previous.get(chunk_id) {
            // A provider/schema change re-embeds everything.
            _ if !provider_compatible && !previous.is_empty() => {
                (Action::Update, ReasonCode::ProviderModelChanged)
            }
            Some(previous_hash) if previous_hash == hash => {
                (Action::Reuse, ReasonCode::UnchangedReuse)
            }
            Some(_) => (Action::Update, ReasonCode::InputChanged),
            None => (Action::Insert, ReasonCode::MissingState),
        };
        entries.push(ExplainEntry {
            component_path: "search-index/vector".to_owned(),
            key: chunk_id.clone(),
            action,
            compatible: matches!(action, Action::Reuse),
            reason,
            differing_field: (!matches!(action, Action::Reuse)).then(|| "content_hash".to_owned()),
            dependency_path: Vec::new(),
        });
    }
    // Orphans: previously indexed chunks no longer present.
    for chunk_id in previous.keys() {
        if !desired.contains_key(chunk_id) {
            entries.push(ExplainEntry::orphan_delete(
                "search-index/vector",
                chunk_id.clone(),
            ));
        }
    }
    Ok(ExplainPlan::new(entries))
}

/// Rebuilds the code-search cache for `root` from scratch (reusing unchanged
/// vectors from any prior cache). Explicit refresh entry point (AC#4).
pub(crate) fn refresh(
    root: &Path,
    provider: &dyn EmbeddingProvider,
    identity: &ProviderIdentity,
) -> Result<CodeIndexFile, CodeSearchError> {
    let (artifacts, graph, _) = crate::orchestrate::analyze_repository(root)
        .map_err(|error| CodeSearchError::Analyze(error.to_string()))?;
    let graph_hash = hash_graph(&graph)?;
    let previous = CodeIndexFile::load(root)
        .map(|file| file.vectors)
        .unwrap_or_else(|| ChunkIndex::empty(identity));
    build_file(
        root,
        (&artifacts, &graph),
        &graph_hash,
        &previous,
        provider,
        identity,
    )
}

/// Runs a code search: analyzes the current repository, rebuilds the cache only
/// when missing or explicitly requested (a stale cache is used and *reported*,
/// not silently rebuilt -- AC#4), then embeds the query and ranks under the
/// graph constraints.
pub(crate) fn search(
    root: &Path,
    request: &CodeSearchRequest,
    provider: &dyn EmbeddingProvider,
    identity: &ProviderIdentity,
) -> Result<CodeSearchResponse, CodeSearchError> {
    let (artifacts, graph, _) = crate::orchestrate::analyze_repository(root)
        .map_err(|error| CodeSearchError::Analyze(error.to_string()))?;
    let current_hash = hash_graph(&graph)?;
    let cached = CodeIndexFile::load(root);
    let index_present = cached.is_some();

    let file = if request.refresh || cached.is_none() {
        let previous = cached
            .map(|file| file.vectors)
            .unwrap_or_else(|| ChunkIndex::empty(identity));
        build_file(
            root,
            (&artifacts, &graph),
            &current_hash,
            &previous,
            provider,
            identity,
        )?
    } else {
        // Safe unwrap: `cached.is_none()` handled above.
        cached.unwrap_or_else(unreachable_cache)
    };

    // Freshness reflects the index actually used: after a (re)build it matches
    // the current graph; on the fast path a stale cache reports `false` without
    // having been rebuilt (AC#4).
    let is_fresh = file.graph_hash == current_hash;
    let freshness = Freshness {
        index_present,
        index_graph_hash: Some(file.graph_hash.clone()),
        current_graph_hash: current_hash,
        is_fresh,
    };

    // Rank the persisted graph's view so results stay consistent with what a
    // caller can navigate; fall back to the freshly analyzed graph when no
    // persisted graph exists yet.
    let ranking_graph = GraphStore::new(root)
        .load()
        .map(|stored| stored.graph)
        .unwrap_or(graph);

    let query_vector = provider
        .embed(&request.query)
        .map_err(|error| CodeSearchError::Embed(error.to_string()))?;
    let hits = file.vectors.search(&query_vector, PREFETCH);
    let catalog: HashMap<&str, &CatalogEntry> = file
        .catalog
        .iter()
        .map(|entry| (entry.enriched.chunk_id.as_str(), entry))
        .collect();
    let candidates: Vec<Candidate> = hits
        .iter()
        .filter_map(|hit| {
            let entry = catalog.get(hit.chunk_id.as_str())?;
            Some(Candidate {
                enriched: entry.enriched.clone(),
                vector_score: hit.score,
                lexical_score: 0.0,
                language: entry.language.clone(),
            })
        })
        .collect();

    let rank_query = RankQuery {
        filters: request.filters.clone(),
        expansion: request.expansion.clone(),
        weights: None,
    };
    let (ranked, diagnostics) = match rank(&ranking_graph, &rank_query, &candidates) {
        Ok(pair) => pair,
        Err(RankError::NoCandidates) => (Vec::new(), RankDiagnostics::default()),
        Err(error) => return Err(CodeSearchError::Embed(error.to_string())),
    };

    let total_matched = ranked.len();
    let limit = if request.limit == 0 {
        20
    } else {
        request.limit
    };
    let results: Vec<RankedResult> = ranked
        .into_iter()
        .skip(request.offset)
        .take(limit)
        .collect();

    Ok(CodeSearchResponse {
        results,
        diagnostics,
        freshness,
        provider_model: identity.model.clone(),
        provider_digest: identity.digest(),
        index_schema_version: CHUNK_INDEX_SCHEMA_VERSION,
        scoring_version: CHUNK_SCORING_VERSION,
        total_matched,
        offset: request.offset,
        limit,
    })
}

/// Unreachable helper documenting the invariant that `search` never uses a
/// missing cache on the fast path.
fn unreachable_cache() -> CodeIndexFile {
    CodeIndexFile {
        schema_version: CODE_INDEX_SCHEMA_VERSION,
        scoring_version: CHUNK_SCORING_VERSION,
        graph_hash: String::new(),
        vectors: ChunkIndex::empty(&ProviderIdentity {
            model: String::new(),
            dimensions: 0,
            normalized: false,
            document_prompt: String::new(),
            query_prompt: String::new(),
        }),
        catalog: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::{CodeSearchRequest, explain, provider_identity, refresh, search};
    use crate::retrieval::chunk_rank::RankFilters;
    use crate::retrieval::semantic_search::MockEmbeddingProvider;
    use std::path::Path;

    /// Writes a tiny repository, indexes it, and searches -- the end-to-end
    /// parity the CLI/MCP/viewer surfaces all call into (AC#7).
    fn fixture_repo() -> Result<tempfile::TempDir, Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("router.py"),
            "def route_service(request):\n    return handle(request)\n\ndef handle(request):\n    return 200\n",
        )?;
        std::fs::write(
            temp.path().join("unrelated.py"),
            "def banana_smoothie():\n    return 'yum'\n",
        )?;
        Ok(temp)
    }

    #[test]
    fn cold_search_builds_index_and_returns_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let identity = provider_identity(&MockEmbeddingProvider);
        let request = CodeSearchRequest {
            query: "route service request".to_owned(),
            limit: 10,
            ..CodeSearchRequest::default()
        };
        let response = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;

        assert!(!response.results.is_empty(), "cold search returns hits");
        // Full metadata is present (AC#6).
        assert_eq!(response.provider_model, "mock-hash-v1");
        assert!(!response.provider_digest.is_empty());
        assert_eq!(response.scoring_version, 1);
        // The cache was absent, then built this call; the built index reflects
        // the current graph (AC#4).
        assert!(!response.freshness.index_present);
        assert!(response.freshness.is_fresh);
        // The router chunk should rank above the unrelated one.
        assert!(
            response.results[0]
                .evidence
                .path
                .as_str()
                .contains("router.py")
        );
        Ok(())
    }

    #[test]
    fn second_search_is_fresh_and_paginates() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let identity = provider_identity(&MockEmbeddingProvider);
        // Build the cache.
        refresh(repo.path(), &MockEmbeddingProvider, &identity)?;

        let request = CodeSearchRequest {
            query: "handle request".to_owned(),
            limit: 1,
            offset: 0,
            ..CodeSearchRequest::default()
        };
        let response = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
        assert!(response.freshness.index_present);
        assert!(response.freshness.is_fresh, "cache matches current graph");
        assert!(response.results.len() <= 1, "limit is honored");
        assert!(response.total_matched >= response.results.len());
        Ok(())
    }

    #[test]
    fn language_filter_excludes_non_matching() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let identity = provider_identity(&MockEmbeddingProvider);
        let request = CodeSearchRequest {
            query: "route".to_owned(),
            filters: RankFilters {
                language: Some("does-not-exist".to_owned()),
                ..RankFilters::default()
            },
            limit: 10,
            ..CodeSearchRequest::default()
        };
        let response = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
        assert!(
            response.results.is_empty(),
            "no python matches a bogus language"
        );
        Ok(())
    }

    /// AC#4: a graph change makes an unrefreshed index report stale rather than
    /// silently rebuilding.
    #[test]
    fn edit_makes_index_report_stale_without_refresh() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        let identity = provider_identity(&MockEmbeddingProvider);
        refresh(repo.path(), &MockEmbeddingProvider, &identity)?;

        // Change the repository so the current graph hash diverges.
        std::fs::write(
            repo.path().join("new_module.py"),
            "def brand_new_symbol():\n    return 1\n",
        )?;

        let request = CodeSearchRequest {
            query: "route".to_owned(),
            limit: 5,
            refresh: false,
            ..CodeSearchRequest::default()
        };
        let response = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
        assert!(!response.freshness.is_fresh, "index is stale after an edit");
        assert!(response.freshness.index_present);
        // Re-running with refresh brings it back to fresh.
        let refreshed = search(
            repo.path(),
            &CodeSearchRequest {
                refresh: true,
                ..request
            },
            &MockEmbeddingProvider,
            &identity,
        )?;
        assert!(refreshed.freshness.is_fresh);
        Ok(())
    }

    /// LIT-86.17 AC#4/#5: explain produces a per-chunk dry-run plan and writes
    /// nothing; a warm index reports all reuse, an edit reports an update.
    #[test]
    fn explain_is_a_dry_run_plan() -> Result<(), Box<dyn std::error::Error>> {
        use crate::explain::{Action, ReasonCode};
        let repo = fixture_repo()?;
        let identity = provider_identity(&MockEmbeddingProvider);
        refresh(repo.path(), &MockEmbeddingProvider, &identity)?;

        // Warm: every chunk is unchanged -> reuse.
        let plan = explain(repo.path(), &identity)?;
        assert!(!plan.entries.is_empty());
        assert!(
            plan.entries
                .iter()
                .all(|entry| entry.action == Action::Reuse),
            "warm index reuses everything"
        );

        // Edit a file; explain now reports an update/insert without writing.
        std::fs::write(
            repo.path().join("router.py"),
            "def route_service(request):\n    return handle_v2(request)\n",
        )?;
        let before_mtime =
            std::fs::metadata(repo.path().join(".lithograph/derived/code-search.json"))?
                .modified()?;
        let edited_plan = explain(repo.path(), &identity)?;
        assert!(
            edited_plan.entries.iter().any(|entry| matches!(
                entry.action,
                Action::Update | Action::Insert
            ) && entry.reason != ReasonCode::UnchangedReuse),
            "edit shows a non-reuse action"
        );
        // Dry run: the persisted index file was not rewritten.
        let after_mtime =
            std::fs::metadata(repo.path().join(".lithograph/derived/code-search.json"))?
                .modified()?;
        assert_eq!(before_mtime, after_mtime, "explain writes nothing");
        Ok(())
    }

    #[test]
    fn secrets_are_never_indexed() -> Result<(), Box<dyn std::error::Error>> {
        let repo = fixture_repo()?;
        // A .env file is classified model-exposure Never and must not be chunked.
        std::fs::write(
            repo.path().join(".env"),
            "API_KEY=supersecret route service\n",
        )?;
        let identity = provider_identity(&MockEmbeddingProvider);
        let request = CodeSearchRequest {
            query: "supersecret API_KEY".to_owned(),
            limit: 10,
            ..CodeSearchRequest::default()
        };
        let response = search(repo.path(), &request, &MockEmbeddingProvider, &identity)?;
        assert!(
            response
                .results
                .iter()
                .all(|result| !result.evidence.path.as_str().contains(".env")),
            "the .env secret file is never indexed or returned"
        );
        let _ = Path::new(".");
        Ok(())
    }
}
