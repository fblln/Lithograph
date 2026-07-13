import { describe, expect, it } from 'vitest'
import layoutFixture from '../testdata/polyglot-layout.json'
import architectureFixture from '../testdata/polyglot-architecture.json'
import type { LayoutResult, PositionedNode } from './types'
import type { ArchitectureCluster } from '../api/architecture'
import { computeMatrixPositions } from './matrixLayout'
import { nodeWorldPosition } from './positions'
import { convexHull2D } from './convexHull'
import { colorForLabel, FALLBACK_NODE_COLOR } from './palette'

/**
 * `src/testdata/polyglot-*.json` are real `get_graph_layout`/`get_architecture`
 * responses captured from a running `cargo run -- serve` instance against
 * `fixtures/polyglot` (the same repo `tests/golden/polyglot` is built
 * from) -- not hand-built toy data. LIT-24.47 AC1 asks for the added
 * layout modes to be tested at that scale; the smaller hand-built fixtures
 * in matrixLayout.test.ts/convexHull.test.ts/positions.test.ts already
 * cover exact behavior in isolation, so this file focuses on what only a
 * realistic graph can exercise: no node/edge count is small enough to
 * mask an off-by-one or a NaN, and the cluster data is real community
 * detection output, not a hand-picked convex shape.
 */
const layout = layoutFixture as LayoutResult
const clusters = (architectureFixture as { clusters: ArchitectureCluster[] }).clusters

describe('layout modes at tests/golden/polyglot scale', () => {
  it('loads a realistic fixture, not a toy graph', () => {
    expect(layout.nodes.length).toBeGreaterThan(100)
    expect(layout.edges.length).toBeGreaterThan(100)
    expect(clusters.length).toBeGreaterThan(0)
  })

  it('computeMatrixPositions (matrix view) places every node exactly once, deterministically, with finite coordinates', () => {
    const first = computeMatrixPositions(layout.nodes)
    const second = computeMatrixPositions(layout.nodes)
    expect(first.size).toBe(layout.nodes.length)

    for (const node of layout.nodes) {
      const position = first.get(node.id)
      expect(position).toBeDefined()
      expect(position).toEqual(second.get(node.id))
      const [x, y, z] = position as [number, number, number]
      expect(Number.isFinite(x)).toBe(true)
      expect(Number.isFinite(y)).toBe(true)
      expect(Number.isFinite(z)).toBe(true)
    }
  })

  it('nodeWorldPosition (radial view) is finite and deterministic for every node', () => {
    for (const node of layout.nodes) {
      const first = nodeWorldPosition(node)
      const second = nodeWorldPosition(node)
      expect(first).toEqual(second)
      expect(first.every(Number.isFinite)).toBe(true)
    }
  })

  it('every real cluster with 3+ resolvable members produces a valid convex hull', () => {
    const byId = new Map<string, PositionedNode>(layout.nodes.map((node) => [node.id, node]))
    let hullsComputed = 0

    for (const cluster of clusters) {
      const flatPoints: [number, number][] = cluster.members
        .map((memberId) => byId.get(memberId))
        .filter((node): node is PositionedNode => node !== undefined)
        .map(nodeWorldPosition)
        .map(([x, , z]) => [x, z])

      if (flatPoints.length < 3) continue

      const hull = convexHull2D(flatPoints)
      hullsComputed += 1

      expect(hull.length).toBeGreaterThanOrEqual(3)
      expect(hull.length).toBeLessThanOrEqual(flatPoints.length)
      for (const [hx, hy] of hull) {
        expect(flatPoints.some(([px, py]) => px === hx && py === hy)).toBe(true)
      }
    }

    // Guards against every cluster silently being skipped (e.g. a member-id
    // mismatch between the two fixture files), which would make the loop
    // above vacuously pass without testing anything.
    expect(hullsComputed).toBeGreaterThan(0)
  })

  it('every node label actually present in real data resolves to a themed color, not the fallback', () => {
    const labels = new Set(layout.nodes.map((node) => node.label))
    expect(labels.size).toBeGreaterThan(0)
    for (const label of labels) {
      expect(colorForLabel(label)).not.toBe(FALLBACK_NODE_COLOR)
    }
  })
})
