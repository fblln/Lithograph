# Lithograph

Lithograph is a Rust-based repository knowledge compiler. The first release is a
local CLI that inventories heterogeneous repositories, builds a semantic graph,
and generates evidence-backed documentation.

## Development Commands

Use these commands from this directory:

```sh
make toolchain
make fmt
make fmt-check
make lint
make test
make unit-test
make integration-test
make coverage
make check-all
```

`make check-all` is the default pre-handoff validation path. It runs formatting
checks, lint/static analysis, and the complete test suite.

The Makefile prefers the rustup-managed Cargo at `~/.cargo/bin/rustup` so the
toolchain declared in `rust-toolchain.toml` is used even when another Rust
installation is also present on the machine.

Coverage is intentionally separate because it requires `cargo-llvm-cov`:

```sh
cargo install cargo-llvm-cov
make coverage
```

## Current CLI

```sh
cargo run -- --help
cargo run -- --version
```
