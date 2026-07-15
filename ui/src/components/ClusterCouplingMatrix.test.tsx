import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import type { ArchitectureCluster } from '../api/architecture'
import { computeClusterCoupling } from '../clusterCoupling'
import { ClusterCouplingMatrix } from './ClusterCouplingMatrix'

const clusters: ArchitectureCluster[] = [
  { id: 'api', members: ['a', 'b'], top_nodes: [], packages: [], edge_types: [], cohesion: 0.5, incoming_pressure: 1, outgoing_pressure: 2 },
  { id: 'db', members: ['c'], top_nodes: [], packages: [], edge_types: [], cohesion: 1, incoming_pressure: 2, outgoing_pressure: 0 },
]
const edges = [{ source: 'a', target: 'b', kind: 'Calls' }, { source: 'a', target: 'c', kind: 'Calls' }, { source: 'b', target: 'c', kind: 'Calls' }]

describe('ClusterCouplingMatrix', () => {
  afterEach(cleanup)

  it('counts directed internal and boundary edges', () => {
    const cells = computeClusterCoupling(clusters, edges)
    expect(cells.find((cell) => cell.source.id === 'api' && cell.target.id === 'api')?.count).toBe(1)
    expect(cells.find((cell) => cell.source.id === 'api' && cell.target.id === 'db')?.count).toBe(2)
    expect(cells.find((cell) => cell.source.id === 'db' && cell.target.id === 'api')?.count).toBe(0)
  })

  it('opens the selected directed coupling cell', async () => {
    const onInspect = vi.fn()
    render(<ClusterCouplingMatrix clusters={clusters} edges={edges} onInspect={onInspect} />)
    await userEvent.click(screen.getByRole('button', { name: 'Api subsystem to Db subsystem: 2 relationships' }))
    expect(onInspect).toHaveBeenCalledWith(clusters[0], clusters[1])
  })
})
