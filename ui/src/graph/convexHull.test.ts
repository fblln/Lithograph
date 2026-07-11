import { describe, expect, it } from 'vitest'
import { convexHull2D } from './convexHull'

describe('convexHull2D', () => {
  it('returns empty for no points', () => {
    expect(convexHull2D([])).toEqual([])
  })

  it('returns the single point unchanged', () => {
    expect(convexHull2D([[1, 2]])).toEqual([[1, 2]])
  })

  it('returns both points for a degenerate two-point "hull"', () => {
    const result = convexHull2D([
      [3, 3],
      [0, 0],
    ])
    expect(result).toHaveLength(2)
    expect(result).toEqual(
      expect.arrayContaining([
        [0, 0],
        [3, 3],
      ]),
    )
  })

  it('excludes an interior point and returns exactly the square corners in CCW order', () => {
    const square: [number, number][] = [
      [0, 0],
      [4, 0],
      [4, 4],
      [0, 4],
    ]
    const withInterior: [number, number][] = [...square, [2, 2]]
    const result = convexHull2D(withInterior)

    expect(result).toEqual([
      [0, 0],
      [4, 0],
      [4, 4],
      [0, 4],
    ])
  })

  it('collapses collinear points to just the two extremes', () => {
    const result = convexHull2D([
      [0, 0],
      [1, 1],
      [2, 2],
      [3, 3],
    ])
    expect(result).toHaveLength(2)
    expect(result).toEqual(
      expect.arrayContaining([
        [0, 0],
        [3, 3],
      ]),
    )
  })

  it('collapses collinear points on a non-diagonal (axis-aligned) line too', () => {
    const result = convexHull2D([
      [0, 5],
      [2, 5],
      [4, 5],
      [1, 5],
    ])
    expect(result).toHaveLength(2)
    expect(result).toEqual(
      expect.arrayContaining([
        [0, 5],
        [4, 5],
      ]),
    )
  })

  it('deduplicates exactly-coincident points instead of producing a degenerate zero-length edge', () => {
    const result = convexHull2D([
      [0, 0],
      [0, 0],
      [4, 0],
      [4, 4],
    ])
    // Three distinct points -> a triangle, not four including a duplicate.
    expect(result).toHaveLength(3)
  })

  it('holds the general correctness property for a scattered point set: every hull point is a member of the input, hull count never exceeds input count, and every non-hull input point lies on the inward side of every hull edge', () => {
    const points: [number, number][] = [
      [5, 0],
      [10, 3],
      [12, 8],
      [8, 12],
      [3, 11],
      [0, 6],
      [2, 2],
      [6, 6],
      [7, 4],
      [4, 8],
      [9, 9],
      [1, 3],
      [11, 5],
      [6, 1],
      [3, 4],
    ]
    const hull = convexHull2D(points)

    expect(hull.length).toBeLessThanOrEqual(points.length)
    expect(hull.length).toBeGreaterThanOrEqual(3)

    const pointKeys = new Set(points.map((p) => `${p[0]},${p[1]}`))
    for (const p of hull) {
      expect(pointKeys.has(`${p[0]},${p[1]}`)).toBe(true)
    }

    // For a CCW hull, every other point must be on the left-of-or-on
    // each directed edge (cross product >= 0); a negative cross product
    // for any point would mean that point sits outside the hull, which
    // would mean the hull is wrong.
    function cross(o: [number, number], a: [number, number], b: [number, number]): number {
      return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
    }

    for (let i = 0; i < hull.length; i++) {
      const o = hull[i]
      const a = hull[(i + 1) % hull.length]
      for (const p of points) {
        expect(cross(o, a, p)).toBeGreaterThanOrEqual(-1e-9)
      }
    }
  })

  it('handles three non-collinear points as a triangle', () => {
    const result = convexHull2D([
      [0, 0],
      [4, 0],
      [2, 4],
    ])
    expect(result).toHaveLength(3)
    expect(result).toEqual(
      expect.arrayContaining([
        [0, 0],
        [4, 0],
        [2, 4],
      ]),
    )
  })
})
