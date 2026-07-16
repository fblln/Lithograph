import { describe, expect, it } from 'vitest'
import type { LayoutResult } from './types'
import { deriveSliceFacetCounts } from './sliceFacets'

describe('deriveSliceFacetCounts', () => {
  it('counts the accepted bounded slice in stable label and kind order', () => {
    const layout = {
      nodes: [
        { id: 's1', label: 'Symbol' },
        { id: 'a1', label: 'Artifact' },
        { id: 's2', label: 'Symbol' },
      ],
      edges: [
        { source: 'a1', target: 's1', kind: 'Contains' },
        { source: 's1', target: 's2', kind: 'Calls' },
        { source: 's2', target: 's1', kind: 'Calls' },
      ],
    } as LayoutResult

    const counts = deriveSliceFacetCounts(layout)

    expect([...counts.nodeLabels]).toEqual([['Artifact', 1], ['Symbol', 2]])
    expect([...counts.edgeKinds]).toEqual([['Calls', 2], ['Contains', 1]])
  })
})
