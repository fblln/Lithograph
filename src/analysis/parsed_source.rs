//! Shared parsed-source product (LIT-86.14).
//!
//! One safe source buffer parses at most once per content hash per pipeline
//! run, and every syntax consumer -- analyzers, syntax-aware chunking
//! (LIT-86.2), and structural search (LIT-86.12) -- reads the same product
//! instead of standing up its own parsing stack.
//!
//! Tree-sitter trees are *not* retained or serialized. Everything a consumer
//! needs is projected out of the tree at parse time -- normalized facts, a
//! reusable line index, and top-level definition byte boundaries -- so there
//! are no syntax-node lifetimes to manage, `Send`/`Sync` is trivial, and the
//! bounded-memory concern reduces to holding normalized data, not live trees.
//! The parse product is keyed by content hash and deliberately excludes the
//! artifact path, so two files with identical bytes parse once and each caller
//! pairs the shared product with its own path.

// ponytail: the arena and ParsedSource are exercised by tests today; the
// production consumer is the chunk-node builder (LIT-86.4), which wires this
// in as the single parse point. LineIndex is already used by the chunker.
// Drop this allow when 86.4 lands.
#![allow(dead_code)]

use crate::analysis::tree_sitter_adapter::{
    TreeSitterAdapterOutput, TreeSitterParseStatus, TreeSitterParserAdapter,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

/// Byte offset of the start of each line, so a byte offset maps to a one-based
/// line/column in one pass. Shared by the chunker and any evidence-span
/// consumer. CRLF is handled by not counting a trailing `\r` as a column
/// character, so CRLF and LF inputs produce identical columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineIndex {
    line_starts: Vec<usize>,
}

impl LineIndex {
    /// Builds a line index over `text`.
    pub(crate) fn new(text: &str) -> Self {
        let mut line_starts = vec![0usize];
        for (offset, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push(offset + 1);
            }
        }
        Self { line_starts }
    }

    /// One-based line and one-based character column of a byte offset that lies
    /// on a char boundary. `text` must be the same string the index was built
    /// from.
    pub(crate) fn line_col(&self, text: &str, offset: usize) -> (u32, u32) {
        let line_idx = match self.line_starts.binary_search(&offset) {
            Ok(exact) => exact,
            Err(next) => next.saturating_sub(1),
        };
        let line_start = self.line_starts[line_idx];
        let column_chars = text[line_start..offset]
            .chars()
            .filter(|c| *c != '\r')
            .count();
        (
            u32::try_from(line_idx + 1).unwrap_or(u32::MAX),
            u32::try_from(column_chars + 1).unwrap_or(u32::MAX),
        )
    }
}

/// The content-derived parse product for one source buffer. A pure function of
/// the source bytes (plus the chosen grammar), so it is safe to share across
/// every consumer and to cache by content hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ParsedSource {
    /// `blake3` of the source bytes; the cache key and identity.
    pub content_hash: String,
    /// Stable language id (`"rust"`, `"go"`, ...), or `"text"` when no grammar
    /// was available.
    pub language_id: String,
    /// The immutable source text.
    pub text: String,
    /// Reusable byte -> line/column index.
    pub line_index: LineIndex,
    /// Whether a grammar produced a tree or the source fell back to text.
    pub status: TreeSitterParseStatus,
    /// Normalized syntax facts (definitions, imports, symbols, comments,
    /// errors), identical to `TreeSitterParserAdapter::parse`.
    pub facts: TreeSitterAdapterOutput,
    /// Byte offsets of top-level definitions, for syntax-aware chunking.
    pub syntax_boundaries: Vec<usize>,
}

impl ParsedSource {
    /// Parses `text` with `adapter` when one is available, else records a
    /// text-only fallback product. `language_id` names the language even when
    /// no adapter exists, so callers can still report it.
    fn build(text: &str, language_id: &str, adapter: Option<&TreeSitterParserAdapter>) -> Self {
        let content_hash = blake3::hash(text.as_bytes()).to_hex().to_string();
        let (facts, syntax_boundaries) = match adapter {
            Some(adapter) => adapter.parse_indexed(text),
            None => (
                TreeSitterAdapterOutput::fallback(language_id, "no tree-sitter adapter available"),
                Vec::new(),
            ),
        };
        Self {
            content_hash,
            language_id: facts.language_id.clone(),
            text: text.to_owned(),
            line_index: LineIndex::new(text),
            status: facts.status.clone(),
            facts,
            syntax_boundaries,
        }
    }

    /// True when a grammar successfully produced a tree.
    pub(crate) fn is_parsed(&self) -> bool {
        matches!(self.status, TreeSitterParseStatus::Parsed)
    }
}

/// Parses source buffers at most once per content hash within one pipeline run
/// and hands out shared products. Sequential by construction -- the pipeline
/// analyzes artifacts one at a time -- so a single-threaded interior-mutable
/// cache is sufficient; `parse_count` proves the parse-once guarantee in tests.
#[derive(Debug, Default)]
pub(crate) struct ParsedSourceArena {
    cache: RefCell<HashMap<String, Rc<ParsedSource>>>,
    parses: Cell<usize>,
}

impl ParsedSourceArena {
    /// Creates an empty arena.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Returns the shared parse product for `text`, parsing it only on the
    /// first request for that content. `language_id`/`adapter` are used only
    /// on a miss; two calls with identical bytes return the same `Rc` and
    /// increment the parse count once.
    pub(crate) fn get_or_parse(
        &self,
        text: &str,
        language_id: &str,
        adapter: Option<&TreeSitterParserAdapter>,
    ) -> Rc<ParsedSource> {
        let key = blake3::hash(text.as_bytes()).to_hex().to_string();
        if let Some(existing) = self.cache.borrow().get(&key) {
            return Rc::clone(existing);
        }
        // Build outside the borrow so a re-entrant consumer can't deadlock the
        // RefCell; a duplicate build under contention is impossible here
        // (single-threaded), so a plain insert is safe.
        let parsed = Rc::new(ParsedSource::build(text, language_id, adapter));
        self.parses.set(self.parses.get() + 1);
        self.cache.borrow_mut().insert(key, Rc::clone(&parsed));
        parsed
    }

    /// Number of actual parses performed -- one per distinct content hash.
    pub(crate) fn parse_count(&self) -> usize {
        self.parses.get()
    }

    /// Releases all cached products (AC#6 bounded retention): the arena holds
    /// only normalized data, so dropping the map frees it immediately with no
    /// dangling syntax nodes.
    pub(crate) fn clear(&self) {
        self.cache.borrow_mut().clear();
    }
}

#[cfg(test)]
mod tests {
    use super::{LineIndex, ParsedSourceArena};
    use crate::analysis::tree_sitter_adapter::{TreeSitterParseStatus, TreeSitterParserAdapter};

    /// AC#2: sequential consumers of one content hash cause exactly one parse,
    /// and a second identical request returns the same shared product.
    #[test]
    fn identical_content_parses_once() {
        let arena = ParsedSourceArena::new();
        let adapter = TreeSitterParserAdapter::rust();
        let source = "fn a() {}\nfn b() {}\n";

        let first = arena.get_or_parse(source, "rust", Some(&adapter));
        let second = arena.get_or_parse(source, "rust", Some(&adapter));

        assert_eq!(arena.parse_count(), 1);
        assert!(std::rc::Rc::ptr_eq(&first, &second));
    }

    /// AC#10: duplicate content in different files still parses once; callers
    /// supply their own path, which is not part of the cached product.
    #[test]
    fn duplicate_content_in_two_files_shares_one_parse() {
        let arena = ParsedSourceArena::new();
        let adapter = TreeSitterParserAdapter::go();
        let source = "package main\nfunc main() {}\n";

        let a = arena.get_or_parse(source, "go", Some(&adapter));
        let b = arena.get_or_parse(source, "go", Some(&adapter));

        assert_eq!(arena.parse_count(), 1);
        assert_eq!(a.syntax_boundaries, b.syntax_boundaries);
        assert_eq!(a.content_hash, b.content_hash);
    }

    /// AC#3/#4: the shared product's facts are identical to a direct
    /// `adapter.parse`, and one product serves both fact and boundary
    /// consumers from a single parse.
    #[test]
    fn shared_facts_equal_direct_parse_and_expose_boundaries() {
        let arena = ParsedSourceArena::new();
        let adapter = TreeSitterParserAdapter::go();
        let source = "package main\n\nfunc alpha() {}\n\nfunc beta() {}\n";

        let parsed = arena.get_or_parse(source, "go", Some(&adapter));
        let direct = adapter.parse(source);

        assert_eq!(parsed.facts, direct);
        assert!(parsed.is_parsed());
        // Two top-level funcs => two definition boundaries, each at a `func`.
        assert_eq!(parsed.syntax_boundaries.len(), 2);
        for boundary in &parsed.syntax_boundaries {
            assert!(source[*boundary..].starts_with("func"));
        }
    }

    /// AC#7: no adapter yields a typed fallback product, not a panic, and no
    /// boundaries.
    #[test]
    fn missing_adapter_is_a_typed_fallback() {
        let arena = ParsedSourceArena::new();
        let parsed = arena.get_or_parse("some prose\nmore prose\n", "text", None);

        assert!(!parsed.is_parsed());
        assert!(matches!(
            parsed.status,
            TreeSitterParseStatus::FallbackDetected { .. }
        ));
        assert!(parsed.syntax_boundaries.is_empty());
        assert_eq!(arena.parse_count(), 1);
    }

    /// AC#6: clearing the arena frees products; a later request re-parses.
    #[test]
    fn clear_releases_products() {
        let arena = ParsedSourceArena::new();
        let adapter = TreeSitterParserAdapter::rust();
        let source = "fn a() {}\n";

        arena.get_or_parse(source, "rust", Some(&adapter));
        arena.clear();
        arena.get_or_parse(source, "rust", Some(&adapter));

        assert_eq!(arena.parse_count(), 2);
    }

    /// AC#8: line/column conversion round-trips for Unicode and CRLF, counting
    /// columns in characters and treating CRLF like LF.
    #[test]
    fn line_index_handles_unicode_and_crlf() {
        let lf = "abc\ndéf\n";
        let index = LineIndex::new(lf);
        // Byte offset of 'd' on line 2 is 4; it is column 1.
        assert_eq!(index.line_col(lf, 4), (2, 1));
        // 'f' sits after the two-byte 'é', so it is byte 7 but char column 3.
        let f_offset = lf.find('f').unwrap_or(0);
        assert_eq!(index.line_col(lf, f_offset), (2, 3));

        let crlf = "abc\r\ndef\r\n";
        let crlf_index = LineIndex::new(crlf);
        // Start of line 2 ('d') is column 1 in CRLF just as in LF.
        let d_offset = crlf.find('d').unwrap_or(0);
        assert_eq!(crlf_index.line_col(crlf, d_offset), (2, 1));
    }
}
