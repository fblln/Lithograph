# Hybrid Semantic Retrieval and Incremental Indexing — Architecture (LIT-86.1)

Foundation design for the LIT-86 epic. Defines the contracts every follow-on
task builds on **before** any storage crate is chosen or any persisted format
changes. CocoIndex is a conceptual reference (component paths, target-state
reconciliation); it is **not** a dependency mandate.

Scope of this document: contracts and decisions only. No code in this task.

## 0. Current state and named weaknesses

Grounded in the tree as of 2026-07-20 (paths corrected from the stale task
references):

| Concern | Today | File | Weakness |
|---|---|---|---|
| Semantic index | Rebuilt on every search: hashes the whole graph JSON and re-embeds all nodes | `src/retrieval/semantic_search.rs` (`SemanticSearch::search` → `EmbeddingIndex::build`) | Never persisted; `search_index` (reuse path) exists but is unwired. O(graph) per query. |
| FTS index | One `fts-index.json`, rewritten whole | `.lithograph/fts-index.json`, `src/retrieval/fts.rs` | No per-record reconciliation. |
| Doc page hashes | 9 repo-wide pages depend on `all_dependencies` under one `repository_input_hash` | `src/manifest.rs` (`PageManifestBuilder`) | Any module change invalidates every repo page. |
| Model identity | `GenerationTask.model` stored but ignored by the compat check | `src/manifest.rs:100` (`is_version_compatible_with`) | Model-only change does **not** invalidate. |
| Pipeline identity | Hand-bumped `GRAPH_BUILD_PIPELINE_VERSION` constant | `src/run.rs:39` (`graph_pipeline_version`) | Manual; drifts from actual logic (→ LIT-86.7). |
| Node embedding | `collect_documents` embeds symbol/artifact/config node *text* | `src/retrieval/semantic_search.rs` | Re-encodes metadata the graph already indexes structurally — the lowest-value slice (removed, §2). |
| Analysis cache | Content-hash + `AnalyzerKind` + `ANALYSIS_CACHE_VERSION=12` per JSON file | `src/analysis/cache.rs` | The ownership/versioning pattern to reuse: component-owned, versioned, miss-is-safe. |

Persisted state layout (`.lithograph/`): `graph/current.json`, `manifest.json`,
`fts-index.json`, `cache/analysis/`, `research/`, `layout/`, `analytics/`,
`adrs/`. No embedding index is persisted at all today.

## 1. The canonical/derived seam: span vs embedding (AC#1)

The naive split is "graph vs chunks", which forces a second identity system and
a back-reference layer to reconnect chunks to nodes — reinventing what the graph
already does (`EvidenceRef` is already `path + span`; a chunk *is* a span). So
the seam is drawn one level deeper:

- **A chunk *span* is canonical graph state.** Chunking is deterministic
  (LIT-86.2), so a chunk node asserts only "bytes X..Y of file F exist and
  contain this text" — as evidence-backed as a `Symbol`, and equally
  reproducible. Chunk spans become first-class **`GraphNode::Chunk`** nodes with
  real `Contains` edges from their owning artifact/symbol.
- **An *embedding* is derived state.** The vector is provider-dependent, large,
  and non-deterministic across models. It never enters the canonical graph; it
  lives in a separate, provider-tagged store keyed by chunk `content_hash`.

This satisfies AC#1 more precisely than the naive split: the fuzzy thing (the
vector) is exactly what stays out of the graph, while the structural thing (the
span) unifies with it. Chunks never "compete" as a semantic graph — a chunk node
asserts a byte range, never a typed relation, confidence, or resolution.

**Why keep both layers at all.** They are complementary, not redundant:

- The **graph** is precise and typed but only knows what analyzers modeled; it
  cannot serve a natural-language/intent query ("where do we handle expired auth
  tokens?") unless you already know the symbol.
- **Embeddings** give recall over raw source text the graph never modeled
  (bodies, comments), but cannot assert structure — asked "what calls this?" a
  vector index returns text that *resembles* a call, never the true edge, with
  no provenance.

Composed: a fuzzy query lands on a chunk node → its `Contains` edge reaches the
owning symbol → graph traversal gives the precise, evidence-backed answer
(callers, config, service). The vector supplies recall; the graph supplies
precision. Neither alone produces that composite. The vector layer earns its
keep specifically on NL/intent/exploratory queries; for purely structural usage
it is overhead, so it is optional at query time, never on the ingest critical
path for structural correctness.

Everything derived is **reconstructable** from `(graph snapshot + chunk nodes,
source bytes, provider identity, pipeline identity)`. Deleting all derived state
and rebuilding must reproduce byte-identical canonical output (clean == cached
== incremental equivalence — an existing repo invariant).

## 2. Identities, one embedding unit, collisions (AC#2)

Chunks reuse the **existing typed-string node id convention** (`kind:payload`,
e.g. `symbol:path#qualified`). No separate chunk identity system, no back-ref
machinery — a chunk is a node, its links are edges.

- **Chunk node id:** `chunk:{artifact_path}#{ordinal}`, `ordinal` = 0-based
  index in the file's deterministic chunk sequence (LIT-86.2 owns the chunker).
  Path + ordinal is stable and **not** content-derived, so identical code in two
  files yields two distinct nodes (no false sharing). Consistent with today's
  `symbol:path#qualified`.
- **Parser-fallback chunks** (no AST) get a marker: `chunk:{path}#{ordinal}~raw`,
  so a later AST-capable run never silently reuses a coarse fallback chunk's
  vector.
- **`content_hash = blake3(chunk_bytes)`** is stored on the chunk node. It is the
  **embedding-invalidation key**, distinct from identity: two chunks with equal
  `content_hash` share one cached vector (dedup at the embedding layer) while
  keeping distinct node ids and distinct edges.

**One embedding unit.** Node-level metadata embedding (`collect_documents`) is
removed. A hit on a symbol's own signature chunk already lands on the symbol via
the `Contains` edge, so embedding node metadata is redundant. Clean tripartite
split, each doing one job:

- **graph** → structure (what calls what, provenance, confidence)
- **vectors** → meaning over source bytes (chunk nodes)
- **FTS/BM25** → names and lexical lookup (env vars, config keys — the
  source-less nodes that node-embedding used to cover)

Collision / edge-case rules:

| Case | Rule |
|---|---|
| Identical chunks (same bytes, different files) | Distinct node ids + edges; one shared vector via `content_hash`. |
| Moved/renamed file | New path ⇒ new chunk nodes; old nodes orphan and are swept (§3). Vectors survive via `content_hash`, so a rename re-embeds nothing. |
| Duplicate code within a file | Distinct ordinals ⇒ distinct nodes. |
| Overlapping chunks | Chunker guarantees total order; ordinals never collide. |
| Parser-fallback chunks | `~raw` marker prevents cross-generation vector reuse. |

Identity is a pure function of `(artifact_path, ordinal, fallback_flag)`;
`content_hash` a pure function of bytes. Neither depends on wall-clock, absolute
paths, or run id (correctness-artifact invariant).

## 3. Ownership, deletion, transactions, recovery, migration (AC#3)

**Concrete reconcilers over a shared doctrine — no premature framework.** There
are exactly three derived stores. Rather than a generic component engine up
front (over-abstraction for three known, differently-shaped consumers, and
`AnalysisCache` — the proven pattern — is deliberately concrete), each store is
its own concrete reconciler that follows one documented **doctrine**. The shared
trait is deferred to LIT-86.9 *only if* the third implementation reveals a real
common shape.

The three stores and their canonical/derived status:

| Store | Path | Status |
|---|---|---|
| Chunk nodes | `graph/chunks.json` (partitioned, §storage) | **Canonical** graph fragment |
| Chunk index (vector + BM25) | `derived/chunk-index/` | Derived, provider-tagged |
| Doc/name FTS | `fts-index.json` | Derived (separate lifecycle) |

Chunk vector and chunk BM25 are folded into **one** `chunk_index` record
(`{content_hash, provider_id, vector, terms}`): same keyspace, always reconcile
together, cannot drift out of sync. Doc/name FTS keeps its own lifecycle (it
indexes generated docs and source-less nodes, not chunks) — only the parts that
genuinely move together are merged.

**Doctrine** (generalizes `AnalysisCache`):

- One store owns its keyspace; nothing else writes into it.
- **Target-state reconciliation:** on `update`, recompute the desired keyset
  from current inputs, diff against the persisted keyset, emit add/edit/delete.
  Deletion is **orphan-driven** — a key on disk but absent from the recomputed
  target is deleted. Correct add/edit/rename/delete with no mutation-event log.
- **Miss-is-safe:** a missing/corrupt entry is a recompute, never a correctness
  error (`AnalysisCache` doctrine).
- **Transaction = one store's reconciliation.** Write new/changed entries to
  temp files, `fsync`, atomic-rename into place, then sweep orphans **last**.
  - Crash before rename → previous consistent generation; a miss is safe.
  - Crash after some renames, before sweep → stale extras linger but are never
    *wrong*; re-diffed and swept next run.
  - Readers never see a torn write (only fully-renamed files).
- **Per-store manifest** records `(store_version, provider_id, pipeline_id,
  key → content_hash)` so the next run diffs without re-reading payloads.
- **Rebuild is the migration.** Each store carries a `version: u32` (mirroring
  `ANALYSIS_CACHE_VERSION`). A bump treats the store as empty and rebuilds;
  versioned filenames make old files inert and sweepable. No in-place schema
  migration — always correct because derived state is reconstructable.

## 4. Invalidation inputs (AC#4)

A derived `chunk_index` entry is valid iff **all** identity inputs match:

```
valid(entry) ⇔ entry.content_hash    == current chunk bytes hash
             ∧ entry.provider_id      == current provider/model/config identity
             ∧ entry.pipeline_id      == current pipeline-logic fingerprint
             ∧ entry.store_version     == current store version
```

- **Provider identity** (fixes the model-only-change gap): `provider:model:
  config-digest` — dims, normalization, purpose (doc vs query, LIT-86.15).
  Today `EmbeddingIndex.model_identity` exists but is coarse and unpersisted;
  this makes it a first-class, persisted invalidation input. The manifest gap
  (`is_version_compatible_with` ignoring `model`) is fixed under LIT-86.8 by
  folding `model` into the compat check.
- **Pipeline identity** (LIT-86.7): replaces the hand-bumped
  `GRAPH_BUILD_PIPELINE_VERSION` with an automatic fingerprint over the code
  paths that produce the output. Until 86.7 lands, the manual constant *is* the
  pipeline_id — same contract, better computation later.

Model-only change is explicit: bytes and pipeline unchanged, `provider_id`
changes ⇒ every vector re-embeds, chunk **nodes** and graph untouched (the span
didn't move, only its embedding did — a direct payoff of the §1 seam).

## 5. Safety and model exposure (AC#5)

Remote embedding is a network egress of source text, governed by the **existing**
policy in `docs/dev/security.md` — no new policy, one reused gate.

Before any chunk is sent to a **remote** embedding provider it passes the same
`src/inventory/safety.rs` filter that gates model prompts today:

- `ModelExposurePolicy::Never` artifacts (secrets/credentials: `.env`, `*.pem`,
  private keys — classified `metadata_only`) are **excluded before chunking**,
  not filtered after. They never become chunk nodes eligible for remote
  embedding. (A metadata-only artifact may still exist as a plain `Artifact`
  node; it simply produces no `Chunk` children.)
- Private-key markers redacted line-by-line still apply to chunk bytes.
- Binary/generated/vendored content: excluded via the same walk classification
  that keeps it out of prompts and `scan_exclude_globs`.

The offline `MockEmbeddingProvider` (deterministic feature-hashing) touches no
network, so **normal tests embed everything locally** — the safety gate is only
material with a real remote provider configured, exactly like the prompt path.
Preserves the parent invariant: no normal/PR/CI command makes a network call or
needs a live model.

## 6. Decision matrix (AC#6)

| Axis | **Native (chosen)** | CocoIndex Rust SDK (0.1.0) | CocoIndex/ccc sidecar |
|---|---|---|---|
| Offline-deterministic gate | Full control; trivially satisfied | Depends on SDK internals staying offline | Separate process; hard to prove offline |
| No-Python invariant | Guaranteed | Risk: transitive Python runtime | High risk: pulls Python as undeclared runtime |
| Maturity | Reuses proven in-repo patterns (`AnalysisCache`, `JsonStore`) | 0.1.0, unstable API surface | Extra service to run/version |
| Determinism / canonical JSON | Native `JsonStore`, already canonical | Must verify SDK serialization is stable | Cross-process ordering hazards |
| Ops surface | None (files under `.lithograph/`) | New crate + its deps | New daemon, lifecycle, ports |
| Reuse of CocoIndex ideas | Concepts only (component paths, target state) | Code reuse | Code reuse |

**Decision: native.** Implement chunk nodes, the chunk index, and reconciliation
in Rust, reusing the `AnalysisCache` ownership/versioning pattern. CocoIndex
contributes the *shape* (target-state reconciliation, purpose-typed embeddings),
not a runtime.

**Vector storage — native, but not JSON-of-floats and not a vector DB.** A vector
DB (lancedb, sqlite-vec) is unjustified over-engineering at this scale; JSON
arrays of f32 at real provider dims (768–1536) are wasteful to parse. Lazy
middle: the `chunk_index` record stays a JSON envelope (metadata, `content_hash`,
`provider_id`, `terms`) but the vector field is **base64 of little-endian f32**
— one line each way, no new store, no float parsing, no dependency. Ranking is
brute-force cosine over the repo's chunks (a few ms at this scale).
`// ponytail: brute-force cosine + base64 f32; add a binary sidecar / ANN when a
repo's chunk count makes it measurable`.

**Chunk-node storage.** Canonical but physically partitioned into
`graph/chunks.json` (same node-id/edge system as the main snapshot) so
structural-only consumers don't pay the parse cost of fine-grained nodes.
Unified identity, partitioned storage.

**Rollback.** Every store is versioned and lives under `.lithograph/` (a build
artifact). Rollback of the derived layer = delete `derived/chunk-index/` and drop
its module; the canonical graph is untouched. Rolling back chunk *nodes*
themselves = stop emitting `GraphNode::Chunk` and drop `graph/chunks.json` (a
partition, not a format migration of the main snapshot). The `EmbeddingProvider`
trait boundary lets a future SDK/sidecar back-end slot behind the same contract
without touching callers.

## 7. Follow-on mapping and validation (AC#7)

| Task | Modules | Version constant |
|---|---|---|
| 86.2 Chunking | `src/analysis/chunk.rs` (new), shared parsed-source (86.14) | `CHUNK_SCHEMA_VERSION` |
| 86.3 Vector index persist/reconcile | `src/retrieval/chunk_index.rs` (new) | `CHUNK_INDEX_VERSION` |
| 86.4 Chunk enrichment | `GraphNode::Chunk` + `Contains` edges in `src/graph/model.rs`, builder | folds into graph pipeline |
| 86.5 Graph-constrained ranking | `src/retrieval/chunk_index.rs` (traversal-based filters) | — |
| 86.6 CLI/MCP/viewer surface | `src/agent/mcp.rs`, `src/viewer.rs`, `src/commands.rs` | — |
| 86.7 Logic fingerprints | `src/run.rs` (`graph_pipeline_version` → fingerprint) | `GRAPH_BUILD_PIPELINE_VERSION` |
| 86.8 Apply fingerprints | `src/manifest.rs` (`is_version_compatible_with` + `model`), `src/analysis/cache.rs` | `ANALYSIS_CACHE_VERSION` |
| 86.9 Reconciliation doctrine | documented convention; shared trait only if 3rd impl proves it | per-store version |
| 86.10 Graph-fragment reconcile | `graph/chunks.json` partition, builder | graph snapshot version |
| 86.11 BM25 records | folded into `chunk_index` (`terms`); doc/name FTS stays `src/retrieval/fts.rs` | `CHUNK_INDEX_VERSION` / `FTS_INDEX_VERSION` |
| 86.12 AST-by-example (DRAFT-1) | decision only until go | — |
| 86.13 Gates | `src/lab/` | lab contract version |
| 86.14 Parsed-source product | `src/analysis/` | `CHUNK_SCHEMA_VERSION` shared |
| 86.15 Embedding purposes | `EmbeddingProvider` trait | folded into `provider_id` |
| 86.16 Graph-bound attachments | `chunk_index` keyed by chunk node id | `CHUNK_INDEX_VERSION` |
| 86.17 Invalidation explain plans | `src/run.rs`, `src/commands.rs` | — |

**One-command validation.** `just check-all` stays the ordinary gate;
`just baseline-pr` covers correctness for inventory/analysis/graph/resolution
changes (chunk nodes are graph nodes, so they fall under existing graph baselines
automatically). The hybrid-specific gate — clean == cached == incremental
equivalence, deletion/rename/crash-recovery, provider-only invalidation — lands
under LIT-86.13 as offline lab scenarios on the existing corpus harness. No new
command.

## Non-goals

- No new runtime dependency, daemon, network egress, or vector DB in
  normal/PR/CI paths.
- No competing semantic graph; embeddings never enter the canonical graph and
  chunk nodes never assert typed relations or confidence.
- No in-place schema migration; rebuild is the migration.
- No generic reconciliation framework before three concrete stores justify it.
