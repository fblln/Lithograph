import { convexHull2D } from './convexHull'

/**
 * Returns a deterministic padded region around a cluster's visible points.
 * Singletons and pairs receive compact non-degenerate regions; larger groups
 * retain their actual convex silhouette instead of being normalized to a
 * circle or regular island.
 */
export function adaptiveHull2D(points: [number, number][], padding = 0.28): [number, number][] {
  const hull = convexHull2D(points)
  if (hull.length === 0) return []
  if (hull.length === 1) return singletonRegion(hull[0], padding)
  if (hull.length === 2) return capsuleRegion(hull[0], hull[1], padding)
  const centroid = hull.reduce(([x, y], point) => [x + point[0], y + point[1]], [0, 0] as [number, number])
  centroid[0] /= hull.length
  centroid[1] /= hull.length
  return hull.map(([x, y]) => {
    const dx = x - centroid[0]
    const dy = y - centroid[1]
    const distance = Math.max(1e-9, Math.hypot(dx, dy))
    return [round(x + dx / distance * padding), round(y + dy / distance * padding)]
  })
}

function singletonRegion([x, y]: [number, number], padding: number): [number, number][] {
  const width = Math.max(0.36, padding * 1.45)
  const height = Math.max(0.28, padding)
  return [
    [x - width, y - height * 0.45],
    [x - width * 0.55, y - height],
    [x + width * 0.7, y - height * 0.82],
    [x + width, y + height * 0.15],
    [x + width * 0.4, y + height],
    [x - width * 0.75, y + height * 0.72],
  ].map(([px, py]) => [round(px), round(py)])
}

function capsuleRegion(a: [number, number], b: [number, number], padding: number): [number, number][] {
  const dx = b[0] - a[0]
  const dy = b[1] - a[1]
  const distance = Math.max(1e-9, Math.hypot(dx, dy))
  const nx = -dy / distance * padding
  const ny = dx / distance * padding
  const tx = dx / distance * padding
  const ty = dy / distance * padding
  return [
    [a[0] - tx + nx, a[1] - ty + ny],
    [b[0] + tx + nx, b[1] + ty + ny],
    [b[0] + tx - nx, b[1] + ty - ny],
    [a[0] - tx - nx, a[1] - ty - ny],
  ].map(([x, y]) => [round(x), round(y)])
}

function round(value: number): number {
  return Math.round(value * 1_000_000) / 1_000_000
}
