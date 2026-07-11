import type { PositionedNode } from './types'

/**
 * Maps the server's deterministic 2D concentric-ring layout
 * (src/graph/layout.rs::radial_positions -- x/y per hop ring) into a 3D
 * scene position: the ring itself stays on the XZ plane, and hop distance
 * from the focus node becomes height, so successive rings visibly recede
 * upward rather than sitting flat. Client-side layout math ends here --
 * everything about *where* a node sits is the server's decision (LIT-24.16);
 * this is presentation only.
 */
export const POSITION_SCALE = 0.05
export const HOP_HEIGHT = 0.6

export function nodeWorldPosition(node: PositionedNode): [number, number, number] {
  // `+ 0` normalizes `-0` (from `-0 * HOP_HEIGHT` at hop 0) to `0`, so the
  // focus node's height compares equal to plain `0` for callers/tests.
  return [node.x * POSITION_SCALE, -node.hop * HOP_HEIGHT + 0, node.y * POSITION_SCALE]
}

/**
 * Edge-count-aware opacity: fully visible below a small threshold, then
 * fades toward a visible floor as edge count grows, since edges (not
 * nodes) are what first turns a dense graph into an unreadable mass.
 */
export function edgeFadeOpacity(edgeCount: number): number {
  if (edgeCount <= 50) return 0.6
  return Math.max(0.08, 0.6 * Math.sqrt(50 / edgeCount))
}
