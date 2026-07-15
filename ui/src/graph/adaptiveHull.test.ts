import { describe, expect, it } from 'vitest'
import { adaptiveHull2D } from './adaptiveHull'

describe('adaptiveHull2D', () => {
  it('creates understandable non-degenerate regions for empty, singleton, and pair states', () => {
    expect(adaptiveHull2D([])).toEqual([])
    expect(adaptiveHull2D([[2, 3]])).toHaveLength(6)
    expect(adaptiveHull2D([[0, 0], [3, 1]])).toHaveLength(4)
  })

  it('follows an organic member shape rather than forcing a circle', () => {
    const region = adaptiveHull2D([[0, 0], [5, 0], [1, 1], [0, 4]])
    const center = region.reduce(([x, y], point) => [x + point[0] / region.length, y + point[1] / region.length], [0, 0])
    const radii = region.map(([x, y]) => Math.hypot(x - center[0], y - center[1]))

    expect(Math.max(...radii) - Math.min(...radii)).toBeGreaterThan(1)
    expect(new Set(region.map(([x, y]) => `${x},${y}`)).size).toBe(region.length)
  })

  it('is deterministic regardless of input order', () => {
    const points: [number, number][] = [[0, 0], [3, 0], [4, 2], [1, 4], [2, 1]]
    expect(adaptiveHull2D(points)).toEqual(adaptiveHull2D([...points].reverse()))
  })
})
