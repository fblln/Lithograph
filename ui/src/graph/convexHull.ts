/**
 * 2D signed area of the parallelogram spanned by (a - o) and (b - o).
 * Positive when o -> a -> b turns counter-clockwise, negative when
 * clockwise, zero when collinear. This is the only primitive the
 * monotone-chain hull needs.
 */
function cross(o: [number, number], a: [number, number], b: [number, number]): number {
  return (a[0] - o[0]) * (b[1] - o[1]) - (a[1] - o[1]) * (b[0] - o[0])
}

/**
 * Andrew's monotone chain convex hull. Sorts points lexicographically
 * (x, then y), then sweeps left-to-right building a "lower" chain and
 * right-to-left building an "upper" chain, popping the last point of a
 * chain whenever it would make a non-left (clockwise or collinear) turn.
 * Concatenating the two chains (each with its last point dropped, since
 * it's the first point of the other chain) yields the hull in
 * counter-clockwise order.
 *
 * The `<= 0` pop condition (rather than `< 0`) is what makes collinear
 * points fall out of the hull automatically: a wholly collinear input
 * collapses to just its two extreme points, with no extra casing needed
 * here for that or for the 0/1/2-point inputs.
 */
export function convexHull2D(points: [number, number][]): [number, number][] {
  // Dedupe exact-coordinate duplicates so a repeated point can't get
  // stuck in a chain (it would compare non-strictly and never pop), and
  // sort into the order the sweep below assumes.
  const seen = new Map<string, [number, number]>()
  for (const p of points) {
    seen.set(`${p[0]},${p[1]}`, p)
  }
  const unique = Array.from(seen.values())
  unique.sort((a, b) => (a[0] === b[0] ? a[1] - b[1] : a[0] - b[0]))

  if (unique.length <= 1) return unique

  const lower: [number, number][] = []
  for (const p of unique) {
    while (lower.length >= 2 && cross(lower[lower.length - 2], lower[lower.length - 1], p) <= 0) {
      lower.pop()
    }
    lower.push(p)
  }

  const upper: [number, number][] = []
  for (let i = unique.length - 1; i >= 0; i--) {
    const p = unique[i]
    while (upper.length >= 2 && cross(upper[upper.length - 2], upper[upper.length - 1], p) <= 0) {
      upper.pop()
    }
    upper.push(p)
  }

  lower.pop()
  upper.pop()
  return lower.concat(upper)
}
