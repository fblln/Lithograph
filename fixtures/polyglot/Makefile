.PHONY: test rust-test python-test web-lint image

test: python-test rust-test web-lint

python-test:
	python -m pytest

rust-test:
	cargo test --manifest-path rust/Cargo.toml

web-lint:
	npm --prefix web run lint

image:
	docker build -t ghcr.io/example/route-api:dev .

