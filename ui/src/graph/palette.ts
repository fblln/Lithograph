/**
 * Node-kind color palette. Hex values are the exact CSS Color 4
 * OKLCH -> sRGB conversion of the `--node-*` custom properties defined in
 * `src/index.css` (which are themselves lifted from
 * lithograph-atlas-prototype/graph-data.js's `NODE_KINDS`). Duplicated here
 * as plain hex rather than read from CSS at runtime because three.js's
 * `Color.setStyle` does not parse `oklch()` -- browsers do, so the 2D UI
 * chrome (legend, chips) uses the CSS variables directly via `var(--node-*)`
 * and only the 3D scene needs this table. Keep the two in sync by hand if
 * the palette in index.css ever changes.
 */
export const NODE_COLORS: Record<string, string> = {
  Artifact: '#59aaf8',
  Symbol: '#ff9845',
  Module: '#ba93fb',
  Package: '#f8767a',
  Config: '#3cc998',
  Container: '#17d0d8',
  Command: '#e68cde',
  EnvVar: '#e3c23b',
  Documentation: '#b5c87d',
  Unresolved: '#6e7278',
}

export const FALLBACK_NODE_COLOR = '#8892a6'

export function colorForLabel(label: string): string {
  return NODE_COLORS[label] ?? FALLBACK_NODE_COLOR
}
