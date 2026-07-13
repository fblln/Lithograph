import { describe, expect, it } from 'vitest'
import { parseUrlState, serializeUrlState } from './urlState'

describe('parseUrlState', () => {
  it('defaults to radial view with no center node and no budget for an empty query', () => {
    expect(parseUrlState('')).toEqual({ centerNode: undefined, viewMode: 'radial', maxNodes: undefined })
  })

  it('reads center, view, and maxNodes params', () => {
    expect(parseUrlState('?center=artifact%3Areadme.md&view=matrix&maxNodes=300')).toEqual({
      centerNode: 'artifact:readme.md',
      viewMode: 'matrix',
      maxNodes: 300,
    })
  })

  it('falls back to radial for an unrecognized view param', () => {
    expect(parseUrlState('?view=bogus').viewMode).toBe('radial')
  })

  it('ignores a non-numeric or non-positive maxNodes rather than throwing', () => {
    expect(parseUrlState('?maxNodes=not-a-number').maxNodes).toBeUndefined()
    expect(parseUrlState('?maxNodes=-5').maxNodes).toBeUndefined()
    expect(parseUrlState('?maxNodes=0').maxNodes).toBeUndefined()
  })
})

describe('serializeUrlState', () => {
  it('produces an empty string for the all-defaults state', () => {
    expect(serializeUrlState({ centerNode: undefined, viewMode: 'radial', maxNodes: undefined })).toBe('')
  })

  it('only encodes params that differ from the default', () => {
    expect(serializeUrlState({ viewMode: 'matrix', maxNodes: undefined })).toBe('?view=matrix')
    expect(serializeUrlState({ viewMode: 'radial', maxNodes: 300 })).toBe('?maxNodes=300')
  })

  it('round-trips through parseUrlState', () => {
    const state = { centerNode: 'symbol:a.rs#f', viewMode: 'matrix' as const, maxNodes: 250 }
    expect(parseUrlState(serializeUrlState(state))).toEqual(state)
  })
})
