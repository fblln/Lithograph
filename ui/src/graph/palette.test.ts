import { describe, expect, it } from 'vitest'
import { colorForLabel, FALLBACK_NODE_COLOR, NODE_COLORS } from './palette'

describe('colorForLabel', () => {
  it('returns the themed color for every known GraphNode label', () => {
    for (const label of Object.keys(NODE_COLORS)) {
      expect(colorForLabel(label)).toBe(NODE_COLORS[label])
    }
  })

  it('falls back for an unrecognized label instead of throwing', () => {
    expect(colorForLabel('SomethingNew')).toBe(FALLBACK_NODE_COLOR)
  })
})
