import type { PositionedNode } from './types'

/**
 * World-unit spacing between adjacent grid cells. Chosen to sit in the same
 * visual ballpark as the radial view's `POSITION_SCALE`/`HOP_HEIGHT`
 * constants (positions.ts) -- the server's raw x/y * POSITION_SCALE (0.05)
 * typically spans a few units per hop ring, and HOP_HEIGHT stacks rings
 * 0.6 apart, so 0.6 keeps matrix-mode node spacing comparable rather than
 * dwarfing or vanishing next to the radial view when a user switches modes.
 */
export const SPACING = 0.6

/**
 * Lays every node flat on the XZ plane (y=0) in a deterministic,
 * roughly-square grid, sorted by label then id. Sorting by label first
 * clusters same-kind nodes into contiguous grid bands, which is the
 * point of a "matrix" overview: see everything of one kind together,
 * as opposed to the radial view's grouping by hop-distance from a focus
 * node. This is pure presentation layout -- it does not touch the
 * server-computed x/y/hop fields on PositionedNode.
 */
export function computeMatrixPositions(nodes: PositionedNode[]): Map<string, [number, number, number]> {
  const positions = new Map<string, [number, number, number]>()
  if (nodes.length === 0) return positions

  const sorted = [...nodes].sort((a, b) => {
    const byLabel = a.label.localeCompare(b.label)
    if (byLabel !== 0) return byLabel
    return a.id.localeCompare(b.id)
  })

  const columns = Math.ceil(Math.sqrt(sorted.length))
  const rows = Math.ceil(sorted.length / columns)

  // Half-extent offsets center the grid on the origin instead of letting it
  // grow away from (0, 0, 0) into one quadrant, matching the radial view's
  // origin-centered layout.
  const halfWidth = ((columns - 1) * SPACING) / 2
  const halfDepth = ((rows - 1) * SPACING) / 2

  sorted.forEach((node, index) => {
    const column = index % columns
    const row = Math.floor(index / columns)
    positions.set(node.id, [column * SPACING - halfWidth, 0, row * SPACING - halfDepth])
  })

  return positions
}
