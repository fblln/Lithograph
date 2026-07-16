import { describe, expect, it } from 'vitest'
import { EDGE_RESOLUTION_STYLE, aggregateEdgeConfidence, edgeResolution, filterVisibleEdges } from './edgeProvenance'
import type { LayoutEdge } from './types'

const edges: LayoutEdge[] = [
  { id: 'proven', source: 'a', target: 'b', kind: 'Calls', resolution: 'HybridResolved', confidence: 'High', resolver_strategy: 'type-aware' },
  { id: 'syntax', source: 'b', target: 'c', kind: 'Calls', resolution: 'SyntaxOnly', confidence: 'High' },
  { source: 'c', target: 'd', kind: 'Imports' },
]

describe('edge provenance', () => {
  it('defaults legacy edges conservatively and distinguishes styles', () => {
    expect(edgeResolution(edges[2])).toBe('SyntaxOnly')
    expect(EDGE_RESOLUTION_STYLE.HybridResolved.opacity).toBeGreaterThan(EDGE_RESOLUTION_STYLE.SyntaxOnly.opacity)
    expect(EDGE_RESOLUTION_STYLE.SyntaxOnly.dashSize).toBeGreaterThan(0)
    expect(EDGE_RESOLUTION_STYLE.Fallback.opacity).toBeLessThan(EDGE_RESOLUTION_STYLE.SyntaxOnly.opacity)
    expect(aggregateEdgeConfidence([edges[2]])).toBe('Low')
    expect(aggregateEdgeConfidence([edges[0]])).toBe('High')
  })

  it('composes resolution visibility with relationship-kind filters', () => {
    expect(filterVisibleEdges(edges, new Set(), false).map((edge) => edge.id)).toEqual(['proven'])
    expect(filterVisibleEdges(edges, new Set(['Imports']), true)).toEqual([edges[2]])
  })
})
