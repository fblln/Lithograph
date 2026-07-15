import type { LayoutResult, PositionedNode } from '../graph/types'

/**
 * Generates repeatable graph slices for the UI regression suite. Keeping the
 * large case generated rather than checked in as a huge JSON blob makes its
 * scale explicit and lets CI exercise the same data shape without a network
 * request or machine-dependent timings.
 */
export function graphFixture(size: number): LayoutResult {
  const nodes: PositionedNode[] = Array.from({ length: size }, (_, index) => ({
    id: `artifact:fixture-${index}.rs`,
    label: index % 3 === 0 ? 'Artifact' : index % 3 === 1 ? 'Symbol' : 'Module',
    name: `fixture-${index}.rs`,
    file_path: `src/fixture-${index}.rs`,
    in_degree: index === 0 ? 0 : 1,
    out_degree: index === size - 1 ? 0 : 1,
    x: index % 32,
    y: Math.floor(index / 32),
    hop: index % 4,
  }))
  const edges = nodes.slice(1).map((node, index) => ({
    source: nodes[index].id,
    target: node.id,
    kind: 'Calls',
  }))

  return {
    graph_snapshot_id: `fixture:${size}`,
    algorithm_version: 1,
    center_node: null,
    nodes,
    edges,
    budget: {
      node_budget: size,
      edge_budget: Math.max(0, size - 1),
      nodes_available: size,
      edges_available: edges.length,
      nodes_returned: size,
      edges_returned: edges.length,
      nodes_truncated: false,
      edges_truncated: false,
    },
  }
}

export const smallGraphFixture = () => graphFixture(3)
export const mediumGraphFixture = () => graphFixture(156)
export const largeGraphFixture = () => graphFixture(1_000)
