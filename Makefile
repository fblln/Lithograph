RUSTUP ?= $(HOME)/.cargo/bin/rustup
CARGO ?= $(shell if [ -x "$(RUSTUP)" ]; then "$(RUSTUP)" which cargo; else command -v cargo; fi)

.PHONY: toolchain fmt fmt-check lint test unit-test integration-test coverage check-all run help

toolchain:
	@if [ -x "$(RUSTUP)" ]; then \
		"$(RUSTUP)" show; \
	else \
		echo "rustup was not found at $(RUSTUP). Install rustup or set RUSTUP=/path/to/rustup."; \
		exit 1; \
	fi

fmt:
	$(CARGO) fmt --all

fmt-check:
	$(CARGO) fmt --all -- --check

lint:
	$(CARGO) clippy --all-targets --all-features -- -D warnings

test:
	$(CARGO) test --all-targets --all-features

unit-test:
	$(CARGO) test --lib --bins --all-features

integration-test:
	$(CARGO) test --test cli --all-features

coverage:
	@if $(CARGO) llvm-cov --version >/dev/null 2>&1; then \
		$(CARGO) llvm-cov --all-features --workspace --fail-under-lines 100; \
	else \
		echo "cargo-llvm-cov is required for coverage. Install with: cargo install cargo-llvm-cov"; \
		exit 1; \
	fi

check-all: fmt-check lint test

run:
	$(CARGO) run -- --help

help:
	@echo "Available commands:"
	@echo "  make fmt              Format all Rust code"
	@echo "  make fmt-check        Check Rust formatting"
	@echo "  make lint             Run clippy as static analysis with warnings denied"
	@echo "  make test             Run unit and integration tests"
	@echo "  make unit-test        Run unit tests"
	@echo "  make integration-test Run CLI integration tests"
	@echo "  make coverage         Run 100% line coverage check with cargo-llvm-cov"
	@echo "  make check-all        Run formatting, lint/static analysis, and tests"
	@echo "  make run              Run the CLI help output"
