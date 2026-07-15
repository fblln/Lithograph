# Lithograph Explorer (frontend)

React 19 + TypeScript + Vite + react-three-fiber graph explorer UI, served
by Lithograph's embedded local server (`cargo run -- serve <repo>`, see
`../src/serve.rs`). See `../.backlog/decisions/decision-1 - *.md` for the
framework/build-tool decision record.

## Commands

```sh
npm install
npm run dev          # Vite dev server; proxies /rpc to http://127.0.0.1:4318
npm run build         # tsc -b && vite build -> dist/
npm run typecheck
npm run lint          # oxlint
npm run test          # vitest
npm run check-all     # typecheck + lint + test
npm run regression    # three-scale graph and canvas/rendering contracts
npm run visual-test   # projected readability metrics + browser artifacts
```

To run against a real repository during development, start the Rust
server first (`cargo run -- serve /path/to/repo --port 4317`), then
`npm run dev` in this directory -- the dev server's `/rpc` proxy (see
`vite.config.ts`) forwards graph API calls to it.

## CI regression validation

The browser-facing regression harness is deterministic: it uses small,
medium, and 1,000-node generated graph fixtures plus the captured polyglot
layout. It verifies budget preservation, deterministic server-layout inputs,
batched canvas rendering, filters, workbench and overlay interactions. Run it
with the ordinary UI gate:

```sh
npm run check-all
```

The Rust gate's analytics tests cover the exact-to-deterministic-sampled
betweenness policy and the MCP layout test covers server-side budgeted/cached
layouts. For a complete local validation run from the repository root:

```sh
just check-all
(cd ui && npm run check-all)
```

### Browser readability diagnostics

By default the visual test builds the UI and starts the polyglot fixture
server itself. Point it at an already-running realistic repository with
`LITHOGRAPH_VISUAL_BASE_URL`:

```sh
npx playwright install chromium # one-time browser installation
cd ui
npm run visual-test # self-contained polyglot check
(cd .. && cargo run --bin lithograph -- serve /path/to/repository --assets ui/dist --port 4317)
LITHOGRAPH_VISUAL_BASE_URL=http://127.0.0.1:4317 npm run visual-test
```

Adding `?visualDiagnostics=1` to a served explorer exposes the same geometry
probe to an interactive browser session. It reports clipped labels,
perspective-scale ratios, material node-label collisions, cluster spread, and
overlay occlusion. The Playwright test attaches that JSON and a screenshot for
both architecture and tension views, plus a trace and video on failure.
Geometry is the pass/fail oracle; screenshots remain review artifacts because
WebGL pixels and font rasterization vary across operating systems and GPUs.

To serve a production build end-to-end:

```sh
npm run build
cargo run -- serve /path/to/repo --assets "$(pwd)/dist"
```

## Saved investigations and export

The **Saved** sidebar tab stores versioned investigations in browser-local
storage. A saved view records its graph snapshot, focus, selection, filters,
metric overlay, query text/results, notes, the current focused subgraph, and
available health findings. The explorer also keeps the shareable portions of
the view (focus, filters, selection, mode, and budget) in the URL.

Use **Export JSON** on a saved investigation to download a portable report.
The format is a JSON object with `format` set to
`lithograph-investigation-report`, a top-level version, and the saved
investigation under `investigation`. Reports from a different graph snapshot
are explicitly marked as stale in the sidebar before restoration.

## Architecture

- **Data**: `src/api/rpc.ts` implements the JSON-RPC 2.0 `tools/call`
  envelope the server's `POST /rpc` expects; `src/api/graph.ts` wraps the
  `get_graph_layout` tool with a typed request/response
  (`src/graph/types.ts` mirrors `src/graph/layout.rs`'s serde output
  field-for-field).
- **Layout**: three view modes, switched via `ViewModeToggle`. The default
  "Cluster" mode (`src/graph/clusterLayout.ts`) treats analytical communities
  as primary force bodies, assigns every otherwise-unclustered node to a
  deterministic evidence-based fallback region, aggregates directed coupling,
  and runs fixed-iteration seeded force simulations globally and within each
  region. Stable ordering, deterministic hashes, rounded coordinates, and
  bounded local repulsion make unchanged graphs reproduce exact positions.
  "Radial"
  positions are computed server-side (LIT-24.16) as a deterministic
  concentric-ring layout by BFS hop from a focus node (or a degree-ranked
  pseudo-root in overview mode); `src/graph/positions.ts` maps that 2D
  (x, y, hop) into a 3D scene position. "Matrix" is a client-side
  deterministic grid layout (`src/graph/matrixLayout.ts`), sorted by node
  label then id, so same-kind nodes group into contiguous bands -- a
  genuinely different arrangement, not just a camera-angle change. In
  graph modes, a user can drag a node to a custom spot
  (`src/graph/useDragPositions.ts`); overrides persist per graph snapshot
  in `localStorage` and win over whichever base layout is active.
  `GraphScene.tsx` resolves the one `positions` map (base layout + drag
  overrides) that `NodeCloud`, `EdgeLines`, and `ClusterHulls` all read
  from, so nothing can disagree about where a node currently is.
- **Rendering**: `src/graph/NodeCloud.tsx` draws every node as one
  `InstancedMesh` (one draw call regardless of node count) and
  `src/graph/EdgeLines.tsx` draws relationship-kind-colored lines in one
  `LineSegments` batch plus directional arrowheads in one instanced mesh.
  Cluster overview links retain directed counts, dominant kinds, filters, and
  expandable underlying relationships. This technique -- bounded layout plus
  an explicit node/edge budget plus `InstancedMesh` -- is the one thing
  taken as a design reference from `codebase-memory-mcp/graph-ui`
  (MIT licensed, github.com/DeusData/codebase-memory-mcp): no files are
  copied from it, this is an original implementation (see decision-1,
  amended, and LIT-24.47's acceptance criteria).
  `src/graph/ClusterHulls.tsx` draws adaptive padded regions driven by the same
  membership as the force layout, including empty/singleton/pair/partial
  states, with accessible human-readable labels. `src/clusterIdentity.ts`
  derives offline names, responsibilities, counts, entry points, dependencies,
  boundary interpretations, and tension severity from existing architecture,
  layout, and tension evidence; raw cluster IDs remain technical details only.
- **Theme**: `src/index.css` defines the oklch dark-workspace palette
  lifted directly from `../lithograph-atlas-prototype/`'s inline styles
  (the design source of truth); `src/graph/palette.ts` duplicates the
  node-kind colors as plain hex because `THREE.Color` cannot parse
  `oklch()` -- keep the two in sync by hand if the palette changes.

## Scope (LIT-24.47)

Implemented: 3D graph rendering from real server data, cluster, radial, and matrix
layout view-switching, drag-persisted node positions, cluster convex-hull
rendering, node-kind legend with filtering, node selection/detail panel,
focus (re-center) navigation, budget/truncation reporting, strict-theme
re-skin.

Not yet implemented (left for a follow-up increment): a project switcher
(no server-side multi-project concept exists yet to back one) and
stats/control tabs beyond the budget numbers already in the sidebar.
