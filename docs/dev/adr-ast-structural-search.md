# ADR: AST-by-example structural search (LIT-86.12)

Status: **Accepted — native (via tree-sitter queries), build deferred behind demand.**
Date: 2026-07-20. Supersedes the DRAFT-1 placeholder for structural search.

## Context

LIT-86 evaluates AST-by-example structural search — matching code by a
syntactic pattern (e.g. "every function whose body calls `eval`") rather than
by text or embedding. CocoIndex's `code_ast`/`code_match` demonstrate the
technique. This ADR records whether Lithograph should **import** CocoIndex
code, **adapt** it, build a **native** capability, or record **no-go**.

The decisive prior fact: Lithograph already depends on `tree-sitter` and ships
grammar crates for Python, Rust, TypeScript/TSX, JavaScript, Java, Kotlin, Go,
PHP, C, C++, C#, HTML, CSS, and SQL (`src/analysis/tree_sitter_adapter.rs`), and
LIT-86.14 added a parse-once arena (`src/analysis/parsed_source.rs`) that already
yields a reusable parse per artifact content hash. Tree-sitter itself provides a
native **query language** (S-expression patterns with captures, wildcards, and
predicates) that is exactly AST-by-example matching.

## 1. Pattern semantics (AC#1)

The user-facing surface maps onto tree-sitter's query grammar:

- **Single-node capture** — `(call_expression function: (identifier) @fn)` binds
  one node to `@fn`.
- **Variadic sibling capture** — a quantified node `(argument_list (_)* @args)`
  captures zero-or-more siblings.
- **Anonymous wildcard** — `(_)` matches any named node; `_` any node.
- **Literals** — string literals in the pattern match node text
  (`(string) @s (#eq? @s "eval")` via predicates).
- **Language selection** — a required `language:` field selects the grammar; a
  pattern is compiled against exactly one grammar, since node kinds differ per
  language.

This is a thin, documented projection over tree-sitter queries — no new pattern
engine is invented.

## 2. Prototype feasibility across languages (AC#2)

A prototype is a `tree_sitter::Query` compiled against each adapter's
`Language`, run over the parse-once arena's tree. Representative queries and
their grammar support:

| Query class | Pattern shape | Grammars |
|---|---|---|
| Definition | `(function_definition name: (_) @n)` | all |
| Call | `(call_expression function: (_) @callee)` | all with call nodes |
| Decorator/attribute | `(decorator (_) @d)` / `(attribute_item …)` | Python, Rust, Java, C# |
| Import | `(import_statement …)` | per existing `import_kinds` |
| Route | grammar-specific handler patterns | where routes are syntactic |
| Unsafe pattern | `(call_expression function: (identifier) @f (#eq? @f "eval"))` | all |

Every grammar Lithograph already loads supports `tree_sitter::Query`
compilation; node kinds are exactly those the adapters already enumerate
(`definition_kinds`, `import_kinds`, …), so the prototype reuses existing
knowledge. **Conclusion: feasible natively with zero new grammar dependencies.**

## 3. Measurements to gate a build (AC#3)

Because the matcher reuses the arena's existing parse, the marginal cost is
query compilation + a single tree walk per artifact:

- **Correctness / FP / FN** — measured against authored fixtures per language
  (the LIT-86.13 harness pattern), asserting exact match sets.
- **Parse reuse** — zero extra parses: the arena's `parse_count` instrument
  (LIT-86.14) proves the matcher shares the analyzer/chunker parse.
- **Latency / memory** — one `Query::new` (cached per pattern) + one
  `QueryCursor` walk; bounded by node count. No embedding, no network.
- **Binary size / compile time** — **zero** new crates for the native path
  (tree-sitter's query API is already linked). Importing CocoIndex would add
  `code_ast`/`code_match` and their transitive deps.
- **Malformed source** — tree-sitter produces a partial tree with `ERROR`
  nodes; queries run over it and simply match less, never panic (the arena
  already handles this and records fallback status).

These are cheap to gate on the offline suite; a full cross-8-language
measurement spike is the first task if/when a build is scheduled.

## 4. CocoIndex compatibility (AC#4)

CocoIndex `code_ast` wraps `tree-sitter` too, but pins its own grammar crate
versions. Adopting its code would couple Lithograph's tree-sitter and grammar
versions to CocoIndex's, risking version skew against the crates the analyzers
already use, and against the Rust toolchain. The native path uses the grammars
already vendored, so there is **no version-compatibility surface at all**.

## 5. Security analysis (AC#5)

- **No ReDoS**: patterns are AST queries, not regexes; there is no backtracking
  text engine to attack. Text predicates (`#eq?`, `#match?`) apply to already-
  parsed node text and are bounded per node.
- **Pathological patterns / traversal limits**: a query cursor must run under a
  node-visit budget and a match budget; a deeply-quantified pattern is capped
  and returns a truncation flag rather than running unbounded.
- **Result budgets**: bounded, paginated results (same contract as
  `search_code_semantic`).
- **Unsafe file exposure**: structural search reads the same safe corpus as the
  rest of the pipeline — `ModelExposurePolicy::Never` artifacts are never
  parsed for matching, exactly as they are excluded from chunking.
- **Untrusted patterns via MCP/serve**: a pattern is compiled with a hard size
  limit and a language allowlist; a compilation error is a typed, bounded
  diagnostic, never a panic or an unbounded walk.

## 6. License and maintenance (AC#6)

CocoIndex is Apache-2.0; importing or adapting `code_ast`/`code_match` requires
carrying its NOTICE, tracking upstream changes, and re-vetting on each bump —
ongoing burden for functionality tree-sitter already provides. The native path
copies/adapts **no** third-party code beyond the tree-sitter crates already in
the tree, so there is no new attribution or upstream-tracking obligation.

## Decision matrix

| Option | New deps | Version risk | Maint. burden | Verdict |
|---|---|---|---|---|
| **Native (tree-sitter queries)** | none | none | none beyond existing grammars | **Chosen** |
| Adapt CocoIndex `code_match` | its deps | grammar/toolchain skew | Apache-2.0 tracking | Rejected |
| Import CocoIndex as-is | + Python risk | high | high | Rejected |
| No-go | — | — | — | Rejected (technique is valuable and cheap) |

## Decision (AC#7)

**Native.** If and when structural search is built, implement it over the
parse-once arena using `tree_sitter::Query`, exposed with the same filter/budget/
diagnostic contract as semantic search. Do **not** import or adapt CocoIndex
code: it would re-wrap tree-sitter while adding version-compatibility surface and
Apache-2.0 tracking for no capability gain.

**Build is deferred**, not scheduled now: the retrieval stack (chunking → vector
index → enrichment → ranking → surface) is the delivered priority, and
structural search is additive. DRAFT-1 remains Draft.

### API sketch

```rust
// Compiled once per (language, pattern), cached by pattern hash.
struct StructuralQuery { language: SyntaxIndexedLanguage, query: tree_sitter::Query }

struct StructuralMatch { chunk_id: String, evidence: EvidenceRef, captures: BTreeMap<String, EvidenceRef> }

fn structural_search(
    arena: &ParsedSourceArena,
    pattern: &str,
    language: SyntaxIndexedLanguage,
    budget: MatchBudget,   // node-visit cap, match cap
) -> Result<Vec<StructuralMatch>, StructuralError>;
```

Matches carry chunk ids and evidence spans, so results compose with the graph
exactly like semantic hits (an attachment of kind `StructuralIndex`, LIT-86.16).

### Conditions to revisit the deferral

Schedule the native build when any holds:

1. Users request pattern-based code search (MCP/CLI) more than semantic search
   for a concrete workflow (e.g. security lint: "find every `eval` call").
2. A downstream feature (mutation testing, refactor suggestions) needs precise
   syntactic matches the semantic index cannot provide.
3. Tree-sitter's query API gains capture features that make a previously
   awkward pattern class ergonomic.

### Conditions to revisit "reject CocoIndex"

Reconsider importing/adapting CocoIndex only if it evolves a capability with no
tree-sitter-native equivalent (e.g. cross-file structural patterns) that
Lithograph needs and cannot cheaply build.
