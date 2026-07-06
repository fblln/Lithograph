# Lithograph

Lithograph is a local repository knowledge compiler. It inventories a source
tree, builds a typed semantic graph, plans documentation modules, and writes an
evidence-backed wiki under `docs/lithograph/`.

The default path is deterministic and credential-free: without model API keys,
Lithograph uses a mock model so scans, generated metadata, tests, and local
inspection commands can run offline. When configured with DeepInfra or an
OpenAI-compatible endpoint, `init` and `update` can call a real model while
preserving the same manifest, graph, evidence, and validation flow.

## What It Produces

Running `lithograph init <repo>` creates:

- `docs/lithograph/*.md`: repository overview, quickstart, architecture,
  workflow, boundary, configuration, and module pages.
- `.lithograph/graph.json`: deterministic semantic graph export.
- `.lithograph/manifest.json`: page/task manifest with dependencies, evidence,
  prompt versions, context schema versions, input hashes, and output hashes.
- `.lithograph/research/*.json`: deterministic research summaries used by
  repository-level pages.
- `.lithograph/run.json` and `.lithograph/snapshot.json`: run metadata and
  incremental-change state.
- `.lithograph/cache/analysis/`: content-addressed analysis cache.

Generated docs and `.lithograph/` state are excluded from later scans so a
second run does not document its own output.

## Requirements

- Rust toolchain managed by `rustup`.
- `make` for the documented development commands.
- Optional: Node.js only when using `validate-mermaid --node-validator`.
- Optional: `cargo-llvm-cov` only for coverage reports.

This repo pins its Rust toolchain in `rust-toolchain.toml`. The Makefile
prefers the `cargo` resolved through `~/.cargo/bin/rustup` so the pinned
toolchain is used even when another Rust installation is present.

## Quickstart

From this directory:

```sh
make toolchain
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

## Inspection Commands

These commands are deterministic and do not call a model.

```sh
cargo run -- inspect artifacts /path/to/repo
cargo run -- inspect artifacts /path/to/repo --format json

cargo run -- inspect graph /path/to/repo
cargo run -- inspect graph /path/to/repo --format json

cargo run -- inspect modules /path/to/repo
cargo run -- inspect modules /path/to/repo --semantic-grouping --format json
```

Use `drift` to scan existing Markdown for likely documentation drift against
the current repository and graph:

```sh
cargo run -- drift /path/to/repo
cargo run -- drift /path/to/repo --format json
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

Supported tools are:

- `read_wiki_structure`
- `read_wiki_contents`
- `ask_question`

Example request:

```json
{"id":1,"tool":"ask_question","params":{"question":"source evidence"}}
```

Generate a lightweight static viewer with navigation, local search, and
Mermaid-ready code blocks:

```sh
cargo run -- viewer /path/to/repo
cargo run -- viewer /path/to/repo --output-dir .lithograph/viewer
```

The viewer renders Mermaid diagrams when the browser/runtime provides a global
`mermaid` object.

## Output Quality and Regression Checks

Update golden snapshots for generated docs, manifest, and research artifacts:

```sh
cargo run -- golden fixtures/polyglot --golden-dir tests/golden/polyglot --update
```

Check generated output against snapshots:

```sh
cargo run -- golden fixtures/polyglot --golden-dir tests/golden/polyglot
```

Inspect generated wiki quality:

```sh
cargo run -- quality /path/to/repo
cargo run -- quality /path/to/repo --format json
```

The quality report covers missing page evidence, unresolved questions, empty
Mermaid sections, weak module coverage, missing source links, and broken
generated-doc links.

Validate Mermaid fences structurally:

```sh
cargo run -- validate-mermaid /path/to/repo
cargo run -- validate-mermaid /path/to/repo/docs/lithograph/overview.md
```

Optionally invoke a local Node validator. The validator receives one Mermaid
diagram on stdin and should exit nonzero on parser/render errors:

```sh
cargo run -- validate-mermaid /path/to/repo --node-validator scripts/validate-mermaid.mjs
```

Normal tests and `make check-all` do not require Node or network access.

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
Inspection, drift, quality, golden, Mermaid structural validation, ask,
MCP export/server, and viewer generation operate on local files.

## Agent Instruction Integration

`integrate-agents` is the only Lithograph command that edits top-level
`AGENTS.md` or `CLAUDE.md` files. It adds or refreshes a Lithograph reference
section and is idempotent.

```sh
cargo run -- integrate-agents /path/to/repo
```

## Development

Use these commands from this directory:

```sh
make toolchain
make fmt
make fmt-check
make lint
make test
make unit-test
make integration-test
make check-all
```

`make check-all` is the default pre-handoff validation path. It runs formatting
checks, clippy with warnings denied, and the complete test suite:

```sh
make check-all
```

Coverage is intentionally separate because it requires `cargo-llvm-cov`:

```sh
cargo install cargo-llvm-cov
make coverage
```

## Repository Layout

- `src/domain/`: stable IDs, artifacts, evidence, and confidence types.
- `src/inventory/`: repository walking, classification, and safety policy.
- `src/analysis/`: deterministic analyzers for supported file types.
- `src/graph/`: semantic graph model, builder, and validation.
- `src/plan.rs`: deterministic and optional semantic module planning.
- `src/generation/`: context construction, model adapters, evidence
  validation, and page rendering.
- `src/orchestrate.rs`: `init` and `update` pipeline.
- `src/manifest.rs`: page/task manifest and version invalidation metadata.
- `src/research.rs`: deterministic research summaries for repository pages.
- `src/ask.rs`, `src/mcp.rs`, `src/viewer.rs`: local generated-wiki access.
- `src/golden.rs`, `src/quality.rs`, `src/mermaid.rs`, `src/drift.rs`:
  validation and regression tools.
- `fixtures/polyglot/`: representative fixture repository used by tests.
- `tests/`: integration and snapshot coverage.
- `docs/dev/`: design notes for parser and prompt/context version decisions.

## Current Status

Lithograph is an early local CLI. The generated wiki, graph, manifest,
incremental update path, quality checks, MCP-style access, and viewer are
implemented, but the project is still evolving quickly. Prefer `make check-all`
before handoff and keep generated output changes reviewable with `golden`,
`quality`, and `validate-mermaid`.
