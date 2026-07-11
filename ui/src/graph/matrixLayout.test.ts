import { describe, expect, it } from 'vitest'
import { computeMatrixPositions, SPACING } from './matrixLayout'
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

describe('computeMatrixPositions', () => {
  it('returns an empty map for an empty input without throwing', () => {
    const positions = computeMatrixPositions([])
    expect(positions.size).toBe(0)
  })

  it('is deterministic across repeated calls with the same input', () => {
    const nodes = [
      node({ id: 'symbol:c', label: 'Symbol' }),
      node({ id: 'artifact:b', label: 'Artifact' }),
      node({ id: 'module:a', label: 'Module' }),
    ]

    const first = computeMatrixPositions(nodes)
    const second = computeMatrixPositions([...nodes])

    for (const n of nodes) {
      expect(second.get(n.id)).toEqual(first.get(n.id))
    }
  })

  it('places every node flat on the XZ plane', () => {
    const nodes = [
      node({ id: 'a', label: 'Artifact' }),
      node({ id: 'b', label: 'Symbol' }),
      node({ id: 'c', label: 'Module' }),
    ]
    const positions = computeMatrixPositions(nodes)
    for (const [, [, y]] of positions) {
      expect(y).toBe(0)
    }
  })

  it('groups nodes with the same label into adjacent grid cells', () => {
    // Two "Symbol" nodes sort before the lone "Zebra" node (alphabetically
    // earlier) and land in consecutive grid indices, so they differ by
    // exactly one column step (or wrap to the next row) rather than being
    // scattered.
    const nodes = [
      node({ id: 'symbol:2', label: 'Symbol' }),
      node({ id: 'zebra:1', label: 'Zebra' }),
      node({ id: 'symbol:1', label: 'Symbol' }),
    ]
    const positions = computeMatrixPositions(nodes)

    const columns = Math.ceil(Math.sqrt(nodes.length))
    const rows = Math.ceil(nodes.length / columns)
    const halfWidth = ((columns - 1) * SPACING) / 2
    const halfDepth = ((rows - 1) * SPACING) / 2

    // symbol:1 sorts before symbol:2 (id tiebreak) and both sort before
    // zebra:1 (label order), so they occupy consecutive grid indices (0, 1).
    const indexOf = ([x, , z]: [number, number, number]): number => {
      const col = Math.round((x + halfWidth) / SPACING)
      const row = Math.round((z + halfDepth) / SPACING)
      return row * columns + col
    }
    expect(indexOf(positions.get('symbol:1')!)).toBe(0)
    expect(indexOf(positions.get('symbol:2')!)).toBe(1)
    expect(indexOf(positions.get('zebra:1')!)).toBe(2)
  })

  it('sorts by label first, then by id, for a stable grid order', () => {
    const nodes = [
      node({ id: 'z-artifact', label: 'Artifact' }),
      node({ id: 'a-symbol', label: 'Symbol' }),
      node({ id: 'a-artifact', label: 'Artifact' }),
    ]
    const positions = computeMatrixPositions(nodes)

    // Expected sort order: a-artifact, z-artifact (label 'Artifact' tie
    // broken by id), then a-symbol ('Symbol' sorts after 'Artifact').
    const columns = Math.ceil(Math.sqrt(nodes.length))
    const halfWidth = ((columns - 1) * SPACING) / 2
    const halfDepth = ((Math.ceil(nodes.length / columns) - 1) * SPACING) / 2

    expect(positions.get('a-artifact')).toEqual([-halfWidth, 0, -halfDepth])
    expect(positions.get('z-artifact')).toEqual([SPACING - halfWidth, 0, -halfDepth])
  })

  it('centers the grid roughly around the origin', () => {
    const nodes = Array.from({ length: 9 }, (_, i) => node({ id: `n${i}`, label: 'Artifact' }))
    const positions = computeMatrixPositions(nodes)

    let sumX = 0
    let sumZ = 0
    for (const [x, , z] of positions.values()) {
      sumX += x
      sumZ += z
    }
    const meanX = sumX / positions.size
    const meanZ = sumZ / positions.size

    expect(meanX).toBeCloseTo(0)
    expect(meanZ).toBeCloseTo(0)
  })

  it('uses a roughly-square grid whose column count is ceil(sqrt(n))', () => {
    const nodes = Array.from({ length: 10 }, (_, i) => node({ id: `n${i}`, label: 'Artifact' }))
    const positions = computeMatrixPositions(nodes)

    const xs = [...positions.values()].map(([x]) => x)
    const uniqueXs = new Set(xs.map((x) => x.toFixed(6)))
    // ceil(sqrt(10)) = 4 columns.
    expect(uniqueXs.size).toBe(4)
  })
})
