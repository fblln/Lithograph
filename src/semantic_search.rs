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

/// Fixed embedding dimensionality used by every provider in this crate, so
/// vectors from different providers are at least comparably shaped (real
/// providers may still differ semantically, but never in size).
const EMBEDDING_DIMENSIONS: usize = 64;

/// Failure embedding a piece of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EmbeddingError {
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
pub trait EmbeddingProvider {
    /// Embeds `text` into a vector of [`EMBEDDING_DIMENSIONS`] entries.
    fn embed(&self, text: &str) -> Result<Vec<f32>, EmbeddingError>;
}

/// Deterministic, offline embedding provider (AC1/AC3): feature-hashes
/// each token into a fixed-size vector (the "hashing trick"), so texts
/// sharing more tokens land closer together by cosine similarity, with no
/// network access and no live model. Every normal test uses this, never a
/// real provider.
#[derive(Debug, Clone, Copy, Default)]
pub struct MockEmbeddingProvider;

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
}

/// Configuration for a real, OpenAI-compatible `/embeddings` endpoint
/// (AC1: optional real backend). Any server implementing that response
/// shape works (OpenAI itself, or a compatible local/hosted server).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenAiEmbeddingConfig {
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
pub struct OpenAiEmbeddingProvider {
    config: OpenAiEmbeddingConfig,
    agent: ureq::Agent,
}

impl OpenAiEmbeddingProvider {
    /// Builds a provider from `config`.
    pub fn new(config: OpenAiEmbeddingConfig) -> Self {
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

/// One document eligible for semantic search: the same node categories
/// [`crate::fts::FtsIndex`] indexes, plus real evidence and graph
/// references (AC4) that a lexical-only index doesn't need.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SemanticDocument {
    id: GraphNodeId,
    reference: String,
    text: String,
    evidence: Option<EvidenceRef>,
    graph_refs: Vec<String>,
}

/// One semantic search result (AC2/AC4): the blended score plus the
/// vector/graph components that produced it, real evidence, and related
/// graph references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticSearchResult {
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
pub struct SemanticSearchWeights {
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
pub struct SemanticSearch;

impl SemanticSearch {
    /// Embeds `query` with `provider`, then ranks every eligible graph
    /// node by a weighted blend of cosine similarity and graph
    /// connectivity (AC2), returning at most `limit` results with
    /// evidence and graph references (AC4).
    pub fn search(
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
        let query_vector = provider.embed(query)?;
        let documents = collect_documents(graph);
        let degree = degree_index(graph);
        let max_degree = degree.values().copied().max().unwrap_or(0).max(1) as f64;

        let mut scored: Vec<SemanticSearchResult> = documents
            .iter()
            .map(|document| {
                let document_vector = provider.embed(&document.text)?;
                let vector_score = cosine_similarity(&query_vector, &document_vector);
                let graph_score = f64::from(*degree.get(&document.id).unwrap_or(&0)) / max_degree;
                let combined_score = weights.vector * vector_score + weights.graph * graph_score;
                Ok(SemanticSearchResult {
                    document_id: document.id.as_str().to_owned(),
                    reference: document.reference.clone(),
                    evidence: document.evidence.clone(),
                    graph_refs: document.graph_refs.clone(),
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

fn collect_documents(graph: &Graph) -> Vec<SemanticDocument> {
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

    let mut documents: Vec<SemanticDocument> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Symbol(symbol) => {
                let container_ref = contained_by
                    .get(&symbol.id)
                    .and_then(|id| node_label.get(id))
                    .cloned();
                Some(SemanticDocument {
                    id: symbol.id.clone(),
                    reference: symbol.qualified_name.clone(),
                    text: format!(
                        "{} {}",
                        symbol.qualified_name,
                        symbol.doc.as_deref().unwrap_or("")
                    ),
                    evidence: Some(symbol.evidence.clone()),
                    graph_refs: container_ref.into_iter().collect(),
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
                Some(SemanticDocument {
                    id: artifact.id.clone(),
                    reference: artifact.path.clone(),
                    text: artifact.path.clone(),
                    evidence: Some(artifact.evidence.clone()),
                    graph_refs: members,
                })
            }
            GraphNode::Documentation(doc) => Some(SemanticDocument {
                id: doc.id.clone(),
                reference: doc.title.clone(),
                text: doc.title.clone(),
                evidence: Some(doc.evidence.clone()),
                graph_refs: Vec::new(),
            }),
            _ => None,
        })
        .collect();
    documents.sort_by(|a, b| a.id.cmp(&b.id));
    documents
}

fn node_display_name(node: &GraphNode) -> String {
    match node {
        GraphNode::Artifact(node) => node.path.clone(),
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
        EmbeddingError, EmbeddingProvider, MockEmbeddingProvider, SemanticSearch,
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
