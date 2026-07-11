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
```

To run against a real repository during development, start the Rust
server first (`cargo run -- serve /path/to/repo --port 4318`), then
`npm run dev` in this directory -- the dev server's `/rpc` proxy (see
`vite.config.ts`) forwards graph API calls to it.

To serve a production build end-to-end:

```sh
npm run build
cargo run -- serve /path/to/repo --assets "$(pwd)/dist"
```

## Architecture

- **Data**: `src/api/rpc.ts` implements the JSON-RPC 2.0 `tools/call`
  envelope the server's `POST /rpc` expects; `src/api/graph.ts` wraps the
  `get_graph_layout` tool with a typed request/response
  (`src/graph/types.ts` mirrors `src/graph/layout.rs`'s serde output
  field-for-field).
- **Layout**: two view modes, switched via `ViewModeToggle`. "Radial"
  positions are computed server-side (LIT-24.16) as a deterministic
  concentric-ring layout by BFS hop from a focus node (or a degree-ranked
  pseudo-root in overview mode); `src/graph/positions.ts` maps that 2D
  (x, y, hop) into a 3D scene position. "Matrix" is a client-side
  deterministic grid layout (`src/graph/matrixLayout.ts`), sorted by node
  label then id, so same-kind nodes group into contiguous bands -- a
  genuinely different arrangement, not just a camera-angle change. In
  either mode, a user can drag a node to a custom spot
  (`src/graph/useDragPositions.ts`); overrides persist per graph snapshot
  in `localStorage` and win over whichever base layout is active.
  `GraphScene.tsx` resolves the one `positions` map (base layout + drag
  overrides) that `NodeCloud`, `EdgeLines`, and `ClusterHulls` all read
  from, so nothing can disagree about where a node currently is.
- **Rendering**: `src/graph/NodeCloud.tsx` draws every node as one
  `InstancedMesh` (one draw call regardless of node count) and
  `src/graph/EdgeLines.tsx` draws every edge as one `LineSegments` with
  edge-count-aware fading. This technique -- server-computed layout plus
  an explicit node/edge budget plus `InstancedMesh` -- is the one thing
  taken as a design reference from `codebase-memory-mcp/graph-ui`
  (MIT licensed, github.com/DeusData/codebase-memory-mcp): no files are
  copied from it, this is an original implementation (see decision-1,
  amended, and LIT-24.47's acceptance criteria).
  `src/graph/ClusterHulls.tsx` fetches functional-community data from the
  existing `get_architecture` tool (`src/api/architecture.ts`, params
  `{ aspects: ["clusters"] }` -- no new Rust work needed) and draws a
  translucent convex-hull polygon (`src/graph/convexHull.ts`, Andrew's
  monotone chain) under each cluster's members.
- **Theme**: `src/index.css` defines the oklch dark-workspace palette
  lifted directly from `../lithograph-atlas-prototype/`'s inline styles
  (the design source of truth); `src/graph/palette.ts` duplicates the
  node-kind colors as plain hex because `THREE.Color` cannot parse
  `oklch()` -- keep the two in sync by hand if the palette changes.

## Scope (LIT-24.47)

Implemented: 3D graph rendering from real server data, radial and matrix
layout view-switching, drag-persisted node positions, cluster convex-hull
rendering, node-kind legend with filtering, node selection/detail panel,
focus (re-center) navigation, budget/truncation reporting, strict-theme
re-skin.

Not yet implemented (left for a follow-up increment): a project switcher
(no server-side multi-project concept exists yet to back one) and
stats/control tabs beyond the budget numbers already in the sidebar.
