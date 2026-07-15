import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ClusterExplorer } from './ClusterExplorer'
import { deriveClusterInsights } from '../clusterInsights'
import type { LayoutResult } from '../graph/types'
import type { ArchitectureCluster } from '../api/architecture'

vi.mock('../api/tensions', () => ({ getRepositoryTensions: async () => [{ id: 't1', affected_nodes: ['b'], affected_edges: [], evidence_references: [], kind: 'CouplingHotspot', severity: 'High', title: 'Bridge risk', summary: '', why_it_matters: '', confidence: 'High', metrics: {}, suggested_query: '' }] }))

const layout: LayoutResult = {
  graph_snapshot_id: 'g1', algorithm_version: 1, center_node: null,
  nodes: [
    { id: 'a', label: 'Artifact', name: 'a', file_path: 'a.rs', in_degree: 0, out_degree: 1, x: 0, y: 0, hop: 0 },
    { id: 'b', label: 'Symbol', name: 'b', file_path: 'a.rs', in_degree: 1, out_degree: 1, x: 1, y: 0, hop: 0 },
    { id: 'c', label: 'Artifact', name: 'c', file_path: 'c.rs', in_degree: 1, out_degree: 0, x: 2, y: 0, hop: 0 },
  ],
  edges: [{ source: 'a', target: 'b', kind: 'Contains' }, { source: 'b', target: 'c', kind: 'Calls' }],
  budget: { node_budget: 3, edge_budget: 2, nodes_available: 5, edges_available: 2, nodes_returned: 3, edges_returned: 2, nodes_truncated: true, edges_truncated: false },
}
const clusters: ArchitectureCluster[] = [
  { id: 'cluster-a', members: ['a', 'b', 'hidden'], top_nodes: [{ name: 'b' }], packages: ['core'], edge_types: ['Contains', 'Calls'], cohesion: 0.75, incoming_pressure: 0, outgoing_pressure: 1 },
  { id: 'cluster-b', members: ['c'], top_nodes: [{ name: 'c' }], packages: ['web'], edge_types: ['Calls'], cohesion: 1, incoming_pressure: 1, outgoing_pressure: 0 },
]

describe('ClusterExplorer', () => {
  afterEach(cleanup)

  it('derives bridges, boundaries, conductance, and dominant kinds', () => {
    const [first] = deriveClusterInsights(layout, clusters, [])
    expect(first.bridgeNodes).toEqual(['b'])
    expect(first.boundaryEdges).toBe(1)
    expect(first.conductance).toBeCloseTo(1 / 3)
    expect(first.dominantKinds).toEqual(['kind:Artifact', 'kind:Symbol'])
  })

  it('expands, scopes, focuses bridges, pins, compares, and reports budgeted members', async () => {
    const user = userEvent.setup()
    const onScope = vi.fn()
    const onFocus = vi.fn()
    const onInterClusterOnly = vi.fn()
    render(<ClusterExplorer layout={layout} clusters={clusters} interClusterOnly={false} onScope={onScope} onInterClusterOnly={onInterClusterOnly} onFocus={onFocus} onRelatedEntity={() => {}} />)
    await user.click(screen.getByRole('button', { name: 'Expand B subsystem' }))
    expect(screen.getByRole('status')).toHaveTextContent('1 members are outside')
    await user.click(screen.getAllByRole('button', { name: 'b' })[0])
    expect(onFocus).toHaveBeenCalledWith('b')
    await user.click(screen.getByRole('button', { name: 'B subsystem' }))
    expect(onScope).toHaveBeenCalledWith(clusters[0])
    await user.click(screen.getByRole('button', { name: 'Pin B subsystem' }))
    expect(screen.getByRole('button', { name: 'Unpin B subsystem' })).toHaveAttribute('aria-pressed', 'true')
    await user.click(screen.getByRole('button', { name: 'Compare B subsystem' }))
    await user.click(screen.getByRole('button', { name: 'Compare C subsystem' }))
    expect(screen.getByLabelText('Cluster comparison')).toHaveTextContent('B subsystem vs C subsystem')
    await user.click(screen.getByRole('button', { name: 'Boundary edges' }))
    expect(onInterClusterOnly).toHaveBeenCalledWith(true)
  })
})
