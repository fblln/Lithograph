# Ladybug graph operations

Lithograph stores its durable graph projection at
`.lithograph/graph/ladybug.lbug`. It also writes the versioned JSON snapshot
under `.lithograph/graph/` and the compatibility export
`.lithograph/graph.json`. Ladybug is the durable backend; JSON remains useful
for compatibility, diagnostics, golden snapshots, and portable export/import.

## Build, inspect, and rebuild

```sh
cargo run -- init /path/to/repo
cargo run -- update /path/to/repo
cargo run -- inspect graph /path/to/repo --format json
```

`init` creates the index; `update` incrementally reuses analysis cache entries
when content and pipeline versions match. To rebuild, remove only the target
repository's `.lithograph/graph/` and `.lithograph/cache/analysis/` directories,
then run `init` again. Do not delete source files or generated docs merely to
rebuild an index.

## Migration and troubleshooting

Snapshot metadata records schema/model migrations. A newer unsupported snapshot
fails explicitly; an older compatible JSON snapshot is migrated on load. If a
Ladybug projection is corrupt, restore from the JSON snapshot by rebuilding
with `init`; the loader never silently treats a corrupt database as empty.

Use `cargo run -- inspect graph /path/to/repo --format json` to compare node and
relation counts, and `cargo run -- graph export /path/to/repo out.lithograph`
for a portable artifact. `graph import` restores compatible exports.

## Queries and workflows

Use typed MCP/query APIs for schema, search, trace, architecture, change impact,
semantic search, and graph statistics. Raw Ladybug/Cypher access is disabled by
default and must only be enabled by a trusted local caller; see
`docs/dev/raw-graph-queries.md`.

For code-health and exploration workflows, inspect graph relations and run the
MCP `get_architecture`, `trace_path`, `search_semantic`, and metrics surfaces.
The embedded graph explorer is an optional consumer of the same graph data; it
does not change indexing or query semantics.

## Analytics snapshots

Analytics records are tied to one graph snapshot and include algorithm name,
algorithm version, filter scope, and creation metadata. Recompute a metric when
the graph snapshot id, algorithm version, or filter scope changes; the typed
`MetricSnapshot::invalidation_key()` encodes exactly those inputs. Node metrics,
communities, profiles, and health findings remain separate versioned records,
so analytics recomputation never mutates source parser facts.

Community detection uses the versioned deterministic Leiden local-moving
algorithm. It accepts a combined graph or an explicit relation-kind scope and
persists stable summaries with membership, cohesion, conductance, boundary
edges, representative and bridge nodes, and dominant package nodes. Bump the
Leiden algorithm version whenever its clustering semantics change.

## Code-health thresholds

Health detectors are local and deterministic. Conservative defaults report a
god-class candidate at degree 12, a bridge bottleneck at degree 8, a
low-cohesion community at 25% or lower, and shotgun-surgery risk at five
co-change neighbors. Callers may provide `HealthThresholds` to tune those
values for a repository; each result records the actual metric input and a
typed graph investigation query.

## Validation

```sh
just check-all
```

This is the single offline validation entry point: formatting, strict Clippy,
and all unit/integration tests. Use the committed polyglot golden test when
reviewing intentional graph-output changes.
