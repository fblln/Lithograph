import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ClusterTensionDrilldown } from './ClusterTensionDrilldown'

describe('ClusterTensionDrilldown', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })
  it('summarizes mixed-severity cluster pressure and focuses an affected node', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 'critical', category: 'CouplingHotspot', severity: 'High', confidence: 'High', affected_nodes: ['symbol:a'], evidence_references: [], follow_up_queries: [], explanation: 'hot' }, { id: 'low', category: 'DeadCode', severity: 'Low', confidence: 'High', affected_nodes: ['symbol:b'], evidence_references: [], follow_up_queries: [], explanation: 'unused' }]) }] } }) }))
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<ClusterTensionDrilldown onFocus={focus} clusters={[{ id: 'cluster-a', members: ['symbol:a', 'symbol:b'], top_nodes: [{ name: 'a' }], packages: [], edge_types: ['Calls'], cohesion: 0.5, incoming_pressure: 1, outgoing_pressure: 2 }]} />)
    await waitFor(() => expect(screen.getByText(/2 signals · highest High/)).toBeInTheDocument())
    expect(screen.getByText(/Boundary pressure: in 1 \/ out 2/)).toBeInTheDocument()
    expect(screen.getByText(/Relationships: Calls/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Cluster A subsystem' }))
    expect(focus).toHaveBeenCalledWith('symbol:a')
  })

  it('renders nothing when tension data is empty', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: '[]' }] } }) }))
    const { container } = render(<ClusterTensionDrilldown onFocus={() => {}} clusters={[{ id: 'cluster-a', members: ['symbol:a'], top_nodes: [], packages: [], edge_types: [], cohesion: 0, incoming_pressure: 0, outgoing_pressure: 0 }]} />)
    await waitFor(() => expect(container).toBeEmptyDOMElement())
  })
})
