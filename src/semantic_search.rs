//! Pluggable semantic search: a swappable [`EmbeddingProvider`] behind a
//! deterministic mock backend for normal tests, blended with a graph
//! connectivity signal so results reflect both textual meaning and
//! structural importance (LIT-22.4.4).

use crate::domain::EvidenceRef;
use crate::fts::tokenize;
use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt::{Display, Formatter};
use std::path::Path;

/// Version of the persisted semantic embedding index contract.
pub(crate) const EMBEDDING_INDEX_VERSION: u32 = 1;

/// Fixed embedding dimensionality used by every provider in this crate, so
/// vectors from different providers are at least comparably shaped (real
/// providers may still differ semantically, but never in size).
const EMBEDDING_DIMENSIONS: usize = 64;

/// Failure embedding a piece of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmbeddingError {
    /// Human-readable failure description.
    pub message: String,
}

impl Display for EmbeddingError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EmbeddingError {}

/// Provider boundary for turning text into a fixed-size embedding vector
/// (AC1). Implementations may call a real local or remote model; nothing
/// in this trait requires it.
pub(crate) trait EmbeddingProvider {
    /// Embeds `text` into a vector of [`EMBEDDING_DIMENSIONS`] entries.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;

    /// Stable provider/model identity used for cache invalidation.
    fn model_identity(&self) -> String {
        "provider".to_owned()
    }
}

/// Deterministic, offline embedding provider (AC1/AC3): feature-hashes
/// each token into a fixed-size vector (the "hashing trick"), so texts
/// sharing more tokens land closer together by cosine similarity, with no
/// network access and no live model. Every normal test uses this, never a
/// real provider.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MockEmbeddingProvider;

impl EmbeddingProvider for MockEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let mut vector = vec![0.0f32; EMBEDDING_DIMENSIONS];
        for token in tokenize(text) {
            let hash = blake3::hash(token.as_bytes());
            let bytes = hash.as_bytes();
            let bucket_bytes: [u8; 8] = bytes[0..8].try_into().unwrap_or_default();
            let bucket = (u64::from_le_bytes(bucket_bytes) as usize) % EMBEDDING_DIMENSIONS;
            let sign = if bytes[8].is_multiple_of(2) {
                1.0
            } else {
                -1.0
            };
            vector[bucket] += sign;
        }
        Ok(normalize(vector))
    }

    fn model_identity(&self) -> String {
        "mock-hash-v1".to_owned()
    }
}

/// Optional local embedding backend. It uses the deterministic hashing model
/// but has a distinct identity so a future learned local model can invalidate
/// the same cache contract without changing callers.
#[cfg(feature = "local-embeddings")]
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalEmbeddingProvider;

#[cfg(feature = "local-embeddings")]
impl EmbeddingProvider for LocalEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        MockEmbeddingProvider.embed(text)
    }

    fn model_identity(&self) -> String {
        "local-hash-v1".to_owned()
    }
}

/// Configuration for a real, OpenAI-compatible `/embeddings` endpoint
/// (AC1: optional real backend). Any server implementing that response
/// shape works (OpenAI itself, or a compatible local/hosted server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OpenAiEmbeddingConfig {
    /// API base URL, e.g. `https://api.openai.com/v1`.
    pub base_url: String,
    /// Bearer API key. Never included verbatim in error messages.
    pub api_key: String,
    /// Embedding model name.
    pub model: String,
}

/// Real embedding provider over an OpenAI-compatible `/embeddings`
/// endpoint. Never constructed by any normal test (AC3); exists so a
/// caller can opt into live semantic search.
pub(crate) struct OpenAiEmbeddingProvider {
    config: OpenAiEmbeddingConfig,
    agent: ureq::Agent,
}

impl OpenAiEmbeddingProvider {
    /// Builds a provider from `config`.
    pub(crate) fn new(config: OpenAiEmbeddingConfig) -> Self {
        Self {
            config,
            agent: ureq::Agent::new_with_defaults(),
        }
    }
}

#[derive(Serialize)]
struct EmbeddingRequestBody<'a> {
    model: &'a str,
    input: &'a str,
}

#[derive(Deserialize)]
struct EmbeddingResponseBody {
    data: Vec<EmbeddingResponseEntry>,
}

#[derive(Deserialize)]
struct EmbeddingResponseEntry {
    embedding: Vec<f32>,
}

impl EmbeddingProvider for OpenAiEmbeddingProvider {
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError> {
        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let body = EmbeddingRequestBody {
            model: &self.config.model,
            input: text,
        };
        let response: EmbeddingResponseBody = self
            .agent
            .post(&url)
            .header("Authorization", &format!("Bearer {}", self.config.api_key))
            .send_json(&body)
            .map_err(|error| EmbeddingError {
                message: format!("embedding request failed: {error}"),
            })?
            .body_mut()
            .read_json()
            .map_err(|error| EmbeddingError {
                message: format!("failed to parse embedding response: {error}"),
            })?;
        response
            .data
            .into_iter()
            .next()
            .map(|entry| entry.embedding)
            .ok_or_else(|| EmbeddingError {
                message: "embedding response had no data entries".to_owned(),
            })
    }

    fn model_identity(&self) -> String {
        format!("openai-compatible:{}", self.config.model)
    }
}

fn normalize(mut vector: Vec<f32>) -> Vec<f32> {
    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in &mut vector {
            *value /= magnitude;
        }
    }
    vector
}

fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    f64::from(dot)
}

/// One enriched node document eligible for semantic search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct NodeDocument {
    /// Stable graph node identifier.
    pub id: GraphNodeId,
    /// Graph node kind, such as `Symbol` or `Config`.
    pub kind: String,
    /// Human-readable node name.
    pub name: String,
    /// Repository-relative path when the node has source evidence.
    pub path: Option<String>,
    /// Owning service/job name when graph ownership identifies one.
    pub service: Option<String>,
    /// Text supplied to an embedding provider.
    pub reference: String,
    /// Enriched text used for embedding.
    pub text: String,
    /// Nearby evidence for explainability.
    pub evidence: Option<EvidenceRef>,
    /// Stable nearby graph context.
    pub graph_refs: Vec<String>,
}

/// One cached embedding and its enriched source document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EmbeddingIndexEntry {
    /// Enriched source document.
    pub document: NodeDocument,
    /// Normalized embedding vector.
    pub vector: Vec<f32>,
    /// Graph degree captured when the index was built.
    pub graph_degree: u32,
}

/// Versioned, serializable semantic embedding index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EmbeddingIndex {
    /// Index schema version.
    pub version: u32,
    /// Provider/model identity.
    pub model_identity: String,
    /// Source graph hash used to invalidate stale entries.
    pub source_hash: String,
    /// Embedding dimensionality.
    pub dimensions: usize,
    /// Stable sorted entries.
    pub entries: Vec<EmbeddingIndexEntry>,
}

impl EmbeddingIndex {
    /// Builds an index from enriched graph documents.
    pub(crate) fn build(
        provider: &dyn EmbeddingProvider,
        graph: &Graph,
        source_hash: impl Into<String>,
    ) -> Result<Self, EmbeddingError> {
        let documents = collect_documents(graph);
        let mut entries = Vec::with_capacity(documents.len());
        let degree = degree_index(graph);
        for document in documents {
            let vector = provider.embed(&document.text)?;
            entries.push(EmbeddingIndexEntry {
                graph_degree: *degree.get(&document.id).unwrap_or(&0),
                document,
                vector,
            });
        }
        Ok(Self {
            version: EMBEDDING_INDEX_VERSION,
            model_identity: provider.model_identity(),
            source_hash: source_hash.into(),
            dimensions: entries
                .first()
                .map_or(EMBEDDING_DIMENSIONS, |entry| entry.vector.len()),
            entries,
        })
    }

    /// Returns whether this index matches the current source and model.
    pub(crate) fn is_compatible(
        &self,
        provider: &dyn EmbeddingProvider,
        source_hash: &str,
    ) -> bool {
        self.version == EMBEDDING_INDEX_VERSION
            && self.model_identity == provider.model_identity()
            && self.source_hash == source_hash
            && self
                .entries
                .iter()
                .all(|entry| entry.vector.len() == self.dimensions)
    }

    /// Writes the index only when its deterministic JSON changes.
    pub(crate) fn save_if_changed(&self, path: &Path) -> std::io::Result<bool> {
        let payload = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        if std::fs::read(path).ok().as_deref() == Some(payload.as_slice()) {
            return Ok(false);
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, payload)?;
        Ok(true)
    }

    /// Loads a cached index, returning `None` when it does not exist.
    pub(crate) fn load(path: &Path) -> std::io::Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let payload = std::fs::read(path)?;
        serde_json::from_slice(&payload)
            .map(Some)
            .map_err(std::io::Error::other)
    }
}

/// One semantic search result (AC2/AC4): the blended score plus the
/// vector/graph components that produced it, real evidence, and related
/// graph references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct SemanticSearchResult {
    /// Source graph node id.
    pub document_id: String,
    /// Human-readable reference (path or qualified name).
    pub reference: String,
    /// Evidence backing this result, when the source node carries one.
    pub evidence: Option<EvidenceRef>,
    /// Related graph node references (e.g. the containing artifact, or
    /// contained symbols).
    pub graph_refs: Vec<String>,
    /// Cosine similarity between the query and document embeddings.
    pub vector_score: f64,
    /// Normalized graph connectivity (relation degree) signal.
    pub graph_score: f64,
    /// `vector_weight * vector_score + graph_weight * graph_score`.
    pub combined_score: f64,
}

/// Blends embedding similarity with a graph connectivity signal (AC2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct SemanticSearchWeights {
    /// Weight applied to the cosine similarity component.
    pub vector: f64,
    /// Weight applied to the graph connectivity component.
    pub graph: f64,
}

impl Default for SemanticSearchWeights {
    fn default() -> Self {
        Self {
            vector: 0.7,
            graph: 0.3,
        }
    }
}

/// Semantic search over the knowledge graph, backed by a pluggable
/// [`EmbeddingProvider`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SemanticSearch;

impl SemanticSearch {
    /// Embeds `query` with `provider`, then ranks every eligible graph
    /// node by a weighted blend of cosine similarity and graph
    /// connectivity (AC2), returning at most `limit` results with
    /// evidence and graph references (AC4).
    pub(crate) fn search(
        &self,
        provider: &dyn EmbeddingProvider,
        graph: &Graph,
        query: &str,
        limit: usize,
        weights: SemanticSearchWeights,
    ) -> Result<Vec<SemanticSearchResult>, EmbeddingError> {
        let limit = if limit == 0 { 10 } else { limit };
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let source_hash =
            blake3::hash(&serde_json::to_vec(graph).map_err(|error| EmbeddingError {
                message: format!("failed to hash graph for embeddings: {error}"),
            })?)
            .to_hex()
            .to_string();
        let index = EmbeddingIndex::build(provider, graph, source_hash)?;
        self.search_index(provider, &index, query, limit, weights)
    }

    /// Searches a previously built embedding index without rebuilding vectors.
    pub(crate) fn search_index(
        &self,
        provider: &dyn EmbeddingProvider,
        index: &EmbeddingIndex,
        query: &str,
        limit: usize,
        weights: SemanticSearchWeights,
    ) -> Result<Vec<SemanticSearchResult>, EmbeddingError> {
        let limit = if limit == 0 { 10 } else { limit };
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let query_vector = provider.embed(query)?;
        let max_degree = index
            .entries
            .iter()
            .map(|entry| entry.graph_degree)
            .max()
            .unwrap_or(0)
            .max(1) as f64;

        let mut scored: Vec<SemanticSearchResult> = index
            .entries
            .iter()
            .map(|entry| {
                let vector_score = cosine_similarity(&query_vector, &entry.vector);
                let graph_score = f64::from(entry.graph_degree) / max_degree;
                let combined_score = weights.vector * vector_score + weights.graph * graph_score;
                Ok(SemanticSearchResult {
                    document_id: entry.document.id.as_str().to_owned(),
                    reference: entry.document.reference.clone(),
                    evidence: entry.document.evidence.clone(),
                    graph_refs: entry.document.graph_refs.clone(),
                    vector_score,
                    graph_score,
                    combined_score,
                })
            })
            .collect::<Result<_, EmbeddingError>>()?;

        scored.sort_by(|a, b| {
            b.combined_score
                .partial_cmp(&a.combined_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.document_id.cmp(&b.document_id))
        });
        scored.truncate(limit);
        Ok(scored)
    }
}

pub(crate) fn collect_documents(graph: &Graph) -> Vec<NodeDocument> {
    let contained_by: BTreeMap<&GraphNodeId, &GraphNodeId> = graph
        .relations
        .iter()
        .filter(|relation| relation.kind == RelationKind::Contains)
        .map(|relation| (&relation.target, &relation.source))
        .collect();
    let node_label: BTreeMap<&GraphNodeId, String> = graph
        .nodes
        .iter()
        .map(|node| (node.id(), node_display_name(node)))
        .collect();

    let mut documents: Vec<NodeDocument> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Symbol(symbol) => {
                let container_ref = contained_by
                    .get(&symbol.id)
                    .and_then(|id| node_label.get(id))
                    .cloned();
                let evidence = symbol.evidence.clone();
                let graph_refs =
                    graph_context(graph, &symbol.id, container_ref.into_iter().collect());
                let service = service_context(graph, &symbol.id);
                Some(NodeDocument {
                    id: symbol.id.clone(),
                    kind: "symbol".to_owned(),
                    name: symbol.qualified_name.clone(),
                    path: Some(evidence.path.as_str().to_owned()),
                    service: service.clone(),
                    reference: symbol.qualified_name.clone(),
                    text: format!(
                        "symbol {} path:{} service:{} context:{} {}",
                        symbol.qualified_name,
                        evidence.path,
                        service.as_deref().unwrap_or("none"),
                        graph_refs.join(" "),
                        symbol.doc.as_deref().unwrap_or("")
                    ),
                    evidence: Some(evidence),
                    graph_refs,
                })
            }
            GraphNode::Artifact(artifact) => {
                let members: Vec<String> = graph
                    .relations
                    .iter()
                    .filter(|relation| {
                        relation.kind == RelationKind::Contains && relation.source == artifact.id
                    })
                    .filter_map(|relation| node_label.get(&relation.target).cloned())
                    .collect();
                let graph_refs = graph_context(graph, &artifact.id, members);
                let evidence = artifact.evidence.clone();
                Some(NodeDocument {
                    id: artifact.id.clone(),
                    kind: "artifact".to_owned(),
                    name: artifact.path.clone(),
                    path: Some(artifact.path.clone()),
                    service: service_context(graph, &artifact.id),
                    reference: artifact.path.clone(),
                    text: format!(
                        "artifact {} context:{}",
                        artifact.path,
                        graph_refs.join(" ")
                    ),
                    evidence: Some(evidence),
                    graph_refs,
                })
            }
            GraphNode::Documentation(doc) => Some(NodeDocument {
                id: doc.id.clone(),
                kind: "documentation".to_owned(),
                name: doc.title.clone(),
                path: Some(doc.evidence.path.as_str().to_owned()),
                service: service_context(graph, &doc.id),
                reference: doc.title.clone(),
                text: format!("documentation {}", doc.title),
                evidence: Some(doc.evidence.clone()),
                graph_refs: graph_context(graph, &doc.id, Vec::new()),
            }),
            GraphNode::Config(config) => Some(NodeDocument {
                id: config.id.clone(),
                kind: "config".to_owned(),
                name: config.name.clone(),
                path: Some(config.evidence.path.as_str().to_owned()),
                service: service_context(graph, &config.id),
                reference: config.name.clone(),
                text: format!("config {} path:{}", config.name, config.evidence.path),
                evidence: Some(config.evidence.clone()),
                graph_refs: graph_context(graph, &config.id, Vec::new()),
            }),
            GraphNode::EnvVar(env) => Some(NodeDocument {
                id: env.id.clone(),
                kind: "environment".to_owned(),
                name: env.name.clone(),
                path: None,
                service: service_context(graph, &env.id),
                reference: env.name.clone(),
                text: format!(
                    "environment {} context:{}",
                    env.name,
                    graph_context(graph, &env.id, Vec::new()).join(" ")
                ),
                evidence: graph
                    .relations
                    .iter()
                    .flat_map(|relation| relation.evidence.iter())
                    .next()
                    .cloned(),
                graph_refs: graph_context(graph, &env.id, Vec::new()),
            }),
            _ => None,
        })
        .collect();
    documents.sort_by(|a, b| a.id.cmp(&b.id));
    documents
}

fn graph_context(graph: &Graph, node_id: &GraphNodeId, mut existing: Vec<String>) -> Vec<String> {
    for relation in graph
        .relations
        .iter()
        .filter(|relation| relation.source == *node_id || relation.target == *node_id)
    {
        let neighbor = if relation.source == *node_id {
            &relation.target
        } else {
            &relation.source
        };
        existing.push(format!("{:?}:{}", relation.kind, neighbor));
    }
    existing.sort();
    existing.dedup();
    existing.truncate(12);
    existing
}

fn service_context(graph: &Graph, node_id: &GraphNodeId) -> Option<String> {
    graph.relations.iter().find_map(|relation| {
        if relation.kind != RelationKind::Contains || relation.target != *node_id {
            return None;
        }
        graph.nodes.iter().find_map(|node| match node {
            GraphNode::Config(config)
                if config.id == relation.source
                    && matches!(
                        config.kind,
                        crate::graph::ConfigNodeKind::Service | crate::graph::ConfigNodeKind::Job
                    ) =>
            {
                Some(config.name.clone())
            }
            _ => None,
        })
    })
}

fn node_display_name(node: &GraphNode) -> String {
    match node {
        GraphNode::Artifact(node) => node.path.clone(),
        GraphNode::Rationale(node) => node.text.clone(),
        GraphNode::Symbol(node) => node.qualified_name.clone(),
        GraphNode::Config(node) => node.name.clone(),
        GraphNode::Documentation(node) => node.title.clone(),
        GraphNode::Container(node) => node.reference.clone(),
        GraphNode::Command(node) => node.text.clone(),
        GraphNode::EnvVar(node) => node.name.clone(),
        GraphNode::Module(node) => node.path.clone(),
        GraphNode::Package(node) => node.name.clone(),
        GraphNode::Unresolved(node) => node.value.clone(),
    }
}

fn degree_index(graph: &Graph) -> BTreeMap<GraphNodeId, u32> {
    let mut degree: BTreeMap<GraphNodeId, u32> = BTreeMap::new();
    for relation in &graph.relations {
        *degree.entry(relation.source.clone()).or_insert(0) += 1;
        *degree.entry(relation.target.clone()).or_insert(0) += 1;
    }
    degree
}

#[cfg(test)]
mod tests {
    use super::{
        EmbeddingError, EmbeddingIndex, EmbeddingProvider, MockEmbeddingProvider, SemanticSearch,
        SemanticSearchWeights,
    };
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_graph() -> Result<crate::graph::Graph, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        Ok(GraphBuilder.build(&root, &artifacts))
    }

    /// LIT-22.4.4 AC1/AC3: the mock provider is deterministic (repeated
    /// calls are byte-identical) and needs no network.
    #[test]
    fn mock_embeddings_are_deterministic() -> Result<(), EmbeddingError> {
        let provider = MockEmbeddingProvider;
        let first = provider.embed("RouteService handles routing")?;
        let second = provider.embed("RouteService handles routing")?;
        assert_eq!(first, second);
        Ok(())
    }

    /// LIT-22.4.4 AC1: texts sharing more tokens embed closer together
    /// (higher cosine similarity) than texts sharing none.
    #[test]
    fn mock_embeddings_reflect_lexical_overlap() -> Result<(), EmbeddingError> {
        let provider = MockEmbeddingProvider;
        let a = provider.embed("route service handles requests")?;
        let b = provider.embed("route service handles responses")?;
        let c = provider.embed("completely unrelated banana smoothie recipe")?;

        let similarity =
            |x: &[f32], y: &[f32]| -> f32 { x.iter().zip(y).map(|(p, q)| p * q).sum() };
        assert!(similarity(&a, &b) > similarity(&a, &c));
        Ok(())
    }

    #[test]
    fn embedding_index_is_versioned_cacheable_and_source_invalidated()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let provider = MockEmbeddingProvider;
        let index = EmbeddingIndex::build(&provider, &graph, "source-a")?;
        assert_eq!(index.version, super::EMBEDDING_INDEX_VERSION);
        assert!(index.entries.iter().all(|entry| {
            !entry.document.kind.is_empty()
                && !entry.document.name.is_empty()
                && (entry.document.path.is_some() || entry.document.kind == "environment")
        }));
        assert!(index.is_compatible(&provider, "source-a"));
        assert!(!index.is_compatible(&provider, "source-b"));

        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("embeddings.json");
        assert!(index.save_if_changed(&path)?);
        assert!(!index.save_if_changed(&path)?);
        let loaded = EmbeddingIndex::load(&path)?.ok_or("missing embedding index")?;
        assert_eq!(loaded, index);
        Ok(())
    }

    #[cfg(feature = "local-embeddings")]
    #[test]
    fn local_embedding_backend_is_explicit_and_deterministic() -> Result<(), EmbeddingError> {
        let provider = super::LocalEmbeddingProvider;
        assert_eq!(provider.model_identity(), "local-hash-v1");
        assert_eq!(
            provider.embed("local route")?,
            provider.embed("local route")?
        );
        Ok(())
    }

    /// LIT-22.4.4 AC2/AC4: search results combine a vector score with a
    /// graph connectivity score, and carry evidence plus graph refs.
    #[test]
    fn search_blends_vector_and_graph_scores_with_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;

        let results = SemanticSearch.search(
            &MockEmbeddingProvider,
            &graph,
            "RouteService route handling",
            10,
            SemanticSearchWeights::default(),
        )?;

        assert!(!results.is_empty());
        for result in &results {
            assert!(
                (result.combined_score - (0.7 * result.vector_score + 0.3 * result.graph_score))
                    .abs()
                    < 1e-9
            );
        }
        assert!(results.iter().any(|result| result.evidence.is_some()));
        assert!(
            results
                .iter()
                .any(|result| result.reference.contains("RouteService"))
        );

        Ok(())
    }

    /// LIT-22.4.4 AC2: weighting entirely toward the graph score still
    /// produces a valid, deterministically-ordered ranking.
    #[test]
    fn graph_only_weighting_still_ranks_deterministically() -> Result<(), Box<dyn std::error::Error>>
    {
        let graph = fixture_graph()?;
        let weights = SemanticSearchWeights {
            vector: 0.0,
            graph: 1.0,
        };

        let first = SemanticSearch.search(&MockEmbeddingProvider, &graph, "service", 5, weights)?;
        let second =
            SemanticSearch.search(&MockEmbeddingProvider, &graph, "service", 5, weights)?;

        assert_eq!(first, second);
        Ok(())
    }

    /// LIT-22.4.4 AC3: an empty query returns no results without ever
    /// calling the embedding provider on nothing meaningful.
    #[test]
    fn empty_query_returns_no_results() -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;

        let results = SemanticSearch.search(
            &MockEmbeddingProvider,
            &graph,
            "   ",
            10,
            SemanticSearchWeights::default(),
        )?;

        assert!(results.is_empty());
        Ok(())
    }
}
