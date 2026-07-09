# Distribution

Lithograph is not yet published to crates.io or shipping prebuilt binaries.
This document describes how it is installed today, what a release artifact
looks like once packaging lands, and the checklist to follow when cutting one.

## Supported Platforms

Lithograph is a single-binary Rust CLI with no runtime dependencies beyond
what `cargo build` produces. It is developed and tested on:

- macOS (aarch64, x86_64)
- Linux (x86_64, aarch64)

Windows is not actively tested. Nothing in the codebase is platform-specific
(no shell-outs beyond `git rev-parse HEAD`, no POSIX-only APIs), so a Windows
build is expected to work but is unverified; treat it as best-effort until a
CI runner exercises it.

The Rust toolchain is pinned in `rust-toolchain.toml` (`stable`, with
`rustfmt`, `clippy`, and `llvm-tools-preview`). Building from source requires
only `rustup` and `make`; see the README's Requirements section.

## Installing Today

Until published, install from source or from a git checkout:

```sh
git clone https://github.com/fblln/Lithograph.git
cd Lithograph
make toolchain
cargo install --path . --locked
```

`cargo install --path . --locked` builds a release binary and places it on
`$PATH` via `~/.cargo/bin`. Use `--locked` so the install always uses the
exact dependency versions this repository was tested against, not whatever
`cargo update` would resolve today.

To use a pinned revision without cloning:

```sh
cargo install --git https://github.com/fblln/Lithograph.git --locked
```

## Release Artifacts (Planned)

No release pipeline exists yet. When one is added, each tagged release should
produce:

- A source tarball (what GitHub already generates automatically per tag).
- Prebuilt binaries for `aarch64-apple-darwin`, `x86_64-apple-darwin`,
  `x86_64-unknown-linux-gnu`, and `aarch64-unknown-linux-gnu`, built with
  `cargo build --release --locked` under the pinned toolchain.
- A `SHA256SUMS` file covering every binary artifact, so an installer can
  verify what it downloaded before running it.

Each binary artifact should be a single static-ish executable (`lithograph`
or `lithograph.exe`); there is no separate runtime, config directory, or
plugin set to package alongside it. `docs/lithograph/` and `.lithograph/` are
per-repository generated output, never shipped with the binary.

## Distribution Checklist

Before tagging a release:

- [ ] `make check-all` passes (format check, clippy with warnings denied, full
      test suite) on the toolchain pinned in `rust-toolchain.toml`.
- [ ] `cargo test --test golden_snapshot` passes against the committed
      snapshots in `tests/golden/polyglot/` -- no unreviewed generated-output
      drift.
- [ ] `Cargo.toml` version bumped; `Cargo.lock` committed and matches.
- [ ] `Cargo.toml` `repository`, `license`, and `description` fields are
      accurate (also checked by `cargo package --list` / `cargo publish
      --dry-run` if publishing to crates.io).
- [ ] `LICENSE` file present and matches the `license` field in `Cargo.toml`.
- [ ] README quickstart commands still work against a fresh clone (the
      commands under `## Quickstart` are the actual smoke test).
- [ ] No credentials, API keys, or `.env`-style files are staged for commit
      (see `docs/dev/security.md`).
- [ ] If prebuilt binaries are produced: built with `--locked` under the
      pinned toolchain, checksummed, and the checksums published alongside
      the binaries.
- [ ] Release notes describe user-visible CLI/behavior changes, not internal
      refactors -- match the tone of existing commit messages.

This checklist is the reviewable artifact required by this task; it is not
automated. Wiring it into CI (a GitHub Actions release workflow) is future
work once binary distribution is actually needed.
