import type { PositionedNode } from './types'

/**
 * Picks the nodes worth labelling, by priority and then by elbow room.
 *
 * Priority alone stacked labels: the highest-degree nodes congregate in the
 * densest region, so the live polyglot render measured 83% of node labels
 * materially overlapping (LIT-24.48 AC8 geometry oracle). With the near
 * top-down camera, world XZ distance is proportional to screen distance, so a
 * greedy minimum-separation pass over the laid-out positions keeps a lower
 * priority label only when it has room. The selected node is always kept.
 */
export function chooseImportantNodes(
  nodes: PositionedNode[],
  selectedId: string | null,
  entryPointIds: Set<string>,
  clusterMemberIds?: Set<string>,
  positions?: Map<string, [number, number, number]>,
): PositionedNode[] {
  const limit = clusterMemberIds ? 28 : 12
  const ranked = [...nodes]
    .filter((node) => !clusterMemberIds || clusterMemberIds.has(node.id))
    .sort((a, b) => nodePriority(b, selectedId, entryPointIds) - nodePriority(a, selectedId, entryPointIds) || a.id.localeCompare(b.id))
  if (!positions) return ranked.filter((node, index) => index < limit || node.id === selectedId)

  // Separation scales with the layout's own extent, so overview and zoomed
  // cluster scopes get the same visual density without knowing the camera.
  const placed = ranked.filter((node) => positions.has(node.id))
  const minSeparation = spanOf(placed, positions) * (clusterMemberIds ? 0.05 : 0.11)
  const kept: PositionedNode[] = []
  const keptPositions: Array<[number, number, number]> = []
  for (const node of ranked) {
    if (kept.length >= limit && node.id !== selectedId) break
    const position = positions.get(node.id)
    if (!position) continue
    const crowded = keptPositions.some((other) => distance(position, other) < minSeparation)
    if (crowded && node.id !== selectedId) continue
    kept.push(node)
    keptPositions.push(position)
  }
  return kept
}

function spanOf(nodes: PositionedNode[], positions: Map<string, [number, number, number]>): number {
  let minX = Infinity
  let maxX = -Infinity
  let minZ = Infinity
  let maxZ = -Infinity
  for (const node of nodes) {
    const position = positions.get(node.id)
    if (!position) continue
    minX = Math.min(minX, position[0])
    maxX = Math.max(maxX, position[0])
    minZ = Math.min(minZ, position[2])
    maxZ = Math.max(maxZ, position[2])
  }
  if (minX > maxX) return 0
  return Math.hypot(maxX - minX, maxZ - minZ)
}

function distance(a: [number, number, number], b: [number, number, number]): number {
  return Math.hypot(a[0] - b[0], a[2] - b[2])
}

function nodePriority(node: PositionedNode, selectedId: string | null, entryPointIds: Set<string>): number {
  let score = node.in_degree + node.out_degree
  if (node.id === selectedId) score += 10_000
  if (entryPointIds.has(node.id)) score += 2_000
  if (['Command', 'Container', 'Module'].includes(node.label)) score += 600
  if (node.label === 'Artifact') score += 120
  if (node.label === 'Unresolved') score -= 200
  return score
}
