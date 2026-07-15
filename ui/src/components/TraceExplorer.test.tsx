import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { TraceExplorer } from './TraceExplorer'

const RESULT = { root: { id: 'symbol:a#f', label: 'Symbol', name: 'f', file_path: 'a.rs', in_degree: 1, out_degree: 2 }, visited: [{ node: { id: 'symbol:a#f', label: 'Symbol', name: 'f', file_path: 'a.rs', in_degree: 1, out_degree: 2 }, hop: 0 }, { node: { id: 'symbol:b#g', label: 'Symbol', name: 'g', file_path: 'b.rs', in_degree: 1, out_degree: 0 }, hop: 1 }], relations: [{ source: 'symbol:a#f', target: 'symbol:b#g', kind: 'Calls' }] }

function mockRpc(fail = false) {
  vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
    const tool = JSON.parse(init?.body as string).params.name
    const payload = tool === 'get_architecture' ? { clusters: [{ id: 'cluster-a', members: ['symbol:b#g'], top_nodes: [], packages: [], edge_types: [], cohesion: 1 }, { id: 'cluster-b', members: ['symbol:a#f'], top_nodes: [], packages: [], edge_types: [], cohesion: 1 }] } : RESULT
    const body = fail && tool !== 'get_architecture' ? { jsonrpc: '2.0', id: 1, error: { code: -32000, message: 'no graph node matched' } } : { jsonrpc: '2.0', id: 1, result: { content: [{ type: 'text', text: JSON.stringify(payload) }] } }
    return Promise.resolve({ ok: true, json: () => Promise.resolve(body) })
  }))
}

describe('TraceExplorer', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })
  it('renders path evidence and focuses affected nodes from trace and impact calls', async () => {
    mockRpc()
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<TraceExplorer onFocusNode={focus} />)
    await user.type(screen.getByLabelText('Trace node'), 'f')
    await user.click(screen.getByText('Trace path'))
    await waitFor(() => expect(screen.getByText(/1 affected nodes/)).toBeInTheDocument())
    expect(screen.getByText(/1 evidence relations/)).toBeInTheDocument()
    expect(screen.getByText(/risk: medium/)).toBeInTheDocument()
    expect(screen.getByText(/Affected clusters: cluster-a, cluster-b/)).toBeInTheDocument()
    await user.click(screen.getAllByText('Focus')[1])
    expect(focus).toHaveBeenCalledWith('symbol:b#g')
    await user.click(screen.getByText('Analyze impact'))
    await waitFor(() => expect(vi.mocked(fetch).mock.calls.some(([, init]) => String((init as RequestInit).body).includes('impact_analysis'))).toBe(true))
  })

  it('shows trace errors', async () => {
    mockRpc(true)
    const user = userEvent.setup()
    render(<TraceExplorer onFocusNode={vi.fn()} />)
    await user.click(screen.getByText('Trace path'))
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('no graph node matched'))
  })

  it('clips trace evidence to the current cluster scope', async () => {
    mockRpc()
    const user = userEvent.setup()
    render(<TraceExplorer onFocusNode={vi.fn()} scopeNodeIds={['symbol:b#g']} />)
    await user.click(screen.getByText('Trace path'))
    await waitFor(() => expect(screen.getByLabelText('Trace scope')).toHaveTextContent('1 nodes'))
    expect(screen.queryByText(/hop 0: f/)).not.toBeInTheDocument()
    expect(screen.getByText(/hop 1: g/)).toBeInTheDocument()
  })
})
