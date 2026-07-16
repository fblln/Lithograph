import type { LayoutResult } from './types'

export interface SliceFacetCounts {
  nodeLabels: ReadonlyMap<string, number>
  edgeKinds: ReadonlyMap<string, number>
}

/** Counts facets from the accepted bounded layout, before client-only filters. */
export function deriveSliceFacetCounts(layout: LayoutResult): SliceFacetCounts {
  const nodeLabels = new Map<string, number>()
  const edgeKinds = new Map<string, number>()

  for (const node of layout.nodes) nodeLabels.set(node.label, (nodeLabels.get(node.label) ?? 0) + 1)
  for (const edge of layout.edges) edgeKinds.set(edge.kind, (edgeKinds.get(edge.kind) ?? 0) + 1)

  return {
    nodeLabels: sortedCounts(nodeLabels),
    edgeKinds: sortedCounts(edgeKinds),
  }
}

function sortedCounts(counts: Map<string, number>): ReadonlyMap<string, number> {
  return new Map([...counts].sort(([left], [right]) => left.localeCompare(right)))
}
