import { describe, expect, it } from 'vitest'
import { parseUrlState, serializeUrlState } from './urlState'

describe('parseUrlState', () => {
  it('defaults to clustered view with no center node and no budget for an empty query', () => {
    expect(parseUrlState('')).toEqual({ centerNode: undefined, viewMode: 'cluster', maxNodes: undefined })
  })

  it('reads center, view, budget, filters, and selection params', () => {
    expect(parseUrlState('?center=artifact%3Areadme.md&view=matrix&maxNodes=300&maxEdges=900&labels=Artifact,Symbol&selected=symbol%3Areadme.md%23title&tension=t1')).toEqual({
      centerNode: 'artifact:readme.md',
      viewMode: 'matrix',
      maxNodes: 300,
      maxEdges: 900,
      nodeLabels: ['Artifact', 'Symbol'],
      selectedNode: 'symbol:readme.md#title',
      tensionId: 't1',
    })
  })

  it('falls back to clustered view for an unrecognized view param', () => {
    expect(parseUrlState('?view=bogus').viewMode).toBe('cluster')
  })

  it('ignores a non-numeric or non-positive maxNodes rather than throwing', () => {
    expect(parseUrlState('?maxNodes=not-a-number').maxNodes).toBeUndefined()
    expect(parseUrlState('?maxNodes=-5').maxNodes).toBeUndefined()
    expect(parseUrlState('?maxNodes=0').maxNodes).toBeUndefined()
  })
})

describe('serializeUrlState', () => {
  it('produces an empty string for the all-defaults state', () => {
    expect(serializeUrlState({ centerNode: undefined, viewMode: 'cluster', maxNodes: undefined })).toBe('')
  })

  it('only encodes params that differ from the default', () => {
    expect(serializeUrlState({ viewMode: 'matrix', maxNodes: undefined })).toBe('?view=matrix')
    expect(serializeUrlState({ viewMode: 'cluster', maxNodes: 300 })).toBe('?maxNodes=300')
  })

  it('round-trips through parseUrlState', () => {
    const state = { centerNode: 'symbol:a.rs#f', viewMode: 'matrix' as const, maxNodes: 250, maxEdges: 800, nodeLabels: ['Artifact', 'Symbol'], selectedNode: 'symbol:a.rs#f', tensionId: 'cycle-1', tagExpression: 'kind:artifact,!role:test', workspaceMode: 'docs' as const, docSectionId: 'section:overview' }
    expect(parseUrlState(serializeUrlState(state))).toEqual(state)
  })
})
