# Lithograph Atlas — graph exploration prototype

Interactive semantic-graph explorer for Lithograph, grounded in the
`fixtures/polyglot` fixture. Dark dev-tool aesthetic (Obsidian/Neo4j style).

## Files
- `Lithograph Atlas.dc.html` — the app (a Design Component; opens directly in a browser)
- `graph-data.js` — nodes, relations, node/relation kind metadata
- `graph-engine.js` — layout, force sim, blast-radius, hull/geometry, SVG draw helpers
- `atlas-data.js` — clusters, tensions, tags, docs sections, saved investigations
- `support.js` — DC runtime (required; do not edit)

## Running
Serve the folder over any static HTTP server (ES modules need http://, not file://):

    npx serve .
    # or
    python3 -m http.server

Then open `Lithograph Atlas.dc.html`.

## Features
- Cluster-first graph with convex-hull regions; drag a cluster's ∷ pill (or any
  node) to move the whole group — layout persists (localStorage), Reset clears it.
- Always-on cluster separation (no overlaps).
- Views: **Graph** (force clusters), **Radial** (all nodes on one circle grouped
  into cluster arcs), **Matrix** (cluster×cluster coupling grid, honors filters).
- Edge modes: node-to-node edges or aggregated cluster↔cluster bundles.
- Color overlays: Kind / Tensions / Centrality / Blast radius.
- Node sizing by importance; double-click a symbol to expand its members.
- Inspector: evidence, path, relations, tensions, blast radius (dependents ↔
  dependencies), related docs.
- Search, kind/relation/tag filters, graph budget, saved investigations.
- Docs workspace with graph↔doc bidirectional linking + MCP-agent draft flow.

Sample data only — swap `graph-data.js` / `atlas-data.js` for a real
`.lithograph` graph export to use with your own repository.
