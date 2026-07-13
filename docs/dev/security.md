# Security Posture

This document describes what Lithograph reads, writes, and sends over the
network, and the guarantees that keep its normal test suite offline and
deterministic.

## Local Code Access

Lithograph runs entirely against the local filesystem path given as its
`<path>` argument. It never accesses paths outside that root: repository
walking (`src/inventory/walk.rs`) canonicalizes the root once and only
descends into it, honoring `.gitignore`/`.git/info/exclude` via the `ignore`
crate the same way `git` itself would skip files.

Two content classes are handled specially before anything is read into a
model prompt (`src/inventory/safety.rs`):

- Paths that look like secrets or credentials (`.env`, `*.pem`, private key
  files, etc.) are classified `metadata_only` -- their path and category are
  recorded, but their content is never read into a model prompt or written to
  `.lithograph/` output.
- Content matching private-key markers (`-----BEGIN ... PRIVATE KEY-----`) is
  redacted line-by-line even in files that otherwise pass the safety check,
  in case a legitimate config file embeds one inline.

## What Gets Sent to a Model

Only `init` and `update` call a language model, and only when
`LITHOGRAPH_DEEPINFRA_API_KEY` or `LITHOGRAPH_OPENAI_API_KEY` is set (see the
README's Model Configuration section). Without one of those variables set,
every command -- including `init`/`update` -- uses the deterministic,
offline `MockModel` and makes no network call at all.

When a real model is configured, only bounded, evidence-scoped excerpts
assembled by `src/generation/context.rs` are sent -- never whole-repository
dumps. `ModelExposurePolicy::Never` artifacts (the metadata-only class above)
are excluded from every excerpt before a prompt is built, not filtered
afterward.

## Generated Artifacts

Everything Lithograph writes stays inside the target repository:

- `docs/lithograph/*.md`: generated documentation, safe to commit and review
  like any other source change.
- `.lithograph/`: graph export, manifest, run metadata, research summaries,
  and the content-addressed analysis cache. Also safe to commit, but treat it
  as a build artifact -- it is fully derived from repository content and is
  excluded from later scans (`scan_exclude_globs`) so a second run never
  documents its own output.

Nothing is written outside the target repository. Lithograph has no daemon,
no telemetry, and no home-directory or system-wide state, with one explicit,
user-invoked exception: `serve` (see below).

## Opt-in Config Writes

Three commands write files outside `docs/lithograph/` and `.lithograph/`,
and all three are explicit, single-purpose, and idempotent:

- `integrate-agents`: the only command that edits top-level `AGENTS.md` or
  `CLAUDE.md`. It only touches a marked section
  (`<!-- lithograph:begin -->` / `<!-- lithograph:end -->`) and only in files
  that already exist -- it never creates a new instruction file.
- `integrate-mcp`: registers Lithograph's MCP server in a target coding
  agent's project-scoped config (`.mcp.json`, `.codex/config.toml`,
  `.gemini/settings.json`, or `.zed/settings.json`). Detection
  (`integrate-mcp <path>` with no `--target`) never writes anything;
  `--target` alone only previews; a file is written only with `--target`
  *and* `--apply`, and merges into whatever the target already has rather
  than overwriting it.
- `watch --auto-index`: `watch` alone only polls and reports staleness. It
  runs a real `update` (and therefore writes `docs/lithograph/` and
  `.lithograph/`) only when `--auto-index` is explicitly passed.

No other command writes outside the target repository, and none of these
three ever run unless the corresponding command or flag is invoked directly.

## Local Graph Explorer Server (`serve`)

`serve` is the one command that starts a listening process, and it only
runs when invoked directly -- no other command starts it implicitly.

- **Binds `127.0.0.1` only.** There is no `--host` flag; the bind address
  is not configurable, so the server can never be made reachable from
  another machine.
- **Rejects non-loopback `Host` and `Origin` headers.** Every request is
  checked before reaching a handler: a `Host` header that doesn't name
  `127.0.0.1`/`localhost` is rejected with `403`, which defeats DNS
  rebinding (a public hostname resolved to `127.0.0.1` after a browser's
  initial same-origin check passes); an `Origin` header on a genuine
  cross-origin request is rejected the same way.
- **Sends a strict `Content-Security-Policy`** on every response
  (`default-src 'self'`, no external or inline script/style sources, no
  framing) plus `X-Content-Type-Options: nosniff` and
  `X-Frame-Options: DENY`.
- **Serves two things only**: read-only graph queries at `POST /rpc`
  (translated to/from the existing MCP tool handlers in `src/mcp.rs` --
  same read-only guarantees as `mcp-server`) and static UI asset files
  from a configured local directory. It writes nothing.
- **Every request is bounded to a fixed time budget** (30s); a handler
  that exceeds it is cancelled and the client gets `504` rather than a
  hung connection.
- Runs until `Ctrl-C`, exactly like `watch` without `--once`. It is not a
  background/system service and does not persist across invocations.

## Tests Stay Offline and Deterministic

`just test` / `just check-all` (`cargo test --all-targets --all-features`,
without `--ignored`) never makes a real network call:

- Model-selection tests exercise `MockModel` directly, or point the
  `OpenAiModel`/`DeepInfraModel` HTTP adapters at a `TcpListener` bound to
  `127.0.0.1:0` (an ephemeral local port), never a real endpoint
  (`src/generation/openai.rs`, `src/generation/deepinfra.rs`).
- `SemanticSearch`'s default embedding provider (`MockEmbeddingProvider`) is
  a deterministic feature-hashing function, not a live embeddings API call.
- The only two `#[ignore]`-gated tests in the repository are
  `regression_scan` (scans real repositories that happen to exist on the
  machine running it -- filesystem only, no network) and the golden-snapshot
  regeneration test (deterministic `MockModel` output, gated so a normal
  test run never silently rewrites the committed snapshots). Neither runs
  under `just test`/`just check-all`, and neither touches the network.

This is a structural guarantee, not a policy: nothing in the default test
path constructs a real model client with a real API key, because
`select_model()` (`src/commands.rs`) only returns one when the corresponding
environment variable is set, and no test sets it.
