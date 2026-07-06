# Parser spike decisions (LIT-1.13)

Decision record for the Python and Rust analyzer parser choice, required before
implementing LIT-1.14 (Python analyzer) and LIT-1.15 (Rust analyzer).

## Method

Two throwaway spike crates (not committed) parsed the polyglot fixture's real
source files plus a deliberately malformed variant of each, using each
candidate crate, and compared:

- Whether spans are exposed per node, and whether they are line-based or
  byte-only.
- What happens on a syntax error: whole-file failure vs partial recovery.
- Dependency shape (pure Rust vs native/C build step, subprocess use).

Python fixture: `fixtures/polyglot/src/python_app/service.py` (imports, a
class with three methods, a module function) plus a broken variant with a
missing colon and an unclosed call.

Rust fixture: `fixtures/polyglot/rust/src/lib.rs` (a `use`, a trait, a struct,
two `impl` blocks, a free function) plus a broken variant with a missing
semicolon and an unclosed block.

## Python: rustpython-parser vs Tree-sitter

| | rustpython-parser 0.4 | tree-sitter + tree-sitter-python 0.25 |
| --- | --- | --- |
| Spans | Byte ranges only (`range: 87..98`); line numbers require a separate line-index over the source | Native `start_position()`/`end_position()` row+column per node |
| Well-formed fixture | Full AST: imports, class, methods, function, all present | Same facts, plus reusable generic node walk |
| Broken fixture | **Whole-file failure**, zero nodes recovered: `invalid syntax. Got unexpected token Newline at byte offset 128` | `has_error=true`, but still recovers `import_statement`, and both unaffected `function_definition`/`class_definition` nodes elsewhere in the file |
| Cross-language reuse | Python-only API/AST shape | Same crate/API pattern reused for Rust (and any future language grammar) |
| Build dependency | Pure Rust | Needs a C compiler (`cc`) to build the bundled grammar |

**Decision: Tree-sitter (`tree-sitter` + `tree-sitter-python`).**

Lithograph inspects arbitrary, sometimes-imperfect third-party repositories,
and every graph fact is evidence-backed by a line span (`SourceSpan`). A
parser that returns *nothing* the moment one function in a large file has a
syntax error is a poor fit for that goal — one bad snippet would zero out an
entire file's contribution to the graph. Tree-sitter degrades gracefully and
exposes native line/column spans without a secondary byte-to-line index.

Accepted tradeoff: tree-sitter needs a C compiler at build time. This is not
actually a new requirement — `blake3` (already a dependency) pulls in `cc` as
a build-dependency today, so the project's build already assumes a C
toolchain is available.

## Rust: Tree-sitter vs syn + cargo_metadata

| | syn 2.0 (+ proc-macro2 span-locations) | tree-sitter + tree-sitter-rust 0.24 |
| --- | --- | --- |
| Spans | `Span::start()/end()` gives line+column directly | `start_position()`/`end_position()` gives row+column directly |
| Well-formed fixture | Full item list: `use`, trait, struct, two impls (with trait name), methods, function | Same shape via generic node walk |
| Broken fixture | **Whole-file failure**, zero items recovered: `cannot parse string into token stream` | `has_error=true`, but still recovers the `use_declaration`, `struct_item`, and the unaffected trailing `function_item`; the two `impl` blocks fall inside the error span and are not recovered |
| Cross-language reuse | Rust-only | Same crate/API/extraction pattern as the Python analyzer |

`cargo_metadata --no-deps` was also spiked directly against
`fixtures/polyglot/rust/Cargo.toml` and cleanly returned the crate name and
both targets with resolved kinds (`fixture_worker: [Lib]`, `worker: [Bin]`).
It shells out to the `cargo` binary and only succeeds for files that belong
to a resolvable manifest — it is not a source-AST tool and cannot replace
per-file symbol extraction for arbitrary `.rs` files (e.g. the fixture's
`vendor/example/lib.rs`, which has no owning `Cargo.toml`).

**Decision: Tree-sitter (`tree-sitter` + `tree-sitter-rust`) for per-file
source extraction (modules, `use`, structs, traits, impls, functions, spans),
plus `cargo_metadata --no-deps` for authoritative workspace/crate/target/
dependency facts that TOML-only parsing (LIT-1.12's `CargoProfileAnalyzer`)
cannot resolve (e.g. resolved target kinds, workspace membership).**

`syn` is ruled out as the source-AST engine for the same reason
rustpython-parser was ruled out for Python: one syntax error anywhere in a
file drops all extraction for that file, and it does not generalize to other
languages the way the tree-sitter integration does.

## Consequences for LIT-1.14 / LIT-1.15

- Add `tree-sitter`, `tree-sitter-python`, `tree-sitter-rust`, and
  `cargo_metadata` to `Lithograph/Cargo.toml` when implementing those tasks,
  not before — no spike code or dependency lands in this task.
- Both language analyzers should share a small internal tree-sitter walking
  helper (node kind dispatch + span conversion) rather than duplicating the
  walk loop per language.
- `cargo_metadata` calls should be scoped to Rust package manifests only and
  must tolerate failure (e.g. vendored/standalone `.rs` files, or a manifest
  `cargo metadata` cannot resolve) by falling back to tree-sitter-only facts
  for that artifact.
