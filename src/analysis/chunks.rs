//! Deterministic, syntax-aware source chunking (LIT-86.2).
//!
//! Turns one artifact's already-safe source text into retrieval-sized chunks
//! that prefer syntax and line boundaries, never lose or duplicate source
//! bytes in their non-overlapping cores, and carry stable identities matching
//! the LIT-86.1 contract (`chunk:{path}#{ordinal}`, `~raw` for fallback).
//!
//! This layer is intentionally *pure*: it consumes syntax-boundary byte
//! offsets it is handed rather than parsing anything itself, so it creates no
//! independent parsing stack. The shared parsed-source product (LIT-86.14)
//! supplies those boundaries later without this module changing.

// ponytail: pure chunking API landed ahead of its callers; the vector index
// (LIT-86.3) and chunk-node builder (LIT-86.4) consume it. Drop this allow when
// the first production caller wires in.
#![allow(dead_code)]

use crate::analysis::parsed_source::LineIndex;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Retrieval sizing for the chunker, in explicit **bytes** (not characters, so
/// a multibyte file is bounded by its real embedding cost, not its glyph
/// count). Callers that think in characters must convert first; the byte/char
/// distinction is deliberate and tested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ChunkConfig {
    /// Preferred core size; packing stops adding atoms once a core reaches it.
    pub target_bytes: usize,
    /// Cores below this may still be emitted (e.g. a tiny file or a trailing
    /// remainder); it only biases merging, never drops bytes.
    pub min_bytes: usize,
    /// Hard ceiling: no core exceeds this, even a single oversized
    /// declaration, which is recursively subdivided to satisfy it.
    pub max_bytes: usize,
    /// Bytes of preceding context prepended to each core after the
    /// non-overlapping partition is fixed (AC#3).
    pub overlap_bytes: usize,
}

impl Default for ChunkConfig {
    fn default() -> Self {
        // Retrieval-oriented defaults: ~1.5 KB cores with light overlap read
        // well for both embeddings and human inspection; the ceiling keeps a
        // single pathological declaration from dominating a chunk.
        Self {
            target_bytes: 1500,
            min_bytes: 200,
            max_bytes: 3000,
            overlap_bytes: 150,
        }
    }
}

/// Whether a chunk came from a real syntax parse or a documented fallback.
/// Fallback identities gain a `~raw` marker so a later AST-capable run never
/// silently reuses a coarse fallback chunk (LIT-86.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ChunkParse {
    /// Boundaries came from a syntax parse.
    Syntax,
    /// No usable parse; boundaries are line/character based.
    Fallback {
        /// Human-readable reason (unknown grammar, syntax error, ...).
        reason: String,
    },
}

/// A one-based line and one-based, character-counted column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LineCol {
    /// One-based line.
    pub line: u32,
    /// One-based column, counted in characters (not bytes) from the line start.
    pub column: u32,
}

/// One retrieval chunk of a single artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SourceChunk {
    /// Stable identity: `chunk:{path}#{ordinal}` (+`~raw` when `parse` is a
    /// fallback), per LIT-86.1.
    pub id: String,
    /// Repository-relative artifact path.
    pub path: String,
    /// Zero-based position of this chunk within the artifact.
    pub ordinal: u32,
    /// Byte range covered by `text`, including any leading overlap.
    pub byte_range: Range<usize>,
    /// Character range covered by `text`, including any leading overlap.
    pub char_range: Range<usize>,
    /// One-based start line/column of `byte_range`.
    pub start: LineCol,
    /// One-based end line/column of `byte_range` (inclusive of the last
    /// character; equal to `start` for an empty chunk, which never occurs
    /// here).
    pub end: LineCol,
    /// `blake3` of `text`; the embedding-invalidation key (LIT-86.1), distinct
    /// from `id`, so two byte-identical chunks share a vector while keeping
    /// distinct identities.
    pub content_hash: String,
    /// Parser provenance.
    pub parse: ChunkParse,
    /// Exact source text of `byte_range`; round-trips to the source span.
    pub text: String,
}

/// Why an artifact was not chunked at all (AC#6). The chunker proper only sees
/// safe text; this typed reason lets the caller report an observable skip
/// without the chunker importing the whole inventory layer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SkipReason {
    /// Path/content classified secret or otherwise model-excluded.
    ModelExcluded,
    /// Non-text bytes.
    Binary,
    /// Redacted content (e.g. inline private key).
    Redacted,
    /// Machine-generated / vendored / otherwise excluded from analysis.
    Excluded,
    /// At or above the analyzable size ceiling.
    OverLimit,
    /// Empty after trimming; nothing to embed.
    Empty,
}

/// One-based line/column of a byte offset, as a [`LineCol`]. Thin adapter over
/// the shared [`LineIndex`] so the chunker speaks in its own span type.
fn line_col(index: &LineIndex, text: &str, offset: usize) -> LineCol {
    let (line, column) = index.line_col(text, offset);
    LineCol { line, column }
}

/// Chunks `text` (already safe to read) for `path`. `syntax_boundaries` are
/// byte offsets at strong syntax cut points (e.g. the start of a top-level
/// declaration); pass an empty slice when no parse is available. `parse`
/// records the provenance and drives the `~raw` identity marker.
///
/// Returns chunks in source order whose non-overlapping cores tile the entire
/// input; the returned `text`/ranges include optional leading overlap.
pub(crate) fn chunk_source(
    path: &str,
    text: &str,
    syntax_boundaries: &[usize],
    parse: &ChunkParse,
    config: &ChunkConfig,
) -> Vec<SourceChunk> {
    if text.is_empty() {
        return Vec::new();
    }

    let cores = partition_into_cores(text, syntax_boundaries, config);
    let line_index = LineIndex::new(text);

    cores
        .iter()
        .enumerate()
        .map(|(ordinal, core)| {
            // Overlap extends a core's start backward for context, snapped to a
            // char boundary and never past the previous core's start, so the
            // non-overlapping cores remain the authoritative partition.
            let overlap_start = if ordinal == 0 || config.overlap_bytes == 0 {
                core.start
            } else {
                let prev_start = cores[ordinal - 1].start;
                let raw = core
                    .start
                    .saturating_sub(config.overlap_bytes)
                    .max(prev_start);
                floor_char_boundary(text, raw)
            };
            let range = overlap_start..core.end;
            let slice = &text[range.clone()];
            SourceChunk {
                id: chunk_id(path, to_u32(ordinal), parse),
                path: path.to_owned(),
                ordinal: to_u32(ordinal),
                char_range: char_index(text, range.start)..char_index(text, range.end),
                start: line_col(&line_index, text, range.start),
                // The end column points at the last character, so a one-line
                // chunk's `end.column - start.column + 1` equals its char width.
                end: line_col(
                    &line_index,
                    text,
                    floor_char_boundary(text, range.end.saturating_sub(1)),
                ),
                content_hash: blake3::hash(slice.as_bytes()).to_hex().to_string(),
                parse: parse.clone(),
                text: slice.to_owned(),
                byte_range: range,
            }
        })
        .collect()
}

/// A non-overlapping `[start, end)` core; the authoritative partition of the
/// source before overlap is added.
#[derive(Debug, Clone, Copy)]
struct Core {
    start: usize,
    end: usize,
}

/// Packs the source into cores that tile `[0, len)`: candidate cut points are
/// the union of syntax boundaries and line starts; atoms between them are
/// merged left-to-right until a core reaches `target_bytes`, and any core that
/// would exceed `max_bytes` is recursively subdivided at its best internal
/// boundary. No byte is dropped or duplicated across cores.
fn partition_into_cores(
    text: &str,
    syntax_boundaries: &[usize],
    config: &ChunkConfig,
) -> Vec<Core> {
    let len = text.len();
    let cuts = candidate_cuts(text, syntax_boundaries);

    let mut cores = Vec::new();
    let mut core_start = 0usize;
    for &cut in &cuts {
        if cut <= core_start {
            continue;
        }
        // Close the current core once including this atom reaches the target;
        // this keeps cores near `target_bytes` while cutting on real
        // syntax/line boundaries rather than mid-token.
        if cut - core_start >= config.target_bytes {
            emit_core(text, core_start, cut, config, &mut cores);
            core_start = cut;
        }
    }
    if core_start < len {
        emit_core(text, core_start, len, config, &mut cores);
    }
    // An all-empty or boundary-only input still yields one covering core.
    if cores.is_empty() {
        cores.push(Core { start: 0, end: len });
    }
    cores
}

/// Emits `[start, end)` as one core, recursively subdividing it at the best
/// internal boundary (line preferred over an arbitrary char boundary) whenever
/// it exceeds `max_bytes`, so an oversized declaration is split without losing
/// bytes (AC#2).
fn emit_core(text: &str, start: usize, end: usize, config: &ChunkConfig, out: &mut Vec<Core>) {
    if end - start <= config.max_bytes {
        out.push(Core { start, end });
        return;
    }
    // Aim the split near `target_bytes` from the start, then snap to the
    // closest line start inside the window, falling back to a char boundary.
    let aim = start + config.target_bytes.min(end - start - 1);
    let split = best_line_boundary(text, start, end, aim)
        .unwrap_or_else(|| floor_char_boundary(text, aim).max(start + 1));
    emit_core(text, start, split, config, out);
    emit_core(text, split, end, config, out);
}

/// Sorted, unique, char-aligned candidate cut offsets strictly inside
/// `(0, len)`: every line start plus every supplied syntax boundary.
fn candidate_cuts(text: &str, syntax_boundaries: &[usize]) -> Vec<usize> {
    let len = text.len();
    let mut cuts: Vec<usize> = Vec::new();
    let mut offset = 0usize;
    for byte in text.bytes() {
        offset += 1;
        if byte == b'\n' && offset < len {
            cuts.push(offset);
        }
    }
    for &boundary in syntax_boundaries {
        if boundary > 0 && boundary < len && text.is_char_boundary(boundary) {
            cuts.push(boundary);
        }
    }
    cuts.sort_unstable();
    cuts.dedup();
    cuts
}

/// The line start closest to `aim` within `(start, end)`, if any.
fn best_line_boundary(text: &str, start: usize, end: usize, aim: usize) -> Option<usize> {
    let mut best: Option<usize> = None;
    let mut offset = start;
    for byte in text[start..end].bytes() {
        offset += 1;
        if byte == b'\n' && offset > start && offset < end {
            let candidate = offset;
            best = Some(match best {
                Some(current) if aim.abs_diff(current) <= aim.abs_diff(candidate) => current,
                _ => candidate,
            });
        }
    }
    best
}

/// Largest char-boundary offset `<= offset`.
fn floor_char_boundary(text: &str, offset: usize) -> usize {
    let mut at = offset.min(text.len());
    while at > 0 && !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

/// Character index of a byte offset (counts chars before it).
fn char_index(text: &str, byte_offset: usize) -> usize {
    text[..byte_offset].chars().count()
}

/// Builds a chunk id per the LIT-86.1 identity contract.
fn chunk_id(path: &str, ordinal: u32, parse: &ChunkParse) -> String {
    match parse {
        ChunkParse::Syntax => format!("chunk:{path}#{ordinal}"),
        ChunkParse::Fallback { .. } => format!("chunk:{path}#{ordinal}~raw"),
    }
}

/// Saturating `usize -> u32` for line/column/ordinal values, which never
/// legitimately exceed `u32::MAX` for a sub-megabyte artifact.
fn to_u32(value: usize) -> u32 {
    u32::try_from(value).unwrap_or(u32::MAX)
}

/// Decides whether an artifact should be chunked at all, given the facts a
/// caller already has from inventory classification (AC#6). Returns the first
/// applicable skip reason, or `None` to proceed. Kept free of inventory types
/// so the chunker stays a leaf; the caller maps its own classification onto
/// these flags.
pub(crate) fn skip_reason(
    size_bytes: u64,
    max_analyzable_bytes: u64,
    is_binary: bool,
    is_model_excluded: bool,
    is_redacted: bool,
    is_excluded: bool,
    trimmed_is_empty: bool,
) -> Option<SkipReason> {
    if is_model_excluded {
        Some(SkipReason::ModelExcluded)
    } else if is_binary {
        Some(SkipReason::Binary)
    } else if is_redacted {
        Some(SkipReason::Redacted)
    } else if is_excluded {
        Some(SkipReason::Excluded)
    } else if size_bytes >= max_analyzable_bytes {
        Some(SkipReason::OverLimit)
    } else if trimmed_is_empty {
        Some(SkipReason::Empty)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{ChunkConfig, ChunkParse, SkipReason, SourceChunk, chunk_source, skip_reason};

    fn syntax(
        config: ChunkConfig,
        path: &str,
        text: &str,
        boundaries: &[usize],
    ) -> Vec<SourceChunk> {
        chunk_source(path, text, boundaries, &ChunkParse::Syntax, &config)
    }

    fn fallback(config: ChunkConfig, path: &str, text: &str) -> Vec<SourceChunk> {
        chunk_source(
            path,
            text,
            &[],
            &ChunkParse::Fallback {
                reason: "no parser".to_owned(),
            },
            &config,
        )
    }

    /// AC#3/#4/#8: cores tile the whole input with no lost or duplicated bytes,
    /// each chunk's text is exactly its byte range, and the result is stable
    /// across repeated runs.
    fn assert_covers_and_round_trips(text: &str, chunks: &[SourceChunk]) {
        assert!(!chunks.is_empty());
        // Reconstruct from non-overlapping cores: each core starts where the
        // previous chunk's overlap-free extent ended. We recover cores by
        // clamping each chunk's start forward to the previous chunk's end.
        let mut rebuilt = String::new();
        let mut cursor = 0usize;
        for chunk in chunks {
            assert_eq!(
                chunk.text,
                &text[chunk.byte_range.clone()],
                "text matches span"
            );
            let core_start = chunk.byte_range.start.max(cursor);
            rebuilt.push_str(&text[core_start..chunk.byte_range.end]);
            cursor = chunk.byte_range.end;
        }
        assert_eq!(rebuilt, text, "cores tile the input exactly");
        assert_eq!(cursor, text.len(), "coverage reaches end of input");
    }

    #[test]
    fn tiny_file_is_a_single_chunk() {
        let text = "fn main() {}\n";
        let chunks = syntax(ChunkConfig::default(), "a.rs", text, &[]);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].id, "chunk:a.rs#0");
        assert_eq!(chunks[0].text, text);
        assert_covers_and_round_trips(text, &chunks);
    }

    #[test]
    fn functions_and_classes_align_to_syntax_boundaries() -> Result<(), Box<dyn std::error::Error>>
    {
        // Two "declarations" separated by a supplied syntax boundary; a small
        // target forces a cut, which must land on the boundary.
        let config = ChunkConfig {
            target_bytes: 20,
            min_bytes: 1,
            max_bytes: 100,
            overlap_bytes: 0,
        };
        let text = "def a():\n    return 1\ndef b():\n    return 2\n";
        let boundary = text.find("def b").ok_or("second def")?;
        let chunks = syntax(config, "m.py", text, &[boundary]);
        assert!(chunks.len() >= 2);
        // Some chunk must start exactly at the second declaration.
        assert!(chunks.iter().any(|c| c.byte_range.start == boundary));
        assert_covers_and_round_trips(text, &chunks);
        Ok(())
    }

    #[test]
    fn nested_blocks_and_comments_are_preserved_in_coverage() {
        let text = "// top comment\nfn outer() {\n  if x {\n    inner();\n  }\n}\n";
        let chunks = fallback(
            ChunkConfig {
                target_bytes: 15,
                min_bytes: 1,
                max_bytes: 40,
                overlap_bytes: 0,
            },
            "n.rs",
            text,
        );
        assert_covers_and_round_trips(text, &chunks);
        assert!(chunks.iter().all(|c| c.id.ends_with("~raw")));
    }

    #[test]
    fn oversized_declaration_is_subdivided_without_losing_bytes() {
        // One 400-byte "declaration" with no internal newlines, max 100 => must
        // be hard-split into several cores, still covering every byte.
        let text = format!("{}\n", "x".repeat(400));
        let config = ChunkConfig {
            target_bytes: 80,
            min_bytes: 1,
            max_bytes: 100,
            overlap_bytes: 0,
        };
        let chunks = syntax(config, "big.rs", &text, &[]);
        assert!(chunks.len() >= 4, "subdivided: {}", chunks.len());
        assert!(chunks.iter().all(|c| c.byte_range.len() <= 100));
        assert_covers_and_round_trips(&text, &chunks);
    }

    #[test]
    fn duplicate_identical_chunks_get_distinct_ids_and_equal_hashes() {
        // Two identical blocks; with a boundary between them and zero overlap,
        // the two cores are byte-identical => same content_hash, different id.
        let block = "aaaa\nbbbb\ncccc\ndddd\n";
        let text = format!("{block}{block}");
        let boundary = block.len();
        let config = ChunkConfig {
            target_bytes: block.len(),
            min_bytes: 1,
            max_bytes: block.len() * 2,
            overlap_bytes: 0,
        };
        let chunks = syntax(config, "dup.txt", &text, &[boundary]);
        assert_eq!(chunks.len(), 2);
        assert_ne!(chunks[0].id, chunks[1].id);
        assert_eq!(chunks[0].content_hash, chunks[1].content_hash);
        assert_covers_and_round_trips(&text, &chunks);
    }

    #[test]
    fn malformed_syntax_falls_back_to_full_coverage() {
        let text = "fn broken( {{{ unbalanced\nmore text here\n";
        let chunks = fallback(ChunkConfig::default(), "bad.rs", text);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].id.ends_with("~raw"));
        assert_covers_and_round_trips(text, &chunks);
    }

    #[test]
    fn unicode_round_trips_and_char_range_differs_from_byte_range() {
        // Multibyte characters: char length < byte length, and text must
        // round-trip exactly (no split mid-codepoint).
        let text = "héllo wörld 日本語 café\nsecond line αβγ\n";
        let config = ChunkConfig {
            target_bytes: 10,
            min_bytes: 1,
            max_bytes: 30,
            overlap_bytes: 0,
        };
        let chunks = syntax(config, "u.txt", text, &[]);
        assert_covers_and_round_trips(text, &chunks);
        for chunk in &chunks {
            // Every chunk slice is valid UTF-8 by construction (it came from a
            // &str slice), and its char count is below its byte count here.
            assert!(chunk.char_range.len() < chunk.byte_range.len());
            assert_eq!(chunk.char_range.len(), chunk.text.chars().count());
        }
    }

    #[test]
    fn crlf_line_columns_match_lf() {
        let lf = "abc\ndef\nghi\n";
        let crlf = "abc\r\ndef\r\nghi\r\n";
        let config = ChunkConfig {
            target_bytes: 1,
            min_bytes: 1,
            max_bytes: 100,
            overlap_bytes: 0,
        };
        let lf_chunks = fallback(config, "lf.txt", lf);
        let crlf_chunks = fallback(config, "crlf.txt", crlf);
        // The first character of the second line is line 2, column 1 in both.
        let lf_second = &lf_chunks[1];
        let crlf_second = &crlf_chunks[1];
        assert_eq!(lf_second.start.line, 2);
        assert_eq!(lf_second.start.column, 1);
        assert_eq!(crlf_second.start.line, 2);
        assert_eq!(crlf_second.start.column, 1);
    }

    #[test]
    fn output_is_byte_identical_across_repeated_runs() {
        let text = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
        let config = ChunkConfig {
            target_bytes: 12,
            min_bytes: 1,
            max_bytes: 40,
            overlap_bytes: 4,
        };
        let first = syntax(config, "r.txt", text, &[8, 20]);
        let second = syntax(config, "r.txt", text, &[8, 20]);
        assert_eq!(first, second);
    }

    #[test]
    fn overlap_extends_backward_only_and_still_round_trips_cores() {
        let text = "alpha\nbravo\ncharlie\ndelta\necho\nfoxtrot\n";
        let config = ChunkConfig {
            target_bytes: 12,
            min_bytes: 1,
            max_bytes: 40,
            overlap_bytes: 6,
        };
        let chunks = syntax(config, "o.txt", text, &[]);
        assert!(chunks.len() >= 2);
        // The second chunk starts at or before the first chunk's end (overlap).
        assert!(chunks[1].byte_range.start <= chunks[0].byte_range.end);
        // Cores (overlap removed) still tile exactly.
        assert_covers_and_round_trips(text, &chunks);
    }

    #[test]
    fn empty_input_yields_no_chunks() {
        assert!(syntax(ChunkConfig::default(), "e.txt", "", &[]).is_empty());
    }

    #[test]
    fn skip_reasons_are_reported_in_priority_order() {
        let max = 1_000_000;
        // Model exclusion wins even if other flags are also set.
        assert_eq!(
            skip_reason(10, max, true, true, true, true, false),
            Some(SkipReason::ModelExcluded)
        );
        assert_eq!(
            skip_reason(10, max, true, false, false, false, false),
            Some(SkipReason::Binary)
        );
        assert_eq!(
            skip_reason(max, max, false, false, false, false, false),
            Some(SkipReason::OverLimit)
        );
        assert_eq!(
            skip_reason(10, max, false, false, false, false, true),
            Some(SkipReason::Empty)
        );
        assert_eq!(
            skip_reason(10, max, false, false, false, false, false),
            None
        );
    }
}
