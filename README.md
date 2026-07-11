# Lithograph

Lithograph turns a source tree into a queryable knowledge base: a typed
semantic graph, evidence-backed architecture documentation, and a set of
deterministic tools for search, drift detection, and agent integration — all
running locally, offline by default, with no code ever leaving your machine
unless you explicitly configure a model.

Point it at a repository and `init` produces a durable graph, a C4-oriented
Markdown wiki under `docs/lithograph/`, and a store of architecture decision
records — then `update` keeps all of it current as the repository changes,
regenerating only what actually went stale.

## Why Lithograph

- **Deterministic by default.** Without a model API key, `init`/`update` run
  on a built-in mock model — scans, graph construction, caching, tests, and
  every inspection command are fully offline and reproducible. Point Lithograph
  at DeepInfra or any OpenAI-compatible endpoint and the same manifest, graph,
  evidence, and validation flow runs against a real model instead.
- **Evidence-backed, not hallucinated.** Every generated page and answer is
  tied to source evidence; architecture layer detection and confidence scoring
  report `Low`/`Unknown` rather than guessing when there isn't real signal.
- **A real graph, not a text index.** 29 typed relation kinds (calls,
  imports, inheritance, type references, data flow, cross-service protocol
  edges, near-clone similarity, and more), a Cypher-like query subset, and a
  durable LadybugDB projection you can query directly.
- **Built for agents.** 25 MCP tools cover graph queries, three distinct
  search modes, tag/taxonomy lookups, trace/impact analysis, dead-code
  detection, drift detection, ADR CRUD, and subsystem doc generation —
  wired into Codex, Claude, Gemini, Zed, and Aider via `integrate-mcp`.

## What It Produces

Running `lithograph init <repo>` creates:

- `docs/lithograph/*.md`: a C4-oriented wiki — overview, quickstart,
  architecture, workflows, boundaries, configuration, database, key modules,
  and an ADR-and-drift page, plus a `modules/` tree with per-module pages
  grouped by kind (directories, configuration, documentation, infrastructure,
  language packages/crates).
- `.lithograph/graph.json` and `.lithograph/graph/ladybug.lbug`: a
  deterministic semantic graph export and a durable, queryable LadybugDB
  projection (JSON snapshots retained for compatibility and portable
  exports). See [Ladybug graph operations](docs/dev/ladybug-operations.md).
- `.lithograph/manifest.json`: page/task manifest with dependencies,
  evidence, prompt versions, context schema versions, input hashes, and
  output hashes.
- `.lithograph/research/*.json`: deterministic research summaries backing
  repository-level pages.
- `.lithograph/adrs/*.json`: architecture decision records, one file per
  record, managed through the `adr` command or MCP tools.
- `.lithograph/run.json` / `.lithograph/snapshot.json`: run metadata and
  incremental-change state.
- `.lithograph/cache/analysis/`: content-addressed analysis cache backing
  incremental `update` runs.

Generated docs and `.lithograph/` state are excluded from later scans so a
second run never documents its own output.

## Language Coverage

Lithograph parses with tree-sitter and resolves with per-language hybrid
resolvers, at three honestly-reported tiers rather than one blanket claim:

| Tier | Languages | What you get |
| --- | --- | --- |
| **Hybrid resolved** | Python, Rust | Full parse plus cross-file/package symbol resolution |
| **Syntax indexed** | TypeScript, TSX, JavaScript, Go, Java, Kotlin, C#, PHP, C, C++ | Real tree-sitter AST extraction; cross-file resolution not yet wired |
| **Structured formats** | Markdown, YAML, JSON, TOML, Dockerfile, docker-compose, GitHub Actions, SQL, HTML, CSS, GraphQL, Protobuf | Fully indexed structured/config/protocol data |

An optional type-aware resolution pass upgrades type-reference edges for
Python, Rust, TypeScript, JavaScript, Java, C#, Go, C, and C++ using
compiler-adjacent signals (not full type inference or overload resolution).
Beyond these, the language registry carries roadmap entries toward parity
with broader multi-language tools — those are recognized by name/extension
today and fall back to generic text analysis until wired up.

## Requirements

- Rust toolchain managed by `rustup` (pinned in `rust-toolchain.toml`).
- `just` for the documented development commands.
- Optional: Node.js only for `validate-mermaid --node-validator`.
- Optional: `cargo-llvm-cov` only for coverage reports.
- Optional: `sccache` for local Rust compiler caching.

The justfile prefers the `cargo` resolved through `~/.cargo/bin/rustup` so the
pinned toolchain is used even when another Rust installation is present.

## Quickstart

From this directory:

```sh
just toolchain
cargo run -- --help
cargo run -- init fixtures/polyglot
cargo run -- inspect modules fixtures/polyglot
cargo run -- ask fixtures/polyglot "source evidence"
```

After `init`, open the generated Markdown directly:

```sh
ls fixtures/polyglot/docs/lithograph
```

Or generate the static browser viewer:

```sh
cargo run -- viewer fixtures/polyglot
open fixtures/polyglot/.lithograph/viewer/index.html
```

## Core Workflow

Generate documentation for a repository:

```sh
cargo run -- init /path/to/repo
```

Rescan and selectively regenerate only stale pages:

```sh
cargo run -- update /path/to/repo
```

Use deterministic semantic grouping when planning modules:

```sh
cargo run -- init /path/to/repo --semantic-grouping
cargo run -- update /path/to/repo --semantic-grouping
```

Stamp a prompt version into page/task metadata. Changing it forces affected
pages to regenerate on `update`:

```sh
cargo run -- update /path/to/repo --prompt-version v2
```

Poll a repository for staleness against its last recorded snapshot, and
optionally re-run `update` automatically when it drifts:

```sh
cargo run -- watch /path/to/repo
cargo run -- watch /path/to/repo --auto-index
```

Exchange a compressed, team-shareable graph artifact instead of rebuilding it:

```sh
cargo run -- graph export /path/to/repo --output graph.bundle
cargo run -- graph import /path/to/repo graph.bundle
```

## Inspection, Search, and Graph Queries

These commands are deterministic and never call a model.

```sh
cargo run -- inspect artifacts /path/to/repo
cargo run -- inspect graph /path/to/repo
cargo run -- inspect modules /path/to/repo --semantic-grouping
cargo run -- inspect dsm /path/to/repo        # module dependency matrix + cycles
cargo run -- inspect metrics /path/to/repo    # last-run timings, graph size, cache hit rate
```

Scan existing Markdown for likely documentation drift against the current
repository and graph:

```sh
cargo run -- drift /path/to/repo
```

Beyond CLI inspection, the graph supports 29 typed relation kinds (imports,
calls, inheritance, type references, data flow, HTTP routes, config
bindings, near-clone similarity, cross-service protocol edges, and more), a
deliberately narrow Cypher-like query subset (`src/query.rs`) exposed as the
`query_graph` MCP tool, three independent search layers — BM25 full-text
(`src/fts.rs`), pluggable semantic/embedding search (`src/semantic_search.rs`),
and graph-scoped code search (`src/search.rs`) — plus deterministic
architecture layer detection, tension scoring, and typed tagging
(`src/architecture.rs`, `src/graph/tensions.rs`, `src/graph/tags.rs`).

## Architecture Decision Records

Create, read, update, delete, and list ADRs from the CLI (mirrored 1:1 by
MCP tools for agent access):

```sh
cargo run -- adr create /path/to/repo \
  --title "Use LadybugDB for graph storage" \
  --context "Need a durable, queryable projection of the semantic graph" \
  --decision "Adopt LadybugDB as the on-disk graph store"
cargo run -- adr list /path/to/repo
```

## Generated Wiki Tools

Ask a deterministic local question against generated docs:

```sh
cargo run -- ask /path/to/repo "How does configuration work?"
cargo run -- ask /path/to/repo "How does configuration work?" --format json
```

Export generated wiki data in an MCP-style JSON shape:

```sh
cargo run -- mcp-export /path/to/repo
cargo run -- mcp-export /path/to/repo --question "Which modules own the API?"
```

Serve deterministic JSON-line requests over stdin/stdout:

```sh
cargo run -- mcp-server /path/to/repo
```

`mcp-server` exposes 25 tools spanning wiki access, graph queries and schema
introspection, all three search modes, tag/taxonomy lookups, trace-path and
impact analysis, dead-code detection, change detection, drift detection,
architecture layer reporting, ADR CRUD, subsystem document generation and
refinement, and run metrics. See `src/mcp.rs` for the full list.

Generate a lightweight static viewer with navigation, local search, and
Mermaid-ready code blocks:

```sh
cargo run -- viewer /path/to/repo
```

## Agent Integration

Wire Lithograph's MCP server into a coding agent's config. Without
`--target`, every supported agent is detected and reported; nothing is
written unless `--apply` is passed with a specific `--target`:

```sh
cargo run -- integrate-mcp /path/to/repo
cargo run -- integrate-mcp /path/to/repo --target claude --apply
```

Supported targets: `codex`, `claude`, `gemini`, `zed`, `aider`.

`integrate-agents` is the only Lithograph command that edits top-level
`AGENTS.md` or `CLAUDE.md` files. It adds or refreshes a Lithograph reference
section and is idempotent:

```sh
cargo run -- integrate-agents /path/to/repo
```

## Output Quality and Regression Checks

Update golden snapshots for generated docs, manifest, and research artifacts:

```sh
cargo run -- golden fixtures/polyglot --golden-dir tests/golden/polyglot --update
```

Check generated output against snapshots:

```sh
cargo run -- golden fixtures/polyglot --golden-dir tests/golden/polyglot
```

Inspect generated wiki quality — missing page evidence, unresolved
questions, empty Mermaid sections, weak module coverage, missing source
links, and broken generated-doc links:

```sh
cargo run -- quality /path/to/repo
```

Validate Mermaid fences structurally, optionally through a local Node/
mermaid.js validator:

```sh
cargo run -- validate-mermaid /path/to/repo
cargo run -- validate-mermaid /path/to/repo --node-validator scripts/validate-mermaid.mjs
```

Normal tests and `just check-all` do not require Node or network access.

## Model Configuration

Backend selection is environment-driven:

1. DeepInfra when `LITHOGRAPH_DEEPINFRA_API_KEY` is set.
2. OpenAI-compatible API when `LITHOGRAPH_OPENAI_API_KEY` is set.
3. Deterministic mock model when neither API key is set.

DeepInfra:

```sh
export LITHOGRAPH_DEEPINFRA_API_KEY=...
export LITHOGRAPH_DEEPINFRA_MODEL=deepseek-ai/DeepSeek-R1
# optional
export LITHOGRAPH_DEEPINFRA_BASE_URL=https://api.deepinfra.com/v1/openai
export LITHOGRAPH_DEEPINFRA_REASONING_EFFORT=medium
```

OpenAI-compatible:

```sh
export LITHOGRAPH_OPENAI_API_KEY=...
# optional
export LITHOGRAPH_OPENAI_BASE_URL=https://api.openai.com/v1
export LITHOGRAPH_OPENAI_MODEL=gpt-4o-mini
export LITHOGRAPH_OPENAI_REASONING_EFFORT=medium
```

Only `init` and `update` generate page content through the selected model.
Every other command — inspection, drift, quality, golden, Mermaid
validation, ask, search, graph queries, ADRs, MCP export/server, and viewer
generation — operates on local files only, with zero network access.

## Development

Use these commands from this directory:

```sh
just toolchain
just fmt
just fmt-check
just lint
just test
just unit-test
just integration-test
just check-all
```

`just check-all` is the default pre-handoff validation path. It runs
formatting checks, clippy with warnings denied, and the complete test suite
(500+ test functions across unit and integration coverage):

```sh
just check-all
```

Coverage is intentionally separate because it requires `cargo-llvm-cov`:

```sh
cargo install cargo-llvm-cov
just coverage
```

## Repository Layout

- `src/domain/`: stable IDs, artifacts, evidence, and confidence types.
- `src/inventory/`: repository walking, classification, language registry,
  and safety policy.
- `src/analysis/`: deterministic analyzers for supported file types.
- `src/resolve/`: per-language hybrid import/symbol/type resolvers.
- `src/graph/`: semantic graph model, builder, validation, LadybugDB store,
  analytics, communities, tensions, and tags.
- `src/query.rs`: the Cypher-like graph query subset.
- `src/fts.rs`, `src/semantic_search.rs`, `src/search.rs`: full-text,
  semantic, and graph-scoped code search.
- `src/architecture.rs`, `src/drift.rs`, `src/adr.rs`: architecture layer
  detection, documentation drift, and ADR storage.
- `src/external_knowledge.rs`, `src/graph_docs.rs`, `src/docs_model.rs`:
  external knowledge routing and C4-oriented documentation modeling.
- `src/knowledge_agent.rs`, `src/editor_agent.rs`, `src/subsystem_docs.rs`:
  typed research/editor agents and subsystem documentation generation.
- `src/plan.rs`: deterministic and optional semantic module planning.
- `src/generation/`: context construction, model adapters, evidence
  validation, and page rendering.
- `src/orchestrate.rs`: `init` and `update` pipeline.
- `src/manifest.rs`: page/task manifest and version invalidation metadata.
- `src/research.rs`: deterministic research summaries for repository pages.
- `src/ask.rs`, `src/mcp.rs`, `src/mcp_targets.rs`, `src/viewer.rs`,
  `src/watch.rs`: local generated-wiki access, per-agent MCP integration,
  static viewer, and staleness polling.
- `src/golden.rs`, `src/quality.rs`, `src/mermaid.rs`: validation and
  regression tools.
- `fixtures/polyglot/`: representative fixture repository used by tests.
- `tests/`: integration and snapshot coverage.
- `docs/dev/`: design notes — parser spike decisions, prompt/context
  versioning, type-aware resolution, LadybugDB schema and operations, plus
  distribution and security posture.

## Distribution and Security

Ships as a single static Rust binary with no runtime dependencies; tested on
macOS (aarch64/x86_64) and Linux (x86_64/aarch64), with Windows supported on
a best-effort basis. Not yet published to crates.io — install via
`cargo install --path . --locked` from a pinned revision. See
`docs/dev/distribution.md` for the full pre-release checklist.

Lithograph reads only within the canonicalized repository root and honors
`.gitignore`. Secrets and credentials (`.env`, private keys, and similar) are
classified metadata-only — their path and category are recorded, but their
content is never sent to a model or written to disk, with private-key
markers redacted line-by-line even in otherwise-safe files. Only `init` and
`update` ever call a model, only when an API key is configured, and only
with bounded, evidence-scoped excerpts — never a whole-repo dump. See
`docs/dev/security.md` for the complete guarantees that keep `just test` and
`just check-all` offline and deterministic.

## Current Status

Lithograph's local CLI, C4-oriented generated wiki, durable LadybugDB graph,
incremental update path with content-addressed caching, three search modes,
graph query language, ADR store, drift/quality checks, 25-tool MCP server,
and per-agent integration are implemented and covered by 500+ tests, but the
project is still evolving quickly — most notably a browser-based graph
explorer UI (tracked under the LIT-24 backlog epic) is in active design and
not yet built. Prefer `just check-all` before handoff and keep generated
output changes reviewable with `golden`, `quality`, and `validate-mermaid`.
