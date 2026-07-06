# Polyglot Fixture

This fixture is intentionally small and offline. It exists to pin Lithograph's
first-release repository model against a mixed repository with supported source
languages, unsupported source languages, structured configuration,
documentation, infrastructure, generated/vendor hints, static assets, and
opaque binary data.

The fixture should be used by integration and snapshot tests. It must never
require network access, live services, package installation, Docker pulls, or
model calls.

## Expected Artifact Categories

| Path | Expected category | Support tier | Purpose |
| --- | --- | --- | --- |
| `README.md` | Documentation | StructuredFormat | Repository-level Markdown with links, commands, and code fences |
| `docs/architecture.md` | Documentation | StructuredFormat | Existing architecture evidence and Mermaid diagram |
| `src/python_app/__init__.py` | SourceCode | DeepLanguage | Python package marker and export surface |
| `src/python_app/service.py` | SourceCode | DeepLanguage | Python imports, classes, functions, env reads, and subprocess call |
| `rust/Cargo.toml` | PackageManifest | StructuredFormat | Cargo manifest, binary target, and dependencies |
| `rust/src/lib.rs` | SourceCode | DeepLanguage | Rust public API, trait, struct, function, and env read |
| `rust/src/bin/worker.rs` | SourceCode | DeepLanguage | Rust binary target |
| `config/settings.yaml` | Configuration | StructuredFormat | YAML config with service, image, env, and path references |
| `config/schema.json` | Configuration | StructuredFormat | JSON schema-like structured config |
| `pyproject.toml` | PackageManifest | StructuredFormat | Python package metadata |
| `requirements.txt` | PackageManifest | GenericText | Python dependency list |
| `Dockerfile` | ContainerDefinition | StructuredFormat | Multi-stage container build with image references |
| `docker-compose.yml` | ContainerDefinition | StructuredFormat | Service wiring, ports, env, and build context |
| `.github/workflows/ci.yml` | ContinuousIntegration | StructuredFormat | CI commands across Python, Rust, and web assets |
| `Makefile` | BuildDefinition | GenericText | Extensionless build command file |
| `web/src/App.tsx` | SourceCode | GenericText | Unsupported TSX source fallback |
| `web/index.html` | Template | GenericText | HTML entrypoint/template |
| `assets/logo.svg` | StaticAsset | GenericText | Text static asset with image semantics |
| `data/sample.bin` | BinaryAsset | Opaque | NUL-byte binary data |
| `generated/client.py` | GeneratedSource | GenericText | Generated-file header detection |
| `vendor/example/lib.rs` | SourceCode | GenericText | Vendored source path detection |
| `LICENSE` | Documentation | GenericText | Extensionless legal text |

## Offline Guarantees

- All commands are illustrative and should not be executed by fixture tests.
- Container image references are examples only.
- Package manifests do not need to resolve or install dependencies.
- Binary data is local and tiny.

