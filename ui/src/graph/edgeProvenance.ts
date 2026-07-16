import type { LayoutEdge } from './types'

export type EdgeResolution = 'HybridResolved' | 'SyntaxOnly' | 'Fallback'

export function edgeResolution(edge: LayoutEdge): EdgeResolution {
  return edge.resolution ?? 'SyntaxOnly'
}

export function isUnprovenEdge(edge: LayoutEdge): boolean {
  return edgeResolution(edge) !== 'HybridResolved'
}

export function filterVisibleEdges(edges: LayoutEdge[], edgeKinds: Set<string>, showUnproven: boolean): LayoutEdge[] {
  return edges.filter((edge) => (edgeKinds.size === 0 || edgeKinds.has(edge.kind)) && (showUnproven || !isUnprovenEdge(edge)))
}

export function aggregateEdgeConfidence(edges: LayoutEdge[]): 'Low' | 'High' {
  return edges.length > 0 && edges.every((edge) => edge.confidence === 'High') ? 'High' : 'Low'
}

export const EDGE_RESOLUTION_STYLE: Record<EdgeResolution, { opacity: number; dashSize: number; gapSize: number; label: string }> = {
  HybridResolved: { opacity: 0.9, dashSize: 0, gapSize: 0, label: 'Proven' },
  SyntaxOnly: { opacity: 0.58, dashSize: 0.16, gapSize: 0.08, label: 'Syntax only' },
  Fallback: { opacity: 0.34, dashSize: 0.07, gapSize: 0.11, label: 'Fallback' },
}
