import type { PositionedNode } from './types'

export function chooseImportantNodes(nodes: PositionedNode[], selectedId: string | null, entryPointIds: Set<string>, clusterMemberIds?: Set<string>): PositionedNode[] {
  const limit = clusterMemberIds ? 28 : 12
  return [...nodes]
    .filter((node) => !clusterMemberIds || clusterMemberIds.has(node.id))
    .sort((a, b) => nodePriority(b, selectedId, entryPointIds) - nodePriority(a, selectedId, entryPointIds) || a.id.localeCompare(b.id))
    .filter((node, index) => index < limit || node.id === selectedId)
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
