import { describe, expect, it } from 'vitest'
import { computeClusterLayout, computeClusterPositions } from './clusterLayout'
import type { LayoutEdge, PositionedNode } from './types'
import { largeGraphFixture } from '../testdata/graphFixtures'

const nodes: PositionedNode[] = [
  { id: 'a', label: 'Symbol', name: 'a', file_path: 'src/api/a.py', in_degree: 0, out_degree: 1, x: 0, y: 0, hop: 0 },
  { id: 'b', label: 'Symbol', name: 'b', file_path: 'src/api/b.py', in_degree: 1, out_degree: 1, x: 0, y: 0, hop: 0 },
  { id: 'c', label: 'Artifact', name: 'c', file_path: 'web/src/c.ts', in_degree: 1, out_degree: 0, x: 0, y: 0, hop: 0 },
  { id: 'd', label: 'Package', name: 'react', file_path: null, in_degree: 1, out_degree: 0, x: 0, y: 0, hop: 0 },
]

const edges: LayoutEdge[] = [
  { source: 'a', target: 'b', kind: 'Calls' },
  { source: 'b', target: 'c', kind: 'Imports' },
  { source: 'c', target: 'd', kind: 'DependsOnPackage' },
]

const clusters = [{
  id: 'core', members: ['a', 'b', 'missing'], top_nodes: [], packages: [], edge_types: [], cohesion: 1, incoming_pressure: 0, outgoing_pressure: 0,
}]

describe('computeClusterLayout', () => {
  it('places every visible node exactly once and preserves analytical clusters', () => {
    const result = computeClusterLayout(nodes, clusters, edges)

    expect([...result.positions.keys()].sort()).toEqual(['a', 'b', 'c', 'd'])
    expect(result.clusters.find((cluster) => cluster.id === 'core')).toMatchObject({
      members: ['a', 'b'],
      totalMembers: 3,
    })
    expect(result.clusters.some((cluster) => cluster.id === 'visual:path:web')).toBe(true)
    expect(result.clusters.some((cluster) => cluster.id === 'visual:dependencies')).toBe(true)
    for (const position of result.positions.values()) expect(position.every(Number.isFinite)).toBe(true)
  })

  it('is exactly deterministic independent of node, edge, and member order', () => {
    const first = computeClusterLayout(nodes, clusters, edges)
    const second = computeClusterLayout(
      [...nodes].reverse(),
      [{ ...clusters[0], members: [...clusters[0].members].reverse() }],
      [...edges].reverse(),
    )

    expect(first.positions).toEqual(second.positions)
    expect(first.clusters).toEqual(second.clusters)
    expect(first.links).toEqual(second.links)
  })

  it('uses directed weighted coupling to pull related clusters closer', () => {
    const separatedNodes = ['a', 'b', 'c'].map((id) => ({
      id, label: 'Artifact', name: id, file_path: `${id}/${id}.ts`, in_degree: 0, out_degree: 0, x: 0, y: 0, hop: 0,
    }))
    const separatedClusters = separatedNodes.map((node) => ({
      id: `cluster:${node.id}`, members: [node.id], top_nodes: [], packages: [], edge_types: [], cohesion: 1, incoming_pressure: 0, outgoing_pressure: 0,
    }))
    const coupled = computeClusterLayout(separatedNodes, separatedClusters, Array.from({ length: 8 }, () => ({ source: 'a', target: 'b', kind: 'Calls' })))
    const uncoupled = computeClusterLayout(separatedNodes, separatedClusters, [])
    const distance = (result: typeof coupled, left: string, right: string) => {
      const a = result.clusters.find((cluster) => cluster.id === left)!.center
      const b = result.clusters.find((cluster) => cluster.id === right)!.center
      return Math.hypot(a[0] - b[0], a[2] - b[2])
    }

    expect(distance(coupled, 'cluster:a', 'cluster:b')).toBeLessThan(distance(uncoupled, 'cluster:a', 'cluster:b'))
    expect(coupled.links[0]).toMatchObject({ source: 'cluster:a', target: 'cluster:b', count: 8, kinds: [{ kind: 'Calls', count: 8 }] })
  })

  it('retains the compatibility positions API', () => {
    expect(computeClusterPositions(nodes, clusters, edges)).toEqual(computeClusterLayout(nodes, clusters, edges).positions)
  })

  it('keeps a realistic 1,000-node bounded graph complete and stable', () => {
    const layout = largeGraphFixture()
    const realisticClusters = Array.from({ length: 12 }, (_, index) => ({
      id: `cluster:${index}`,
      members: layout.nodes.filter((_, nodeIndex) => nodeIndex % 12 === index).map((node) => node.id),
      top_nodes: [], packages: [], edge_types: ['References'], cohesion: 0.5, incoming_pressure: index, outgoing_pressure: 12 - index,
    }))
    const first = computeClusterLayout(layout.nodes, realisticClusters, layout.edges)
    const second = computeClusterLayout(layout.nodes, realisticClusters, layout.edges)

    expect(first.positions.size).toBe(1_000)
    expect(first.clusters).toHaveLength(12)
    expect(first.positions).toEqual(second.positions)
    expect(first.links).toEqual(second.links)
  })
})
