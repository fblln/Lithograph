# LadybugDB graph schema

LIT-24.1 defines the durable storage contract for Lithograph's LadybugDB
projection. It is deliberately a projection, not a replacement for the typed
Rust graph: `Graph`, `GraphNode`, `Relation`, and their enums remain the only
canonical semantic model. Ladybug makes that model queryable and persists
analytics results alongside a particular graph snapshot.

Ladybug uses a structured property-graph schema: node/relationship tables,
explicit property types, and a primary key for each node table. Lithograph
therefore creates all schema tables up front and uses an on-disk `.lbug`
database under `.lithograph/` in the adapter task. This follows the official
Ladybug [schema and persistence model](https://docs.ladybugdb.com/get-started/)
and [Rust API setup](https://docs.ladybugdb.com/tutorials/rust/).

## Snapshot identity and graph facts

`Snapshot` is the root node table. Its primary key is a deterministic
`snapshot_id`; it records the schema/model/algorithm versions, repository
hash, creation time, and graph counts. A snapshot owns every projected
`CodeNode` through `SNAPSHOT_OWNS_NODE`.

`CodeNode` is one table for every `GraphNode` variant. `node_key` is the
snapshot-scoped primary key; `graph_node_id`, `node_label`, `node_kind`, and
the indexed display/path/language fields preserve the typed discriminants.
`payload_json` is canonical serialized Rust payload used only for lossless
forward compatibility. Queries use the explicit typed columns, never infer a
kind from JSON.

`GraphRelation` is a relationship table from `CodeNode` to `CodeNode`. It
stores the stable relation id/kind, confidence, resolution, resolver strategy,
language provenance, and canonical evidence array. A relation is not promoted
to a pseudo-node: Ladybug relationship rows retain traversal performance while
the Rust `Relation` type continues to own validity.

| Rust contract | Ladybug representation | Required persisted fields |
| --- | --- | --- |
| `GraphNode` | `CodeNode` | label, subtype/kind, name/path/language, evidence span, full payload |
| `Relation` | `GraphRelation` | kind, confidence, resolver metadata, evidence array, full payload |
| `EvidenceRef` | node columns plus relation `evidence_json` | artifact path, start/end line, structured path |
| `GraphStoreMetadata` | `Snapshot` | model/schema/algorithm versions, hashes and counts |

The adapter constructs table keys from validated Rust ids; it must reject
empty ids, mismatched snapshot ids, unsupported enum strings, or evidence that
does not cite an artifact in the same snapshot. This keeps invalid graph states
out of both the Rust boundary and the database.

## Analytics projection

Analytics are snapshot-scoped and never overwrite graph facts.

| Table | Meaning |
| --- | --- |
| `MetricSnapshot` / `NodeMetric` | reproducible metric run and scalar per-node values/ranks |
| `Community` | deterministic community metadata and cohesion |
| `SemanticProfile` | semantic class/filter classification with confidence |
| `HealthFinding` | rule result, severity, status, evidence, and diagnostic payload |
| `SchemaMigration` | append-only ledger of applied schema/model/algorithm changes |

The relationship tables connect metric rows to the node measured, community to
members, profiles to classified nodes, and findings to their affected nodes.
This means metric recalculation only replaces one metric snapshot; it never
alters a historical graph snapshot or changes code facts.

## Versioning and migrations

Three independent versions are written on every `Snapshot` and migration
ledger row:

1. `LADYBUG_SCHEMA_VERSION` changes for storage-table or column semantics.
2. Existing `GRAPH_MODEL_VERSION` changes when Rust graph meaning changes.
3. `LADYBUG_ALGORITHM_VERSION` changes when persisted analytics/classification
   meaning changes.

The upcoming adapter applies migrations in a single transaction, records a
deterministic `ladybug-schema:<from>-><to>` id, and never opens a newer schema
for writing. Additive fields require a new schema version plus a backfill or a
documented nullable/default policy. Destructive changes require export,
validate, rebuild, and atomic replacement of the `.lbug` directory. Algorithm
changes create a new metric snapshot rather than mutating earlier results.

The executable DDL catalog is
[`src/graph/ladybug_schema.rs`](../../src/graph/ladybug_schema.rs). LIT-24.2
will bind it to Ladybug's `lbug` Rust crate and add idempotent transactional
creation, migration, write, and read paths.

## Local native dependency note

The `lbug` crate is linked as an embedded native dependency. On Apple Silicon
macOS, install `openssl@3` with Homebrew before building; `build.rs` adds its
standard `/opt/homebrew/opt/openssl@3/lib` search path because Ladybug's
current prebuilt static archive does not propagate those OpenSSL link flags.
