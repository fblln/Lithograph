# Baseline and diagnostic lab

`lithograph-lab` is Lithograph's deterministic correctness and performance
harness. It never calls an LLM. Normal runs never use the network: external
repositories must first be fetched into the content-addressed local cache.

## One-command suites

```sh
just baseline-pr
just baseline-fetch merge
just baseline-merge
just baseline-fetch nightly
just baseline-nightly
```

`baseline-pr` uses only `fixtures/diagnostic` and is required on pull
requests. `baseline-merge` adds the pinned Flask, ripgrep, and Full Stack
FastAPI cases. `baseline-nightly` also includes NestJS and uv, then records
five warm-cache samples as median/MAD performance observations. Raw samples
are append-only, and reviewed relative budgets gate only the dedicated runner.
Correctness hashes never include wall-clock values or machine identity.

## Investigating a failure

Every run is stored under `.lithograph-lab/runs/<run-id>/` with its inventory,
final graph, communities, assertions, JSONL events, replay bundle, and one JSON
artifact per graph pass. A CI failure prints its run id and exact replay and
explain commands.

```sh
cargo run --bin lithograph-lab -- inspect RUN
cargo run --bin lithograph-lab -- inspect RUN --stage resolution
cargo run --bin lithograph-lab -- explain RUN --assertion ASSERTION_ID
cargo run --bin lithograph-lab -- minimize RUN
cargo run --bin lithograph-lab -- minimize RUN --materialize /tmp/local-minimized-fixture
just baseline-replay RUN
```

The read-only MCP stdio server gives agents the same typed operations. It
implements JSON-RPC initialization, standard tool discovery, typed input
schemas, and `tools/call`; baseline mutation is not exposed:

```sh
cargo run --bin lithograph-lab -- mcp
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"inspect_stage","arguments":{"run":"RUN","stage":"enrichment"}}}
```

## Baseline governance

Baselines under `lab/baselines/` are reviewable summaries, not complete
third-party graph snapshots. Updating one is always explicit:

```sh
# Preview the semantic diff and print a fresh token.
cargo run --bin lithograph-lab -- accept RUN --reason "Reviewed change ..."

# Perform the atomic write with exactly that reviewed token.
cargo run --bin lithograph-lab -- accept RUN --reason "Reviewed change ..." --confirm TOKEN
```

The token binds the run, reason, current baseline, and semantic diff, so stale
or blind acceptance is rejected. Acceptance also rejects dirty runs and CI
writes, uses an atomic rename, and retains the previous review metadata. Known
defects still require an exact issue signature, Backlog task, and UTC expiry.

## Contract migration and performance modes

Schema migrations are mechanical and never accept semantic changes:

```sh
cargo run --bin lithograph-lab -- migrate lab/baselines/diagnostic.json
cargo run --bin lithograph-lab -- migrate OLD_RUN/manifest.json --apply
cargo run --bin lithograph-lab -- benchmark --suite pr --samples 5 --mode cold
cargo run --bin lithograph-lab -- benchmark --suite nightly --samples 5 --mode warm-cache --gate
cargo run --bin lithograph-lab -- benchmark --suite nightly --case nestjs --samples 5 --mode community-only --gate
```

Supported benchmark modes are `cold`, `warm-cache`, `incremental`, and
`no-op`. `community-only` is a separate graph-replay mode: it verifies the
content-addressed graph hash and expected community output, then measures only
adjacency, movement, and summary phases without invoking `GraphBuilder`. Its
raw samples retain graph hash, normalized scope, algorithm version, machine
fingerprint, and exact replay command. Whole-pipeline and community-only
histories use distinct mode directories and cross-mode comparison is rejected.
Each summary links to its raw samples and machine-specific history.
Graph-stage artifacts use stable snake-case names; wall-clock durations are
reported separately as observations.

Community analytics expose separate `community_adjacency_us`,
`community_movement_us`, and `community_summary_us` observations alongside
deterministic work counters. The optimized implementation uses sorted compact
node indices, an active movement queue, and one summary edge pass. Repeated
lab and health computations can reuse snapshots keyed by the canonical graph
hash, normalized scope, and `LEIDEN_ALGORITHM_VERSION`.

Near-clone detection exposes per-phase observations
`component_clone_tokenization_us`, `component_clone_candidate_generation_us`
(the exact-safe rare-token prefix filter), `component_clone_exact_verification_us`,
`component_clone_cache_lookup_us`, and `component_clone_merge_us`, alongside the
deterministic counters `stage_enrichment_clone_comparisons` (pairs reaching exact
verification), `stage_enrichment_clone_prefilter_pairs`, and
`stage_enrichment_clone_cache_hit`. `warm_cache` benchmarking clears the
persisted clone snapshot before each sample -- exactly as it clears the
community snapshot -- so the tokenization, candidate-generation, and
exact-verification budgets measure the implementation rather than a snapshot hit;
the snapshot's no-op reuse is covered by unit tests instead. Reviewed relative
budgets gate the three dominant clone phases for `uv` and `nestjs`.

The lab also evaluates the versioned weighted scope recorded in
`lab/community-scope-decision.json`. It records edge count, runtime, ARI, NMI,
curated pair accuracy, cohesion, and conductance without changing the
production `Combined` default. A default change requires explicit reviewed
baseline acceptance.

The corpus manifest pins repository URL, commit, Git tree, license, tier, and
expectation file. Fetch verifies all of those plus a clean checkout. Replay
bundles reference third-party source by immutable identity and do not copy it.
