use super::clone_tokens::{body_content_hash, clone_set_identity, jaccard_similarity, word_tokens};
use super::*;

// Read by `mod.rs`'s `build_with_optional_trace` to record per-pass
// diagnostics, so the struct and every field are `pub(super)` (LIT-41.1
// move-only split): visible throughout the `graph::builder` tree, no wider.
#[derive(Debug, Default)]
pub(super) struct CloneDiagnostics {
    pub(super) candidate_count: u64,
    pub(super) comparison_count: u64,
    pub(super) emitted_count: u64,
    pub(super) rejected_near_threshold_count: u64,
    pub(super) pruned_count: u64,
    // Peak number of candidate pairs held in memory at once (LIT-35.1 AC4):
    // the compact Vec<(u32, u32)> that replaced the old BTreeSet pair tree.
    pub(super) peak_candidate_pairs: u64,
    // Deduplicated in-band candidate pairs produced by the bounded prefix-index
    // traversal (LIT-35.2 AC4, LIT-38.1 AC1). Because the length/size-band bound
    // is applied while walking postings, out-of-band shared-token pairs are
    // never generated, so this is far smaller than the pre-LIT-38.1 count that
    // materialized every co-occurring prefix pair before filtering.
    pub(super) prefilter_pairs: u64,
    // Sub-phase wall-clock costs in microseconds (LIT-35.1 AC4). Reported as
    // component durations so the clone bottleneck can be attributed to
    // tokenization vs candidate generation vs exact verification.
    pub(super) tokenize_us: u64,
    pub(super) candidate_gen_us: u64,
    pub(super) exact_verify_us: u64,
    // Cache-lookup and deterministic-merge sub-phase costs (LIT-35.5 AC1), so
    // the five-sample report carries median/MAD for every clone phase.
    pub(super) cache_lookup_us: u64,
    pub(super) merge_us: u64,
    // 1 when the versioned clone snapshot was reused, 0 when detection ran
    // (miss/invalidation/uncacheable) (LIT-35.3 AC4).
    pub(super) cache_hit: u64,
    pub(super) decisions: Vec<GraphDecisionTrace>,
}

/// Bump when the near-clone algorithm or emitted-relation semantics change in a
/// way that must invalidate persisted clone snapshots (LIT-35.3 AC1). Part of
/// the snapshot identity and re-validated on read.
pub(super) const CLONE_ALGORITHM_VERSION: u32 = 1;

/// One emitted `SimilarTo` relation, stored source-free so a warm rebuild can
/// replay it without re-running exact verification (LIT-35.3). `left`/`right`
/// evidence preserve the exact `vec![left, right]` order the live path passes
/// to `relate_with_provenance`, independent of the `source`/`target` ordering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CloneSnapshotRelation {
    source: String,
    target: String,
    confidence: Confidence,
    left_evidence: EvidenceRef,
    right_evidence: EvidenceRef,
}

/// Persisted result of a near-clone pass over one canonical candidate set
/// (LIT-35.3). Contains only what a hit must reproduce byte-for-byte: the
/// emitted relations in emission order plus the deterministic counters. No
/// source text, absolute paths, or timings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct CloneSnapshot {
    algorithm_version: u32,
    relations: Vec<CloneSnapshotRelation>,
    candidate_count: u64,
    comparison_count: u64,
    emitted_count: u64,
    rejected_near_threshold_count: u64,
    pruned_count: u64,
    peak_candidate_pairs: u64,
    prefilter_pairs: u64,
}

#[derive(Debug)]
struct CloneCandidate {
    id: GraphNodeId,
    evidence: EvidenceRef,
    // Sorted, unique interned token IDs (LIT-35.1 AC1). Interning removes the
    // String-heavy ordered-set work from the pairwise Jaccard hot loop while
    // keeping exact set semantics: equal IDs iff equal token strings, so
    // intersection/union sizes -- and thus the Jaccard score -- are identical
    // to the previous BTreeSet<String> representation.
    tokens: Vec<u32>,
    language: String,
    size_band: u32,
}

impl BuilderState {
    /// Emits `SimilarTo` relations between near-clone function/method pairs
    /// (LIT-22.3.6 AC2): deterministic Jaccard similarity over each
    /// symbol's lowercase word-token bag, read from its own evidence span
    /// -- never live embeddings or any external ranking service (AC3;
    /// semantic ranking stays a separate, later search concern).
    // ponytail: O(n^2) pairwise comparison over every function/method
    // symbol in the repo. Fine for a "minimum deterministic" baseline at
    // the scale this tool targets; if a very large repo makes this slow,
    // bucket candidates by token-count or line-count range first.
    pub(super) fn detect_near_clones(
        &mut self,
        repo_root: &Path,
        detail: &GraphBuildTraceDetail,
        selectors: &[String],
        cache: Option<&AnalysisCache>,
    ) -> CloneDiagnostics {
        const MIN_BODY_LINES: u32 = 3;
        const SIMILAR_THRESHOLD: f64 = 0.6;
        const TRACE_NEAR_THRESHOLD: f64 = 0.35;
        const HIGH_CONFIDENCE_THRESHOLD: f64 = 0.85;

        let tokenize_started = Instant::now();
        // Per-build token dictionary: each distinct word token gets a stable
        // u32 ID in first-seen order (LIT-35.1 AC1). Only equality of IDs
        // matters for Jaccard, so any consistent assignment preserves scores.
        let mut interner: HashMap<String, u32> = HashMap::new();
        let mut file_cache: BTreeMap<String, Option<String>> = BTreeMap::new();
        let mut candidates = Vec::new();
        // Per-candidate identity descriptors for the clone snapshot cache
        // (LIT-35.3): node id, evidence path, span, language, and a content
        // hash of the exact body lines. Any add/remove/move/modify changes one
        // descriptor and therefore the whole-set identity.
        let mut identity_parts: Vec<String> = Vec::new();
        for node in self.nodes.values() {
            let GraphNode::Symbol(symbol) = node else {
                continue;
            };
            if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
                continue;
            }
            let Some(span) = &symbol.evidence.span else {
                continue;
            };
            if span.end_line.saturating_sub(span.start_line) + 1 < MIN_BODY_LINES {
                continue;
            }
            let path = symbol.evidence.path.as_str().to_owned();
            let text = file_cache
                .entry(path.clone())
                .or_insert_with(|| fs::read_to_string(repo_root.join(&path)).ok());
            let Some(text) = text else {
                continue;
            };
            let token_strings = word_tokens(text, span.start_line, span.end_line);
            if token_strings.is_empty() {
                continue;
            }
            let body_hash = body_content_hash(text, span.start_line, span.end_line);
            // Intern to IDs and sort so the pairwise loop is a linear
            // two-pointer scan. The source set is already unique.
            let mut tokens: Vec<u32> = token_strings
                .into_iter()
                .map(|token| {
                    let next = interner.len() as u32;
                    *interner.entry(token).or_insert(next)
                })
                .collect();
            tokens.sort_unstable();
            let language = Path::new(symbol.evidence.path.as_str())
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or("unknown")
                .to_ascii_lowercase();
            let size_band = usize::BITS - tokens.len().max(1).leading_zeros() - 1;
            identity_parts.push(format!(
                "{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}\u{1}{}",
                symbol.id.as_str(),
                path,
                span.start_line,
                span.end_line,
                language,
                body_hash,
            ));
            candidates.push(CloneCandidate {
                id: symbol.id.clone(),
                evidence: symbol.evidence.clone(),
                tokens,
                language,
                size_band,
            });
        }
        let tokenize_us = tokenize_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);

        let mut diagnostics = CloneDiagnostics {
            candidate_count: candidates.len() as u64,
            tokenize_us,
            ..CloneDiagnostics::default()
        };

        // LIT-35.3: reuse a persisted snapshot only in the Summary, no-selector
        // mode -- the only mode where per-pair decisions are always empty, so a
        // relation-only snapshot reproduces the full trace. Full/selector builds
        // (PR tier, focused diagnostics) recompute so their decisions are exact.
        let cacheable = *detail == GraphBuildTraceDetail::Summary && selectors.is_empty();
        let snapshot_path = cacheable
            .then(|| {
                cache.map(|cache| {
                    cache.clone_snapshot_path(&clone_set_identity(
                        &mut identity_parts,
                        MIN_BODY_LINES,
                        SIMILAR_THRESHOLD,
                        TRACE_NEAR_THRESHOLD,
                        HIGH_CONFIDENCE_THRESHOLD,
                    ))
                })
            })
            .flatten();
        let cache_lookup_started = Instant::now();
        let cached_snapshot = snapshot_path
            .as_ref()
            .and_then(|path| JsonStore.read::<CloneSnapshot>(path).ok().flatten())
            .filter(|snapshot| snapshot.algorithm_version == CLONE_ALGORITHM_VERSION);
        diagnostics.cache_lookup_us = cache_lookup_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        if let Some(snapshot) = cached_snapshot {
            self.apply_clone_snapshot(&snapshot);
            diagnostics.cache_hit = 1;
            diagnostics.comparison_count = snapshot.comparison_count;
            diagnostics.emitted_count = snapshot.emitted_count;
            diagnostics.rejected_near_threshold_count = snapshot.rejected_near_threshold_count;
            diagnostics.pruned_count = snapshot.pruned_count;
            diagnostics.peak_candidate_pairs = snapshot.peak_candidate_pairs;
            diagnostics.prefilter_pairs = snapshot.prefilter_pairs;
            return diagnostics;
        }
        let total_pairs = candidates
            .len()
            .saturating_mul(candidates.len().saturating_sub(1))
            / 2;
        let candidate_gen_started = Instant::now();
        // LIT-35.2: only generate pairs that share a rare token via a prefix
        // inverted index, then keep those in the size-band universe. The result
        // is the ascending, deduplicated set of pairs to verify exactly -- a
        // subset of the old band product that still contains every pair whose
        // Jaccard can reach the trace threshold, so emitted relations and
        // near-threshold decisions are byte-identical.
        let (pairs, prefilter_pairs) =
            clone_verification_pairs(&candidates, SIMILAR_THRESHOLD, TRACE_NEAR_THRESHOLD);
        diagnostics.prefilter_pairs = prefilter_pairs;
        diagnostics.peak_candidate_pairs = pairs.len() as u64;
        diagnostics.candidate_gen_us = candidate_gen_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        diagnostics.pruned_count = total_pairs.saturating_sub(pairs.len()) as u64;
        let exact_verify_started = Instant::now();
        // LIT-35.4: verify the sorted pair stream in parallel over bounded
        // chunks, producing ordered results without touching the graph, then
        // apply them single-threaded. Relation ids, decisions, and counters are
        // byte-identical for any worker count because chunk results are merged
        // back into the original pair order.
        let workers = std::thread::available_parallelism().map_or(1, usize::from);
        // Bounded chunk size keeps per-chunk memory independent of pair count.
        const VERIFY_CHUNK: usize = 8_192;
        let (verification, merge_us) = verify_clone_pairs(
            &candidates,
            &pairs,
            detail,
            selectors,
            CloneThresholds {
                similar: SIMILAR_THRESHOLD,
                trace: TRACE_NEAR_THRESHOLD,
                high_confidence: HIGH_CONFIDENCE_THRESHOLD,
            },
            workers,
            VERIFY_CHUNK,
        );
        diagnostics.merge_us = merge_us;
        diagnostics.comparison_count = pairs.len() as u64;
        diagnostics.emitted_count = verification.emitted_count;
        diagnostics.rejected_near_threshold_count = verification.rejected_near_threshold_count;
        diagnostics.decisions = verification.decisions;
        for relation in &verification.relations {
            self.relate_with_provenance(
                relation.source.clone(),
                relation.target.clone(),
                RelationKind::SimilarTo,
                relation.confidence,
                vec![
                    relation.left_evidence.clone(),
                    relation.right_evidence.clone(),
                ],
                Some(RelationProvenance {
                    language: None,
                    resolver_strategy: "lexical-jaccard-similarity".to_owned(),
                    resolution: RelationResolution::HybridResolved,
                    confidence: relation.confidence,
                }),
            );
        }
        diagnostics.exact_verify_us = exact_verify_started
            .elapsed()
            .as_micros()
            .try_into()
            .unwrap_or(u64::MAX);
        // Persist the snapshot for a future warm rebuild. Best-effort: a write
        // failure only costs the next build a recompute (LIT-35.3 AC4).
        if let Some(path) = &snapshot_path {
            let _ = JsonStore.write(
                path,
                &CloneSnapshot {
                    algorithm_version: CLONE_ALGORITHM_VERSION,
                    relations: verification
                        .relations
                        .iter()
                        .map(|relation| CloneSnapshotRelation {
                            source: relation.source.as_str().to_owned(),
                            target: relation.target.as_str().to_owned(),
                            confidence: relation.confidence,
                            left_evidence: relation.left_evidence.clone(),
                            right_evidence: relation.right_evidence.clone(),
                        })
                        .collect(),
                    candidate_count: diagnostics.candidate_count,
                    comparison_count: diagnostics.comparison_count,
                    emitted_count: diagnostics.emitted_count,
                    rejected_near_threshold_count: diagnostics.rejected_near_threshold_count,
                    pruned_count: diagnostics.pruned_count,
                    peak_candidate_pairs: diagnostics.peak_candidate_pairs,
                    prefilter_pairs: diagnostics.prefilter_pairs,
                },
            );
        }
        diagnostics
    }
    /// Replays a cached near-clone [`CloneSnapshot`]'s emitted relations in
    /// stored (emission) order, reproducing the exact `relate_with_provenance`
    /// calls a fresh detection would make (LIT-35.3 AC2). Relation ids stay
    /// byte-identical because every prior pass is deterministic, so
    /// `relation_count` matches at replay time.
    fn apply_clone_snapshot(&mut self, snapshot: &CloneSnapshot) {
        for relation in &snapshot.relations {
            self.relate_with_provenance(
                GraphNodeId::new(relation.source.clone()),
                GraphNodeId::new(relation.target.clone()),
                RelationKind::SimilarTo,
                relation.confidence,
                vec![
                    relation.left_evidence.clone(),
                    relation.right_evidence.clone(),
                ],
                Some(RelationProvenance {
                    language: None,
                    resolver_strategy: "lexical-jaccard-similarity".to_owned(),
                    resolution: RelationResolution::HybridResolved,
                    confidence: relation.confidence,
                }),
            );
        }
    }
}

fn clone_decision(
    left: &GraphNodeId,
    right: &GraphNodeId,
    left_evidence: &EvidenceRef,
    right_evidence: &EvidenceRef,
    similarity: f64,
    outcome: &str,
    reason: &str,
) -> GraphDecisionTrace {
    GraphDecisionTrace {
        kind: "near_clone".to_owned(),
        source: left.as_str().to_owned(),
        target: right.as_str().to_owned(),
        strategy: "lexical_jaccard_similarity".to_owned(),
        outcome: outcome.to_owned(),
        score_millionths: (similarity * 1_000_000.0).round() as u32,
        evidence_paths: vec![
            left_evidence.path.as_str().to_owned(),
            right_evidence.path.as_str().to_owned(),
        ],
        reason: reason.to_owned(),
    }
}

/// LIT-35.2: the candidate pairs that must reach exact Jaccard verification.
///
/// Reproduces the size-band universe -- same language, size bands within one,
/// length ratio at or above `similar_threshold` -- but only *generates* pairs
/// that share a rare token, using a per-language prefix inverted index. Tokens
/// are ordered rarest-first by (document frequency, token id); a candidate's
/// prefix is its first `len - ceil(trace_threshold * len) + 1` tokens. Under a
/// common token order, any pair whose Jaccard can reach `trace_threshold`
/// shares a token inside both prefixes (the prefix-filter principle), so no
/// emittable or near-threshold pair is lost. Pairs sharing no rare token have
/// Jaccard strictly below `trace_threshold` and are pruned before verification.
///
/// Returns the deduplicated ascending `(min, max)` index pairs plus the number
/// of distinct pairs the prefix index produced before the band predicate, for
/// diagnostics (LIT-35.2 AC4).
fn clone_verification_pairs(
    candidates: &[CloneCandidate],
    similar_threshold: f64,
    trace_threshold: f64,
) -> (Vec<(u32, u32)>, u64) {
    // Pairing and the inverted index are both per-language, matching the
    // existing behaviour that never compares functions across languages.
    let mut by_language: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, candidate) in candidates.iter().enumerate() {
        by_language
            .entry(candidate.language.as_str())
            .or_default()
            .push(index);
    }

    // The exact band predicate that previously decided which pairs reach
    // verification; a prefix pair outside it (e.g. two size bands apart) must
    // never be verified, or near-threshold diagnostics would change.
    let in_band_universe = |i: usize, j: usize| -> bool {
        let a = &candidates[i];
        let b = &candidates[j];
        if a.size_band.abs_diff(b.size_band) > 1 {
            return false;
        }
        let smaller = a.tokens.len().min(b.tokens.len()) as f64;
        let larger = a.tokens.len().max(b.tokens.len()) as f64;
        smaller / larger >= similar_threshold
    };

    let mut pairs: Vec<(u32, u32)> = Vec::new();
    let mut prefilter_pairs: u64 = 0;
    for members in by_language.values() {
        // Document frequency of each token within this language group.
        let mut frequency: HashMap<u32, u32> = HashMap::new();
        for &index in members {
            for &token in &candidates[index].tokens {
                *frequency.entry(token).or_insert(0) += 1;
            }
        }
        // Prefix inverted index: rarest tokens first, so postings lists stay
        // short and are dominated by genuinely similar candidates.
        let mut index: HashMap<u32, Vec<usize>> = HashMap::new();
        for &member in members {
            let tokens = &candidates[member].tokens;
            let mut ordered: Vec<u32> = tokens.clone();
            ordered.sort_by_key(|token| (frequency[token], *token));
            let length = tokens.len();
            let prefix =
                (length - (trace_threshold * length as f64).ceil() as usize + 1).clamp(1, length);
            for &token in ordered.iter().take(prefix) {
                index.entry(token).or_default().push(member);
            }
        }
        // Candidates co-occurring in a postings list are the only pairs that
        // can meet the threshold. LIT-38.1: walk each postings list ordered by
        // token-set length so the size-band / length-ratio bound stops the
        // inner scan early -- once a longer candidate fails the length ratio,
        // every later (even longer) one fails too, so no in-band pair is ever
        // skipped. This bounds generation from O(sum of list^2) to the in-band
        // neighbourhood, applying the band predicate during traversal instead
        // of materializing every co-occurring pair first.
        let mut language_pairs: Vec<(u32, u32)> = Vec::new();
        for postings in index.values_mut() {
            postings.sort_unstable_by_key(|&member| (candidates[member].tokens.len(), member));
            for (position, &i) in postings.iter().enumerate() {
                let i_len = candidates[i].tokens.len() as f64;
                for &j in &postings[position + 1..] {
                    // Length-ordered, so `j` is at least as long as `i`; this is
                    // the exact length-ratio test `in_band_universe` applies, so
                    // the break agrees with it and drops only out-of-band pairs.
                    if i_len / (candidates[j].tokens.len() as f64) < similar_threshold {
                        break;
                    }
                    if in_band_universe(i, j) {
                        language_pairs.push((i.min(j) as u32, i.max(j) as u32));
                    }
                }
            }
        }
        // A pair can share more than one prefix token; dedupe so verification
        // (and the band-universe subset invariant) sees each pair once.
        language_pairs.sort_unstable();
        language_pairs.dedup();
        // Count the deduplicated in-band candidate pairs, comparable to the
        // pre-LIT-38.1 diagnostic (which the bound reduces because out-of-band
        // pairs are never generated).
        prefilter_pairs += language_pairs.len() as u64;
        pairs.extend_from_slice(&language_pairs);
    }
    // Distinct languages never share a pair, so `pairs` is already unique and
    // sorting yields the ascending order the verifier and decision trace
    // expect.
    pairs.sort_unstable();
    (pairs, prefilter_pairs)
}

/// Emission thresholds for exact verification, grouped to keep the parallel
/// entry point's signature readable.
#[derive(Clone, Copy)]
struct CloneThresholds {
    similar: f64,
    trace: f64,
    high_confidence: f64,
}

/// One verified `SimilarTo` relation, produced before any graph mutation so
/// verification can run off the main thread (LIT-35.4).
#[derive(Debug, PartialEq)]
struct VerifiedRelation {
    source: GraphNodeId,
    target: GraphNodeId,
    confidence: Confidence,
    left_evidence: EvidenceRef,
    right_evidence: EvidenceRef,
}

/// Ordered result of verifying a pair stream (or one chunk of it): emitted
/// relations and trace decisions in pair order, plus summable counters.
#[derive(Debug, PartialEq)]
struct CloneVerification {
    relations: Vec<VerifiedRelation>,
    decisions: Vec<GraphDecisionTrace>,
    emitted_count: u64,
    rejected_near_threshold_count: u64,
}

/// Exact Jaccard verification of `pairs` (LIT-35.4). Splits the sorted stream
/// into `chunk_size` chunks verified across up to `worker_count` scoped threads
/// that mutate no shared state, then concatenates chunk results in ascending
/// chunk order -- the original pair order -- so relations, decisions, and
/// counters are byte-identical for any worker count. A panicking chunk is
/// re-raised with its index and pair span; the caller only mutates the graph
/// after this returns, so a panic never leaves a partial graph (AC1/AC2/AC3).
fn verify_clone_pairs(
    candidates: &[CloneCandidate],
    pairs: &[(u32, u32)],
    detail: &GraphBuildTraceDetail,
    selectors: &[String],
    thresholds: CloneThresholds,
    worker_count: usize,
    chunk_size: usize,
) -> (CloneVerification, u64) {
    // A single chunk's ordered verification. Pure: reads only shared immutable
    // inputs and returns owned results.
    let verify_chunk = |chunk: &[(u32, u32)]| -> CloneVerification {
        let mut relations = Vec::new();
        let mut decisions = Vec::new();
        let mut emitted_count = 0u64;
        let mut rejected_near_threshold_count = 0u64;
        for &(i, j) in chunk {
            let left = &candidates[i as usize];
            let right = &candidates[j as usize];
            let similarity = jaccard_similarity(&left.tokens, &right.tokens);
            let should_trace = (*detail == GraphBuildTraceDetail::Full && selectors.is_empty())
                || selectors.iter().any(|selector| {
                    left.id.as_str().contains(selector)
                        || right.id.as_str().contains(selector)
                        || left.evidence.path.as_str().contains(selector)
                        || right.evidence.path.as_str().contains(selector)
                });
            if similarity < thresholds.similar {
                if similarity >= thresholds.trace {
                    rejected_near_threshold_count += 1;
                    if should_trace {
                        decisions.push(clone_decision(
                            &left.id,
                            &right.id,
                            &left.evidence,
                            &right.evidence,
                            similarity,
                            "rejected",
                            "exact Jaccard score was below the configured emission threshold",
                        ));
                    }
                }
                continue;
            }
            emitted_count += 1;
            if should_trace {
                decisions.push(clone_decision(
                    &left.id,
                    &right.id,
                    &left.evidence,
                    &right.evidence,
                    similarity,
                    "emitted",
                    "exact Jaccard score met the configured emission threshold",
                ));
            }
            let confidence = if similarity >= thresholds.high_confidence {
                Confidence::High
            } else {
                Confidence::Low
            };
            let (source, target) = if left.id <= right.id {
                (left.id.clone(), right.id.clone())
            } else {
                (right.id.clone(), left.id.clone())
            };
            relations.push(VerifiedRelation {
                source,
                target,
                confidence,
                left_evidence: left.evidence.clone(),
                right_evidence: right.evidence.clone(),
            });
        }
        CloneVerification {
            relations,
            decisions,
            emitted_count,
            rejected_near_threshold_count,
        }
    };

    let chunk_size = chunk_size.max(1);
    let chunk_count = pairs.len().div_ceil(chunk_size);
    // Small inputs or a single worker skip thread setup entirely; no merge.
    if worker_count <= 1 || chunk_count <= 1 {
        return (verify_chunk(pairs), 0);
    }

    let next_chunk = std::sync::atomic::AtomicUsize::new(0);
    let results: std::sync::Mutex<Vec<(usize, CloneVerification)>> =
        std::sync::Mutex::new(Vec::with_capacity(chunk_count));
    std::thread::scope(|scope| {
        for _ in 0..worker_count.min(chunk_count) {
            scope.spawn(|| {
                loop {
                    let index = next_chunk.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    if index >= chunk_count {
                        break;
                    }
                    let start = index * chunk_size;
                    let end = (start + chunk_size).min(pairs.len());
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        verify_chunk(&pairs[start..end])
                    }));
                    match outcome {
                        Ok(result) => results
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .push((index, result)),
                        Err(_) => std::panic::resume_unwind(Box::new(format!(
                            "near-clone verification panicked in chunk {index} (pairs {start}..{end})"
                        ))),
                    }
                }
            });
        }
    });

    // Merge chunks in ascending index -- the original pair order -- so relation
    // ids, decisions, and counters do not depend on worker scheduling.
    let merge_started = Instant::now();
    let mut ordered = results
        .into_inner()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    ordered.sort_by_key(|(index, _)| *index);
    let mut merged = CloneVerification {
        relations: Vec::new(),
        decisions: Vec::new(),
        emitted_count: 0,
        rejected_near_threshold_count: 0,
    };
    for (_, chunk) in ordered {
        merged.relations.extend(chunk.relations);
        merged.decisions.extend(chunk.decisions);
        merged.emitted_count += chunk.emitted_count;
        merged.rejected_near_threshold_count += chunk.rejected_near_threshold_count;
    }
    let merge_us = merge_started
        .elapsed()
        .as_micros()
        .try_into()
        .unwrap_or(u64::MAX);
    (merged, merge_us)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    #[test]
    fn full_trace_explains_near_threshold_clone_rejection() -> Result<(), Box<dyn std::error::Error>>
    {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("pairs.py"),
            "def alpha(value):\n    total = value\n    clean = strip(total)\n    return clean\n\ndef beta(value):\n    total = value\n    dirty = encode(total)\n    return dirty\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let output = GraphBuilder.build_with_trace(
            repo.path(),
            &artifacts,
            None,
            GraphBuildTraceConfig {
                detail: GraphBuildTraceDetail::Full,
                selectors: Vec::new(),
            },
        );
        let enrichment = output
            .trace
            .as_ref()
            .and_then(|trace| {
                trace
                    .stages
                    .iter()
                    .find(|stage| stage.pass == GraphBuildPass::Enrichment)
            })
            .ok_or("missing enrichment trace")?;
        assert!(
            enrichment.decisions.iter().any(|decision| {
                decision.kind == "near_clone" && decision.outcome == "rejected"
            })
        );
        Ok(())
    }

    /// LIT-35.3 AC2/AC3/AC5: a warm rebuild over an unchanged candidate set
    /// reuses the persisted clone snapshot (cache hit, no exact verification)
    /// and yields SimilarTo relations byte-identical to a fresh build; editing a
    /// function body invalidates the identity and forces a recompute.
    #[test]
    fn clone_snapshot_cache_hits_then_invalidates_on_edit() -> Result<(), Box<dyn std::error::Error>>
    {
        let repo = tempfile::TempDir::new()?;
        let clones = repo.path().join("clones.py");
        std::fs::write(
            &clones,
            "def calculate_total(items):\n    total = 0\n    for item in items:\n        total += item.price\n    return total\n\n\ndef calculate_total_v2(items):\n    total = 0\n    for item in items:\n        total += item.price * 2\n    return total\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let cache_dir = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(cache_dir.path());
        let summary = GraphBuildTraceConfig {
            detail: GraphBuildTraceDetail::Summary,
            selectors: Vec::new(),
        };

        let similar = |graph: &crate::graph::model::Graph| {
            let mut relations: Vec<_> = graph
                .relations
                .iter()
                .filter(|relation| relation.kind == RelationKind::SimilarTo)
                .cloned()
                .collect();
            relations.sort_by(|a, b| (&a.source, &a.target).cmp(&(&b.source, &b.target)));
            relations
        };
        let cache_hit = |output: &crate::graph::GraphBuildOutput| {
            output
                .trace
                .as_ref()
                .and_then(|trace| {
                    trace
                        .stages
                        .iter()
                        .find(|stage| stage.pass == GraphBuildPass::Enrichment)
                })
                .map(|stage| stage.counters["clone_cache_hit"])
                .unwrap_or_default()
        };

        let fresh = GraphBuilder.build(repo.path(), &artifacts);
        let fresh_similar = similar(&fresh);
        assert!(!fresh_similar.is_empty(), "fixture must have a clone pair");

        // First cached build misses and writes the snapshot.
        let first =
            GraphBuilder.build_with_trace(repo.path(), &artifacts, Some(&cache), summary.clone());
        assert_eq!(cache_hit(&first), 0);
        // Second build over the unchanged set hits and skips verification while
        // reproducing byte-identical relations.
        let second =
            GraphBuilder.build_with_trace(repo.path(), &artifacts, Some(&cache), summary.clone());
        assert_eq!(cache_hit(&second), 1);
        assert_eq!(similar(&second.graph), fresh_similar);

        // Editing a body changes a candidate content hash, so the whole-set
        // identity changes and the next build recomputes.
        std::fs::write(
            &clones,
            "def calculate_total(items):\n    total = 0\n    for item in items:\n        total += item.cost\n    return total\n\n\ndef calculate_total_v2(items):\n    total = 0\n    for item in items:\n        total += item.cost * 2\n    return total\n",
        )?;
        let edited = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let third =
            GraphBuilder.build_with_trace(repo.path(), &edited, Some(&cache), summary.clone());
        assert_eq!(cache_hit(&third), 0);
        Ok(())
    }

    /// LIT-35.3 AC4: a corrupt or wrong-version snapshot is treated as a miss --
    /// detection recomputes and overwrites, and correctness output is unchanged.
    #[test]
    fn clone_snapshot_cache_recovers_from_corrupt_and_stale_entries()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{CLONE_ALGORITHM_VERSION, CloneSnapshot};

        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("clones.py"),
            "def calculate_total(items):\n    total = 0\n    for item in items:\n        total += item.price\n    return total\n\n\ndef calculate_total_v2(items):\n    total = 0\n    for item in items:\n        total += item.price * 2\n    return total\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let cache_dir = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(cache_dir.path());
        let summary = GraphBuildTraceConfig {
            detail: GraphBuildTraceDetail::Summary,
            selectors: Vec::new(),
        };

        let similar = |graph: &crate::graph::model::Graph| {
            graph
                .relations
                .iter()
                .filter(|relation| relation.kind == RelationKind::SimilarTo)
                .count()
        };
        let cache_hit = |output: &crate::graph::GraphBuildOutput| {
            output
                .trace
                .as_ref()
                .and_then(|trace| {
                    trace
                        .stages
                        .iter()
                        .find(|stage| stage.pass == GraphBuildPass::Enrichment)
                })
                .map(|stage| stage.counters["clone_cache_hit"])
                .unwrap_or_default()
        };

        // Populate the snapshot.
        let baseline = GraphBuilder.build(repo.path(), &artifacts);
        let expected = similar(&baseline);
        assert!(expected > 0);
        GraphBuilder.build_with_trace(repo.path(), &artifacts, Some(&cache), summary.clone());

        let snapshot_file = std::fs::read_dir(cache_dir.path())?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.contains("clone"))
            })
            .ok_or("snapshot file was not written")?;

        // Corrupt JSON -> miss, recompute, unchanged output.
        std::fs::write(&snapshot_file, b"{ not valid json")?;
        let after_corrupt =
            GraphBuilder.build_with_trace(repo.path(), &artifacts, Some(&cache), summary.clone());
        assert_eq!(cache_hit(&after_corrupt), 0);
        assert_eq!(similar(&after_corrupt.graph), expected);

        // Valid JSON but a stale algorithm version -> miss, recompute.
        crate::storage::JsonStore.write(
            &snapshot_file,
            &CloneSnapshot {
                algorithm_version: CLONE_ALGORITHM_VERSION + 1,
                relations: Vec::new(),
                candidate_count: 0,
                comparison_count: 0,
                emitted_count: 0,
                rejected_near_threshold_count: 0,
                pruned_count: 0,
                peak_candidate_pairs: 0,
                prefilter_pairs: 0,
            },
        )?;
        let after_stale =
            GraphBuilder.build_with_trace(repo.path(), &artifacts, Some(&cache), summary.clone());
        assert_eq!(cache_hit(&after_stale), 0);
        assert_eq!(similar(&after_stale.graph), expected);
        Ok(())
    }

    /// LIT-35.4 AC1/AC2: verifying the same pair stream with one worker and
    /// with many workers over small chunks yields byte-identical relations,
    /// decisions, and counters -- the parallel merge restores the sequential
    /// pair order regardless of worker scheduling.
    #[test]
    fn parallel_verification_matches_single_worker() -> Result<(), Box<dyn std::error::Error>> {
        use super::{
            CloneCandidate, CloneThresholds, clone_verification_pairs, verify_clone_pairs,
        };
        use crate::domain::EvidenceRef;
        use crate::domain::ids::{ArtifactId, RepoPath};
        use crate::graph::GraphBuildTraceDetail;
        use crate::graph::model::GraphNodeId;

        let repo_path = RepoPath::new("clones.py")?;
        let evidence = EvidenceRef::file(ArtifactId::from_path(&repo_path), repo_path);

        // Small vocabulary so candidates overlap heavily -- plenty of emitted
        // and near-threshold pairs to order across chunks.
        let mut state: u64 = 0x243F6A8885A308D3;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };
        let mut candidates = Vec::new();
        for member in 0..48usize {
            let token_count = 4 + (next() % 6) as usize;
            let mut tokens: Vec<u32> = (0..token_count).map(|_| next() % 10).collect();
            tokens.sort_unstable();
            tokens.dedup();
            let size_band = usize::BITS - tokens.len().max(1).leading_zeros() - 1;
            candidates.push(CloneCandidate {
                id: GraphNodeId::new(format!("symbol:clones.py#clones::member{member}")),
                evidence: evidence.clone(),
                tokens,
                language: "py".to_owned(),
                size_band,
            });
        }

        let (pairs, _) = clone_verification_pairs(&candidates, 0.6, 0.35);
        assert!(pairs.len() > 3, "need multiple chunks to exercise merge");
        let thresholds = CloneThresholds {
            similar: 0.6,
            trace: 0.35,
            high_confidence: 0.85,
        };
        let selectors: Vec<String> = Vec::new();
        // Full detail so decisions are produced and their ordering is tested.
        // The merge duration is ignored: only the verification result is compared.
        let (sequential, _) = verify_clone_pairs(
            &candidates,
            &pairs,
            &GraphBuildTraceDetail::Full,
            &selectors,
            thresholds,
            1,
            1_000_000,
        );
        let (parallel, _) = verify_clone_pairs(
            &candidates,
            &pairs,
            &GraphBuildTraceDetail::Full,
            &selectors,
            thresholds,
            8,
            3,
        );
        assert_eq!(sequential, parallel);
        assert!(
            !sequential.relations.is_empty() || sequential.rejected_near_threshold_count > 0,
            "fixture must produce verification work"
        );
        assert_eq!(sequential.decisions.len(), parallel.decisions.len());
        Ok(())
    }

    /// LIT-35.2 AC1/AC2: exhaustive differential proof that the prefix-filter
    /// candidate generation never drops a size-band pair whose exact Jaccard
    /// can reach the trace threshold, and never produces a pair outside the
    /// band universe. Runs over many deterministic pseudo-random candidate sets
    /// so a false negative in the overlap bound fails the build.
    #[test]
    fn prefix_filter_loses_no_band_universe_pair_at_threshold()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{CloneCandidate, clone_verification_pairs, jaccard_similarity};
        use crate::domain::EvidenceRef;
        use crate::domain::ids::{ArtifactId, RepoPath};
        use crate::graph::model::GraphNodeId;

        let repo_path = RepoPath::new("clones.py")?;
        let evidence = EvidenceRef::file(ArtifactId::from_path(&repo_path), repo_path);

        const SIMILAR_THRESHOLD: f64 = 0.6;
        const TRACE_NEAR_THRESHOLD: f64 = 0.35;

        // Deterministic linear-congruential generator: no wall clock, no
        // std::rand, so the corpus of random candidate sets is reproducible.
        let mut state: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u32
        };

        for trial in 0..400u32 {
            let candidate_count = 2 + (next() % 18) as usize;
            let vocabulary = 3 + (next() % 25);
            let mut candidates = Vec::new();
            for member in 0..candidate_count {
                let token_count = 2 + (next() % 12) as usize;
                let mut tokens: Vec<u32> = (0..token_count).map(|_| next() % vocabulary).collect();
                tokens.sort_unstable();
                tokens.dedup();
                if tokens.is_empty() {
                    continue;
                }
                let size_band = usize::BITS - tokens.len().max(1).leading_zeros() - 1;
                candidates.push(CloneCandidate {
                    id: GraphNodeId::new(format!("symbol:trial{trial}::member{member}")),
                    evidence: evidence.clone(),
                    tokens,
                    language: "py".to_owned(),
                    size_band,
                });
            }

            // Brute-force band universe: exactly the pairs the pre-filter
            // generation used to hand to exact verification.
            let mut band_universe = Vec::new();
            for i in 0..candidates.len() {
                for j in (i + 1)..candidates.len() {
                    let a = &candidates[i];
                    let b = &candidates[j];
                    if a.size_band.abs_diff(b.size_band) > 1 {
                        continue;
                    }
                    let smaller = a.tokens.len().min(b.tokens.len()) as f64;
                    let larger = a.tokens.len().max(b.tokens.len()) as f64;
                    if smaller / larger >= SIMILAR_THRESHOLD {
                        band_universe.push((i as u32, j as u32));
                    }
                }
            }

            let (filtered, _) =
                clone_verification_pairs(&candidates, SIMILAR_THRESHOLD, TRACE_NEAR_THRESHOLD);
            let filtered_set: std::collections::BTreeSet<(u32, u32)> =
                filtered.iter().copied().collect();
            let band_set: std::collections::BTreeSet<(u32, u32)> =
                band_universe.iter().copied().collect();

            // No spurious pairs: the filter never verifies outside the band
            // universe (which would change emitted/near-threshold output).
            assert!(
                filtered_set.is_subset(&band_set),
                "trial {trial}: filtered pairs escaped the band universe"
            );
            // Zero false negatives: every band pair that could reach the trace
            // threshold survives filtering.
            for &(i, j) in &band_universe {
                let similarity = jaccard_similarity(
                    &candidates[i as usize].tokens,
                    &candidates[j as usize].tokens,
                );
                if similarity >= TRACE_NEAR_THRESHOLD {
                    assert!(
                        filtered_set.contains(&(i, j)),
                        "trial {trial}: dropped pair ({i},{j}) with Jaccard {similarity}"
                    );
                }
            }
        }
        Ok(())
    }

    #[test]
    fn clone_candidate_bands_prune_representative_pair_growth()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        let mut source = String::new();
        for index in 0..64usize {
            source.push_str(&format!(
                "def generated_{index}(value):\n    total = value\n"
            ));
            for token in 0..(1usize << (index % 6)) {
                source.push_str(&format!("    total += unique_{index}_{token}\n"));
            }
            source.push_str("    return total\n\n");
        }
        std::fs::write(repo.path().join("generated.py"), source)?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let output = GraphBuilder.build_with_trace(
            repo.path(),
            &artifacts,
            None,
            GraphBuildTraceConfig::default(),
        );
        let enrichment = output
            .trace
            .as_ref()
            .and_then(|trace| {
                trace
                    .stages
                    .iter()
                    .find(|stage| stage.pass == GraphBuildPass::Enrichment)
            })
            .ok_or("missing enrichment trace")?;
        let comparisons = enrichment.counters["clone_comparisons"];
        let total_pairs = 64 * 63 / 2;
        assert!(comparisons < total_pairs / 2);
        assert!(enrichment.counters["clone_pruned"] > comparisons);
        Ok(())
    }
}
