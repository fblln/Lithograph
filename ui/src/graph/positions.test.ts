import { describe, expect, it } from 'vitest'
import { edgeFadeOpacity, HOP_HEIGHT, nodeWorldPosition, POSITION_SCALE } from './positions'
import type { PositionedNode } from './types'

function node(overrides: Partial<PositionedNode> = {}): PositionedNode {
  return {
    id: 'artifact:a.rs',
    label: 'Artifact',
    name: 'a.rs',
    file_path: 'a.rs',
    in_degree: 0,
    out_degree: 0,
    x: 0,
    y: 0,
    hop: 0,
    ...overrides,
  }
}

describe('nodeWorldPosition', () => {
  it('places the hop-0 focus node at the origin plane', () => {
    expect(nodeWorldPosition(node())).toEqual([0, 0, 0])
  })

  it('scales x/y and drops height by hop, so farther rings sit higher up', () => {
    const [x, y, z] = nodeWorldPosition(node({ x: 100, y: -40, hop: 2 }))
    expect(x).toBeCloseTo(100 * POSITION_SCALE)
    expect(z).toBeCloseTo(-40 * POSITION_SCALE)
    expect(y).toBeCloseTo(-2 * HOP_HEIGHT)
  })
})

describe('edgeFadeOpacity', () => {
  it('stays fully visible for small edge counts', () => {
    expect(edgeFadeOpacity(0)).toBe(0.6)
    expect(edgeFadeOpacity(50)).toBe(0.6)
  })

  it('fades monotonically as edge count grows past the threshold', () => {
    const at200 = edgeFadeOpacity(200)
    const at2000 = edgeFadeOpacity(2000)
    expect(at200).toBeLessThan(0.6)
    expect(at2000).toBeLessThan(at200)
  })

  it('never fades below a visible floor', () => {
    expect(edgeFadeOpacity(1_000_000)).toBeGreaterThanOrEqual(0.08)
  })
})
