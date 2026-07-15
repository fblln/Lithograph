import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AnalyticsPanel } from './AnalyticsPanel'

describe('AnalyticsPanel', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })
  it('switches metric overlays and drills into health evidence without graph reload', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
      const tool = JSON.parse(init?.body as string).params.name
      const payload = tool === 'get_architecture'
        ? { clusters: [{ id: 'cluster-a', members: ['a'], top_nodes: [], packages: [], edge_types: [], cohesion: 1 }] }
        : { nodes: [{ id: 'a', fan_in: 1, fan_out: 2, page_rank: 0.1, betweenness: 2 }, { id: 'b', fan_in: 4, fan_out: 0, page_rank: 0.2, betweenness: 1 }], findings: [{ id: 'health:a', rule: 'GodClass', severity: 'High', affected_nodes: ['a'], evidence: ['degree=9'], investigation_query: 'MATCH (a) RETURN a' }] }
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, result: { content: [{ type: 'text', text: JSON.stringify(payload) }] } }) })
    }))
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<AnalyticsPanel onFocusNode={focus} />)
    await waitFor(() => expect(screen.getByText('a')).toBeInTheDocument())
    await user.selectOptions(screen.getByLabelText('Color and size by'), 'fan_in')
    expect(screen.getAllByRole('listitem')[0]).toHaveTextContent('b · 4.000')
    await user.click(screen.getByText('High: GodClass'))
    expect(screen.getByText('degree=9')).toBeInTheDocument()
    await user.click(screen.getByText('Focus finding'))
    expect(focus).toHaveBeenCalledWith('a')
    expect(fetch).toHaveBeenCalledTimes(2)
  })
})
