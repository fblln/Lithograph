# Prompt and Context Versions

Lithograph records prompt and context schema versions per generated page and
generation task in `.lithograph/manifest.json`.

Prompt versions are supplied through `lithograph init --prompt-version` and
`lithograph update --prompt-version`. Context schema versions are owned by
`TaskKind::context_schema_version` in `src/manifest.rs` and should change only
when the model input contract for that page kind changes.

On `update`, a page regenerates when its source input hash changes, its prompt
version changes, or its context schema version changes. This keeps prompt and
context migrations explicit without requiring content changes in the repository.
