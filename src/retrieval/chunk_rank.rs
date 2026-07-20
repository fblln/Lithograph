//! Graph-constrained hybrid ranking of enriched chunks (LIT-86.5).
//!
//! Takes vector-scored [`EnrichedChunk`] candidates and applies hard graph
//! filters, bounded graph expansion, and a versioned deterministic scoring
//! contract that blends the vector score with graph-derived features. Graph
//! *degree* is deliberately never a scoring feature: a highly-connected but
//! unrelated node cannot outrank a relevant scoped candidate on connectivity
//! alone (AC#4). Every result carries its per-feature contributions and an
//! explanation path, and unsatisfiable or over-broad constraints surface as
//! diagnostics rather than silently dropping filters.

// ponytail: consumed by the search surface (LIT-86.6). Drop this allow at first
// production wiring.
#![allow(dead_code)]

use crate::domain::EvidenceRef;
use crate::graph::{Graph, GraphNodeId, RelationKind};
use crate::retrieval::chunk_enrich::{Direction, EnrichedChunk};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Version of the deterministic scoring contract. Bump when feature math or
/// blending changes so callers can detect a scoring migration.
pub(crate) const CHUNK_SCORING_VERSION: u32 = 1;

/// Blend weights for the scoring contract (AC#3). No `degree` weight exists by
/// construction (AC#4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct RankWeights {
    /// Weight on the vector similarity score.
    pub vector: f64,
    /// Weight on the optional lexical score.
    pub lexical: f64,
    /// Weight on containment / evidence proximity to scoped nodes.
    pub containment: f64,
    /// Weight on closeness in the bounded graph expansion.
    pub graph_distance: f64,
    /// Weight on how many neighbors match the requested relation kinds.
    pub relation: f64,
    /// Weight on a constant configurable prior.
    pub prior: f64,
}

impl Default for RankWeights {
    fn default() -> Self {
        Self {
            vector: 0.6,
            lexical: 0.1,
            containment: 0.1,
            graph_distance: 0.1,
            relation: 0.1,
            prior: 0.0,
        }
    }
}

/// A tag expression: every tag in `all` must be present, and at least one tag
/// in `any` (when `any` is non-empty) must be present.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TagExpr {
    /// Tags that must all be present.
    pub all: Vec<String>,
    /// Tags of which at least one must be present (when non-empty).
    pub any: Vec<String>,
}

impl TagExpr {
    fn is_empty(&self) -> bool {
        self.all.is_empty() && self.any.is_empty()
    }

    fn matches(&self, tags: &[String]) -> bool {
        let has = |needle: &String| tags.iter().any(|tag| tag == needle);
        self.all.iter().all(&has) && (self.any.is_empty() || self.any.iter().any(&has))
    }
}

/// Hard filters; a candidate must satisfy every present filter (AC#1).
#[derive(Debug, Clone, Default)]
pub(crate) struct RankFilters {
    /// Glob over the artifact path.
    pub path_glob: Option<String>,
    /// Exact language id.
    pub language: Option<String>,
    /// Owning module node id.
    pub module_id: Option<GraphNodeId>,
    /// Owning package node id.
    pub package_id: Option<GraphNodeId>,
    /// Owning service name.
    pub service: Option<String>,
    /// A symbol or graph node id that must be attached to the chunk.
    pub node_id: Option<GraphNodeId>,
    /// Architecture layer.
    pub layer: Option<String>,
    /// Tag expression.
    pub tags: TagExpr,
}

impl RankFilters {
    fn is_empty(&self) -> bool {
        self.path_glob.is_none()
            && self.language.is_none()
            && self.module_id.is_none()
            && self.package_id.is_none()
            && self.service.is_none()
            && self.node_id.is_none()
            && self.layer.is_none()
            && self.tags.is_empty()
    }
}

/// Bounded graph expansion from an anchor node (AC#2). When active, only chunks
/// whose symbols fall within the hop-bounded closure are admitted.
#[derive(Debug, Clone, Default)]
pub(crate) struct Expansion {
    /// Maximum hops from the anchor; `0` disables expansion.
    pub max_hops: usize,
    /// Relation kinds the walk may follow (empty = any kind).
    pub relations: Vec<RelationKind>,
    /// Direction the walk may follow (`None` = both).
    pub direction: Option<Direction>,
}

impl Expansion {
    fn is_active(&self) -> bool {
        self.max_hops > 0
    }
}

/// One vector-scored candidate to rank.
#[derive(Debug, Clone)]
pub(crate) struct Candidate {
    /// The enriched chunk.
    pub enriched: EnrichedChunk,
    /// Cosine vector score in `[-1, 1]`.
    pub vector_score: f64,
    /// Optional lexical score in `[0, 1]` (`0` when lexical ranking is off).
    pub lexical_score: f64,
    /// Language id of the chunk's artifact.
    pub language: String,
}

/// The complete query: filters, expansion, and weights.
#[derive(Debug, Clone, Default)]
pub(crate) struct RankQuery {
    /// Hard filters.
    pub filters: RankFilters,
    /// Bounded graph expansion.
    pub expansion: Expansion,
    /// Blend weights (defaulted when constructed via `Default`).
    pub weights: Option<RankWeights>,
}

/// Per-feature contributions to a result's final score (AC#5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct FeatureContribution {
    /// Vector similarity feature (pre-weight).
    pub vector: f64,
    /// Lexical feature (pre-weight).
    pub lexical: f64,
    /// Containment / evidence-proximity feature (pre-weight).
    pub containment: f64,
    /// Graph-distance feature (pre-weight).
    pub graph_distance: f64,
    /// Relation-relevance feature (pre-weight).
    pub relation: f64,
    /// Prior feature (pre-weight).
    pub prior: f64,
    /// Weighted sum.
    pub final_score: f64,
}

/// One ranked result with its full explanation (AC#5).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RankedResult {
    /// Chunk identity.
    pub chunk_id: String,
    /// Feature breakdown and final score.
    pub features: FeatureContribution,
    /// Evidence span back to source bytes.
    pub evidence: EvidenceRef,
    /// Attached symbol node ids.
    pub symbol_ids: Vec<GraphNodeId>,
    /// Owning module, if any.
    pub module_id: Option<GraphNodeId>,
    /// Owning service, if any.
    pub service: Option<String>,
    /// Human-readable explanation steps (matched filters, expansion, features).
    pub explanation: Vec<String>,
}

/// Non-fatal diagnostics about the constraints (AC#7).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct RankDiagnostics {
    /// Documented warnings (over-broad, unknown constraint, contradictory).
    pub warnings: Vec<String>,
}

/// A fatal problem with the query (AC#7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RankError {
    /// No candidates were supplied to rank.
    NoCandidates,
    /// A path glob failed to compile.
    InvalidGlob(String),
}

impl std::fmt::Display for RankError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCandidates => formatter.write_str("no candidates to rank"),
            Self::InvalidGlob(pattern) => write!(formatter, "invalid path glob: {pattern}"),
        }
    }
}

impl std::error::Error for RankError {}

/// Ranks `candidates` for `query` against `graph`. Returns results in
/// deterministic descending score order plus any constraint diagnostics.
pub(crate) fn rank(
    graph: &Graph,
    query: &RankQuery,
    candidates: &[Candidate],
) -> Result<(Vec<RankedResult>, RankDiagnostics), RankError> {
    if candidates.is_empty() {
        return Err(RankError::NoCandidates);
    }
    let weights = query.weights.unwrap_or_default();
    let mut diagnostics = RankDiagnostics::default();

    // Compile the path glob once (AC#7: a bad glob is a fatal, documented error,
    // not a silently dropped filter).
    let path_matcher = match &query.filters.path_glob {
        Some(pattern) => Some(
            globset::Glob::new(pattern)
                .map_err(|_| RankError::InvalidGlob(pattern.clone()))?
                .compile_matcher(),
        ),
        None => None,
    };

    // Warn on constraints that name nodes absent from the graph, rather than
    // dropping them silently (AC#7).
    for (label, id) in [
        ("module", query.filters.module_id.as_ref()),
        ("package", query.filters.package_id.as_ref()),
        ("node", query.filters.node_id.as_ref()),
    ] {
        if let Some(id) = id
            && !graph.nodes.iter().any(|node| node.id() == id)
        {
            diagnostics.warnings.push(format!(
                "unknown {label} constraint: {id} is not in the graph"
            ));
        }
    }
    if query.filters.is_empty() && !query.expansion.is_active() {
        diagnostics
            .warnings
            .push("over-broad query: no filters or expansion; ranking all candidates".to_owned());
    }

    // Bounded expansion closure (hop distances from the anchor node).
    let scope = if query.expansion.is_active() {
        query
            .filters
            .node_id
            .as_ref()
            .map(|anchor| expansion_hops(graph, anchor, &query.expansion))
    } else {
        None
    };

    // When expansion is active, `node_id` is the expansion anchor and admits
    // chunks by hop-distance (hop 0 = chunks containing the anchor), so it is
    // not also enforced as a containment hard-filter.
    let enforce_node_filter = !query.expansion.is_active();

    let mut results: Vec<RankedResult> = Vec::new();
    for candidate in candidates {
        if let Some(reason) = fails_filters(
            candidate,
            &query.filters,
            path_matcher.as_ref(),
            enforce_node_filter,
        ) {
            let _ = reason;
            continue;
        }
        // When expansion is active with a resolvable anchor, a candidate must
        // have at least one symbol inside the closure (AC#2: no out-of-scope
        // admittance).
        let hop = match &scope {
            Some(hops) => {
                let nearest = candidate
                    .enriched
                    .symbol_ids
                    .iter()
                    .filter_map(|id| hops.get(id).copied())
                    .min();
                match nearest {
                    Some(hop) => Some(hop),
                    None => continue,
                }
            }
            None => None,
        };
        results.push(score_candidate(candidate, query, &weights, hop));
    }

    // Contradictory: non-empty constraints that jointly admit nothing, even
    // though candidates existed (AC#7).
    if results.is_empty() && (!query.filters.is_empty() || query.expansion.is_active()) {
        diagnostics
            .warnings
            .push("contradictory or unsatisfiable constraints: no candidate matched".to_owned());
    }

    // Deterministic order: score desc, then chunk id asc (AC#6).
    results.sort_by(|a, b| {
        b.features
            .final_score
            .partial_cmp(&a.features.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.chunk_id.cmp(&b.chunk_id))
    });
    Ok((results, diagnostics))
}

/// Returns a reason string when `candidate` fails any present filter, else
/// `None`.
fn fails_filters(
    candidate: &Candidate,
    filters: &RankFilters,
    path_matcher: Option<&globset::GlobMatcher>,
    enforce_node_filter: bool,
) -> Option<String> {
    let enriched = &candidate.enriched;
    if let Some(matcher) = path_matcher
        && !matcher.is_match(enriched.evidence.path.as_str())
    {
        return Some("path".to_owned());
    }
    if let Some(language) = &filters.language
        && &candidate.language != language
    {
        return Some("language".to_owned());
    }
    if let Some(module) = &filters.module_id
        && enriched.module_id.as_ref() != Some(module)
    {
        return Some("module".to_owned());
    }
    if let Some(package) = &filters.package_id
        && enriched.package_id.as_ref() != Some(package)
    {
        return Some("package".to_owned());
    }
    if let Some(service) = &filters.service
        && enriched.service.as_ref() != Some(service)
    {
        return Some("service".to_owned());
    }
    if let Some(layer) = &filters.layer
        && enriched.layer.as_ref() != Some(layer)
    {
        return Some("layer".to_owned());
    }
    if enforce_node_filter
        && let Some(node) = &filters.node_id
        && !enriched.symbol_ids.contains(node)
    {
        return Some("node".to_owned());
    }
    if !filters.tags.matches(&enriched.tags) {
        return Some("tags".to_owned());
    }
    None
}

/// Computes the blended score and explanation for one surviving candidate.
fn score_candidate(
    candidate: &Candidate,
    query: &RankQuery,
    weights: &RankWeights,
    hop: Option<usize>,
) -> RankedResult {
    let enriched = &candidate.enriched;
    let mut explanation = Vec::new();

    // Vector feature, clamped to [0, 1] so a negative cosine cannot subtract.
    let vector = candidate.vector_score.clamp(0.0, 1.0);
    let lexical = candidate.lexical_score.clamp(0.0, 1.0);

    // Containment/evidence proximity: an explicit node filter that matched is
    // full containment; otherwise a mild proxy from how many symbols the chunk
    // pins (bounded to 1). This rewards evidence-tight chunks, never degree.
    let containment = if query
        .filters
        .node_id
        .as_ref()
        .is_some_and(|node| enriched.symbol_ids.contains(node))
    {
        explanation.push("contains the requested node".to_owned());
        1.0
    } else {
        (enriched.symbol_ids.len() as f64 / 4.0).min(1.0)
    };

    // Graph distance: closer to the expansion anchor scores higher.
    let graph_distance = match hop {
        Some(hop) => {
            explanation.push(format!("within {hop} hop(s) of the expansion anchor"));
            1.0 / (1.0 + hop as f64)
        }
        None => 0.0,
    };

    // Relation relevance: fraction of neighbors whose relation is one the query
    // asked to expand along (or all neighbors when no kinds were named).
    let relation = if enriched.neighbors.is_empty() {
        0.0
    } else if query.expansion.relations.is_empty() {
        1.0
    } else {
        let matching = enriched
            .neighbors
            .iter()
            .filter(|neighbor| query.expansion.relations.contains(&neighbor.relation))
            .count();
        matching as f64 / enriched.neighbors.len() as f64
    };

    let prior = 0.0;
    let final_score = weights.vector * vector
        + weights.lexical * lexical
        + weights.containment * containment
        + weights.graph_distance * graph_distance
        + weights.relation * relation
        + weights.prior * prior;

    explanation.push(format!(
        "score {final_score:.4} = v{:.2}*{:.2} + c{:.2}*{:.2} + g{:.2}*{:.2} + r{:.2}*{:.2}",
        weights.vector,
        vector,
        weights.containment,
        containment,
        weights.graph_distance,
        graph_distance,
        weights.relation,
        relation
    ));

    RankedResult {
        chunk_id: enriched.chunk_id.clone(),
        features: FeatureContribution {
            vector,
            lexical,
            containment,
            graph_distance,
            relation,
            prior,
            final_score,
        },
        evidence: enriched.evidence.clone(),
        symbol_ids: enriched.symbol_ids.clone(),
        module_id: enriched.module_id.clone(),
        service: enriched.service.clone(),
        explanation,
    }
}

/// Breadth-first hop distances from `anchor`, following only the requested
/// relation kinds and direction, out to `max_hops`. The returned map is exactly
/// the admitted scope; nothing beyond `max_hops` is included (AC#2).
fn expansion_hops(
    graph: &Graph,
    anchor: &GraphNodeId,
    expansion: &Expansion,
) -> BTreeMap<GraphNodeId, usize> {
    let mut hops: BTreeMap<GraphNodeId, usize> = BTreeMap::new();
    hops.insert(anchor.clone(), 0);
    let mut frontier: VecDeque<(GraphNodeId, usize)> = VecDeque::new();
    frontier.push_back((anchor.clone(), 0));
    let mut visited: BTreeSet<GraphNodeId> = BTreeSet::new();
    visited.insert(anchor.clone());

    while let Some((node, depth)) = frontier.pop_front() {
        if depth >= expansion.max_hops {
            continue;
        }
        for relation in &graph.relations {
            let kind_ok =
                expansion.relations.is_empty() || expansion.relations.contains(&relation.kind);
            if !kind_ok {
                continue;
            }
            // Follow an edge only in an allowed direction relative to `node`.
            let next = if relation.source == node
                && matches!(expansion.direction, None | Some(Direction::Outgoing))
            {
                Some(relation.target.clone())
            } else if relation.target == node
                && matches!(expansion.direction, None | Some(Direction::Incoming))
            {
                Some(relation.source.clone())
            } else {
                None
            };
            if let Some(next) = next
                && visited.insert(next.clone())
            {
                hops.insert(next.clone(), depth + 1);
                frontier.push_back((next, depth + 1));
            }
        }
    }
    hops
}

#[cfg(test)]
mod tests {
    use super::{
        Candidate, Expansion, RankError, RankFilters, RankQuery, RankWeights, TagExpr, rank,
    };
    use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath, SourceSpan};
    use crate::graph::{Graph, GraphNodeId, Relation, RelationKind};
    use crate::retrieval::chunk_enrich::{Direction, EnrichedChunk, GraphNeighbor};

    type TestResult<T> = Result<T, Box<dyn std::error::Error>>;

    fn evidence(path: &str, start: u32, end: u32) -> TestResult<EvidenceRef> {
        let repo = RepoPath::new(path)?;
        let id = ArtifactId::from_path(&repo);
        Ok(EvidenceRef::file(id, repo).with_span(SourceSpan::new(start, end)?))
    }

    fn candidate(
        chunk_id: &str,
        path: &str,
        vector: f64,
        symbols: &[&str],
        module: Option<&str>,
        service: Option<&str>,
    ) -> TestResult<Candidate> {
        Ok(Candidate {
            enriched: EnrichedChunk {
                chunk_id: chunk_id.to_owned(),
                artifact_id: ArtifactId::from_path(&RepoPath::new(path)?),
                evidence: evidence(path, 1, 5)?,
                symbol_ids: symbols.iter().map(|s| GraphNodeId::new(*s)).collect(),
                module_id: module.map(GraphNodeId::new),
                package_id: None,
                service: service.map(str::to_owned),
                layer: None,
                tags: Vec::new(),
                neighbors: Vec::new(),
            },
            vector_score: vector,
            lexical_score: 0.0,
            language: "rust".to_owned(),
        })
    }

    fn relation(id: &str, source: &str, target: &str, kind: RelationKind) -> Relation {
        Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence: Confidence::High,
            evidence: Vec::new(),
            provenance: None,
        }
    }

    #[test]
    fn no_candidates_is_an_error() {
        assert!(matches!(
            rank(&Graph::default(), &RankQuery::default(), &[]),
            Err(RankError::NoCandidates)
        ));
    }

    #[test]
    fn service_filter_keeps_only_matching_candidates() -> Result<(), Box<dyn std::error::Error>> {
        let cands = vec![
            candidate("chunk:a#0", "a.rs", 0.9, &["symbol:a"], None, Some("web"))?,
            candidate(
                "chunk:b#0",
                "b.rs",
                0.95,
                &["symbol:b"],
                None,
                Some("worker"),
            )?,
        ];
        let query = RankQuery {
            filters: RankFilters {
                service: Some("web".to_owned()),
                ..RankFilters::default()
            },
            ..RankQuery::default()
        };
        let (results, _) = rank(&Graph::default(), &query, &cands)?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "chunk:a#0");
        Ok(())
    }

    /// AC#4: a high-degree, high-vector but out-of-scope candidate does not
    /// outrank a relevant in-scope one when the query is scoped by expansion.
    #[test]
    fn degree_does_not_beat_relevant_scope() -> Result<(), Box<dyn std::error::Error>> {
        // Graph: anchor calls helper (1 hop). unrelated has no path to anchor.
        let graph = Graph {
            nodes: vec![],
            relations: vec![relation(
                "r1",
                "symbol:anchor",
                "symbol:helper",
                RelationKind::Calls,
            )],
        };
        let mut in_scope = candidate(
            "chunk:helper#0",
            "h.rs",
            0.30,
            &["symbol:helper"],
            None,
            None,
        )?;
        // Give the unrelated candidate a much higher vector score AND many
        // neighbors (high degree) to prove neither wins it the top slot.
        let mut unrelated = candidate(
            "chunk:unrelated#0",
            "u.rs",
            0.99,
            &["symbol:unrelated"],
            None,
            None,
        )?;
        unrelated.enriched.neighbors = (0..20)
            .map(|i| GraphNeighbor {
                relation: RelationKind::Calls,
                direction: Direction::Outgoing,
                node: GraphNodeId::new(format!("symbol:x{i}")),
            })
            .collect();
        in_scope.enriched.neighbors = vec![GraphNeighbor {
            relation: RelationKind::Calls,
            direction: Direction::Outgoing,
            node: GraphNodeId::new("symbol:anchor"),
        }];
        unrelated.language = "rust".to_owned();

        let query = RankQuery {
            filters: RankFilters {
                node_id: Some(GraphNodeId::new("symbol:anchor")),
                ..RankFilters::default()
            },
            expansion: Expansion {
                max_hops: 2,
                relations: vec![RelationKind::Calls],
                direction: None,
            },
            ..RankQuery::default()
        };
        let (results, _) = rank(&graph, &query, &[unrelated, in_scope])?;
        // Only the in-scope candidate is admitted; the unrelated one is out of
        // the expansion closure entirely (AC#2).
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "chunk:helper#0");
        Ok(())
    }

    /// AC#6: identical inputs produce byte-identical ordering.
    #[test]
    fn ranking_is_deterministic() -> Result<(), Box<dyn std::error::Error>> {
        let cands = vec![
            candidate("chunk:a#0", "a.rs", 0.5, &["symbol:a"], None, None)?,
            candidate("chunk:b#0", "b.rs", 0.5, &["symbol:b"], None, None)?,
            candidate("chunk:c#0", "c.rs", 0.5, &["symbol:c"], None, None)?,
        ];
        let first = rank(&Graph::default(), &RankQuery::default(), &cands)?;
        let second = rank(&Graph::default(), &RankQuery::default(), &cands)?;
        assert_eq!(first.0, second.0);
        // Equal scores tie-break by chunk id ascending.
        let ids: Vec<&str> = first.0.iter().map(|r| r.chunk_id.as_str()).collect();
        assert_eq!(ids, ["chunk:a#0", "chunk:b#0", "chunk:c#0"]);
        Ok(())
    }

    /// AC#7: an unknown node constraint and an over-broad query both warn
    /// instead of silently proceeding.
    #[test]
    fn diagnostics_report_unknown_and_over_broad() -> Result<(), Box<dyn std::error::Error>> {
        let cands = vec![candidate(
            "chunk:a#0",
            "a.rs",
            0.5,
            &["symbol:a"],
            None,
            None,
        )?];
        // Over-broad: no filters.
        let (_, broad) = rank(&Graph::default(), &RankQuery::default(), &cands)?;
        assert!(broad.warnings.iter().any(|w| w.contains("over-broad")));

        // Unknown module constraint (not present in an empty graph).
        let query = RankQuery {
            filters: RankFilters {
                module_id: Some(GraphNodeId::new("module:ghost")),
                ..RankFilters::default()
            },
            ..RankQuery::default()
        };
        let (_, unknown) = rank(&Graph::default(), &query, &cands)?;
        assert!(
            unknown
                .warnings
                .iter()
                .any(|w| w.contains("unknown module"))
        );
        Ok(())
    }

    /// AC#1/#5: a tag expression filters, and results carry a feature breakdown
    /// and explanation.
    #[test]
    fn tag_filter_and_feature_breakdown() -> Result<(), Box<dyn std::error::Error>> {
        let mut tagged = candidate("chunk:a#0", "a.rs", 0.8, &["symbol:a"], None, None)?;
        tagged.enriched.tags = vec!["in-service".to_owned(), "external-package".to_owned()];
        let plain = candidate("chunk:b#0", "b.rs", 0.9, &["symbol:b"], None, None)?;

        let query = RankQuery {
            filters: RankFilters {
                tags: TagExpr {
                    all: vec!["in-service".to_owned()],
                    any: vec![],
                },
                ..RankFilters::default()
            },
            ..RankQuery::default()
        };
        let (results, _) = rank(&Graph::default(), &query, &[tagged, plain])?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].chunk_id, "chunk:a#0");
        // Feature breakdown present and consistent with default weights.
        let features = &results[0].features;
        let expected = RankWeights::default().vector * features.vector
            + RankWeights::default().containment * features.containment;
        assert!((features.final_score - expected).abs() < 1e-9);
        assert!(!results[0].explanation.is_empty());
        Ok(())
    }
}
