set shell := ["sh", "-eu", "-c"]

rustup := `printf '%s\n' "${RUSTUP:-$HOME/.cargo/bin/rustup}"`
cargo := `if [ -x "${RUSTUP:-$HOME/.cargo/bin/rustup}" ]; then "${RUSTUP:-$HOME/.cargo/bin/rustup}" which cargo; else command -v cargo; fi`
sccache := `if [ -n "${SCCACHE:-}" ]; then printf '%s\n' "$SCCACHE"; else command -v sccache 2>/dev/null || true; fi`
cargo_env := if sccache == "" { "" } else { "RUSTC_WRAPPER='" + sccache + "'" }

# List available commands.
default:
    @just --list

# Show the active Rust toolchain.
toolchain:
    @if [ -x '{{rustup}}' ]; then \
        '{{rustup}}' show; \
    else \
        echo "rustup was not found at {{rustup}}. Install rustup or set RUSTUP=/path/to/rustup."; \
        exit 1; \
    fi

# Format all Rust code.
fmt:
    @{{cargo_env}} '{{cargo}}' fmt --all

# Check Rust formatting.
fmt-check:
    @{{cargo_env}} '{{cargo}}' fmt --all -- --check

# Run clippy as static analysis with warnings denied.
lint:
    @{{cargo_env}} '{{cargo}}' clippy --all-targets --all-features -- -D warnings

# Run unit and integration tests.
test:
    @{{cargo_env}} '{{cargo}}' test --all-targets --all-features

# Run unit tests.
unit-test:
    @{{cargo_env}} '{{cargo}}' test --lib --bins --all-features

# Run CLI integration tests.
integration-test:
    @{{cargo_env}} '{{cargo}}' test --test cli --all-features

# Run 100% line coverage check with cargo-llvm-cov.
coverage:
    @if {{cargo_env}} '{{cargo}}' llvm-cov --version >/dev/null 2>&1; then \
        {{cargo_env}} '{{cargo}}' llvm-cov --all-features --workspace --fail-under-lines 100; \
    else \
        echo "cargo-llvm-cov is required for coverage. Install with: cargo install cargo-llvm-cov"; \
        exit 1; \
    fi

# Run formatting, lint/static analysis, and tests.
check-all: fmt-check lint test

# Fetch and verify immutable external corpus repositories. Network is used only here.
baseline-fetch suite="merge":
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- corpus fetch --suite '{{suite}}'

# Run the hermetic, offline correctness baseline required on every pull request.
baseline-pr:
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- check --suite pr

# Run the offline fixture plus pinned medium-repository correctness baselines.
baseline-merge:
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- check --suite merge

# Run the full correctness corpus and five-sample machine-specific performance report.
baseline-nightly:
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- check --suite nightly
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- benchmark --suite nightly --samples 5 --mode warm-cache --gate
    @just baseline-community-nightly

# Benchmark only community phases from verified persisted NestJS and uv graphs.
baseline-community-nightly:
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- benchmark --suite nightly --case nestjs --samples 5 --mode community-only --gate
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- benchmark --suite nightly --case uv --samples 5 --mode community-only --gate

# Replay one content-addressed diagnostic run.
baseline-replay run:
    @{{cargo_env}} '{{cargo}}' run --quiet --bin lithograph-lab -- replay '{{run}}'

# Run the CLI help output.
run:
    @{{cargo_env}} '{{cargo}}' run -- --help

# Show sccache compiler cache statistics.
cache-stats:
    @if [ -n '{{sccache}}' ]; then \
        '{{sccache}}' --show-stats; \
    else \
        echo "sccache was not found on PATH. Install it or set SCCACHE=/path/to/sccache."; \
        exit 1; \
    fi

# Reset sccache compiler cache statistics.
cache-zero:
    @if [ -n '{{sccache}}' ]; then \
        '{{sccache}}' --zero-stats; \
    else \
        echo "sccache was not found on PATH. Install it or set SCCACHE=/path/to/sccache."; \
        exit 1; \
    fi
