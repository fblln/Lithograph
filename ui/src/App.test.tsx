import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import type { LayoutResult, PositionedNode } from './graph/types'
import type { NodeDetail } from './api/nodeDetail'

// GraphScene renders into a WebGL canvas via @react-three/fiber, which
// jsdom cannot provide -- stub it so this test exercises App's real data
// flow (fetch -> state -> panels) without needing a real GPU context. The
// stub exposes a plain button that calls `onSelect` with the layout's
// first node, standing in for a real click on a rendered 3D node.
vi.mock('./graph/GraphScene', () => ({
  GraphScene: ({
    layout,
    onSelect,
    onSelectCluster,
    onEnterCluster,
  }: {
    layout: LayoutResult
    onSelect: (node: PositionedNode) => void
    onSelectCluster?: (cluster: { id: string; members: string[]; totalMembers: number; fallbackKey: string; center: [number, number, number]; radius: number }) => void
    onEnterCluster?: (cluster: { id: string; members: string[]; totalMembers: number; fallbackKey: string; center: [number, number, number]; radius: number }) => void
  }) => (
    <div data-testid="graph-scene">
      {layout.nodes.length} nodes
      <button type="button" onClick={() => onSelect(layout.nodes[0])}>
        select-first-node
      </button>
      <button type="button" onClick={() => onSelectCluster?.({ id: 'visual:path:repository-root', members: layout.nodes.map((node) => node.id), totalMembers: layout.nodes.length, fallbackKey: 'path:repository-root', center: [0, 0, 0], radius: 1 })}>select-root-cluster</button>
      <button type="button" onClick={() => onEnterCluster?.({ id: 'visual:path:repository-root', members: layout.nodes.map((node) => node.id), totalMembers: layout.nodes.length, fallbackKey: 'path:repository-root', center: [0, 0, 0], radius: 1 })}>enter-root-cluster</button>
    </div>
  ),
}))

const FIXTURE_LAYOUT: LayoutResult = {
  graph_snapshot_id: 'blake3:test',
  algorithm_version: 1,
  center_node: null,
  nodes: [
    {
      id: 'artifact:a.rs',
      label: 'Artifact',
      name: 'a.rs',
      file_path: 'a.rs',
      in_degree: 0,
      out_degree: 1,
      x: 0,
      y: 0,
      hop: 0,
    },
    {
      id: 'symbol:a.rs#f',
      label: 'Symbol',
      name: 'f',
      file_path: 'a.rs',
      in_degree: 1,
      out_degree: 0,
      x: 10,
      y: 10,
      hop: 1,
    },
  ],
  edges: [{ source: 'artifact:a.rs', target: 'symbol:a.rs#f', kind: 'Contains' }],
  budget: {
    node_budget: 150,
    edge_budget: 400,
    nodes_available: 2,
    edges_available: 1,
    nodes_returned: 2,
    edges_returned: 1,
    nodes_truncated: false,
    edges_truncated: false,
  },
}

const FIXTURE_DETAIL: NodeDetail = {
  id: 'artifact:a.rs', label: 'Artifact', name: 'a.rs',
  evidence: [{ path: 'a.rs', start_line: 1, end_line: 2 }],
  source: { status: 'available', text: '    1 | fn main() {}', message: null },
  definitions: [],
  references: [{ id: 'r1', direction: 'outbound', kind: 'Contains', counterpart: { id: 'symbol:a.rs#f', label: 'Symbol', name: 'f' }, evidence: [], resolver_strategy: 'syntax-extraction', confidence: 'High' }],
  related_docs: [],
  tags: [{ id: 'tag:artifact', namespace: 'kind', value: 'artifact', source: 'Parser', confidence: 'High', evidence: [], inherited_from: null }],
}

function mockRpcResponse(result: LayoutResult, detail: NodeDetail = FIXTURE_DETAIL) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
      const tool = JSON.parse(init?.body as string).params.name
      const payload = tool === 'get_node_detail' ? detail : tool === 'resolve_tag_expression' ? ['artifact:a.rs'] : result
      return Promise.resolve({
      ok: true,
      status: 200,
      json: () =>
        Promise.resolve({
          jsonrpc: '2.0',
          id: 1,
          result: { content: [{ type: 'text', text: JSON.stringify(payload) }] },
        }),
      })
    }),
  )
}

describe('App', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
    window.history.replaceState(null, '', '/')
  })

  it('restores focus, filters, budget, selection, and view mode from the URL', async () => {
    window.history.replaceState(null, '', '/?center=artifact%3Aa.rs&view=matrix&maxNodes=25&labels=Artifact&selected=symbol%3Aa.rs%23f&tags=kind%3Aartifact')
    mockRpcResponse(FIXTURE_LAYOUT)
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    const layoutCall = vi.mocked(fetch).mock.calls.find(([, init]) => JSON.parse((init as RequestInit).body as string).params.name === 'get_graph_layout')
    if (!layoutCall) throw new Error('expected an initial graph-layout request')
    const body = JSON.parse((layoutCall[1] as RequestInit).body as string)
    expect(body.params.arguments).toMatchObject({ center_node: 'artifact:a.rs', node_labels: ['Artifact'], max_nodes: 25 })
    await waitFor(() => expect(screen.getByText('symbol:a.rs#f')).toBeInTheDocument())
    expect(screen.getByRole('button', { name: 'Matrix' })).toHaveAttribute('data-active', 'true')
    expect(window.location.search).toContain('tags=kind%3Aartifact')
  })

  it('loads the overview layout and renders the fetched node count', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    expect(screen.getByTestId('graph-scene')).toHaveTextContent('2 nodes')
    expect(screen.getByText('How this application is organized')).toBeInTheDocument()
    expect(screen.getByText('Major areas')).toBeInTheDocument()
    expect(screen.getByText('bounded graph slice')).toBeInTheDocument()
  })

  it('scopes from the architecture overview and returns with Back', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)
    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())

    vi.mocked(fetch).mockClear()
    mockRpcResponse(FIXTURE_LAYOUT)
    await user.click(screen.getByRole('button', { name: 'Open Repository root area' }))
    await waitFor(() => expect(fetch).toHaveBeenCalled())
    const scopedCall = vi.mocked(fetch).mock.calls.find(([, init]) => JSON.parse((init as RequestInit).body as string).params.name === 'get_graph_layout')
    if (!scopedCall) throw new Error('expected a scoped graph-layout request')
    expect(JSON.parse((scopedCall[1] as RequestInit).body as string).params.arguments.node_ids).toEqual(['artifact:a.rs', 'symbol:a.rs#f'])
    expect(screen.getByRole('button', { name: 'Back to previous context' })).toBeInTheDocument()

    vi.mocked(fetch).mockClear()
    mockRpcResponse(FIXTURE_LAYOUT)
    await user.click(screen.getByRole('button', { name: 'Back to previous context' }))
    await waitFor(() => expect(fetch).toHaveBeenCalled())
    const overviewCall = vi.mocked(fetch).mock.calls.find(([, init]) => JSON.parse((init as RequestInit).body as string).params.name === 'get_graph_layout')
    if (!overviewCall) throw new Error('expected an overview graph-layout request')
    expect(JSON.parse((overviewCall[1] as RequestInit).body as string).params.arguments.node_ids).toEqual([])
  })

  it('selects a cluster without entering, then drills down and restores with Escape', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)
    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())

    await user.click(screen.getByText('select-root-cluster'))
    expect(screen.getByLabelText('Selected cluster relationships')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'Back to previous context' })).not.toBeInTheDocument()

    vi.mocked(fetch).mockClear()
    mockRpcResponse(FIXTURE_LAYOUT)
    await user.click(screen.getByText('enter-root-cluster'))
    await waitFor(() => expect(fetch).toHaveBeenCalled())
    expect(screen.getByRole('button', { name: 'Back to previous context' })).toBeInTheDocument()
    expect(window.location.search).toContain('maxNodes=150')
    await user.click(screen.getByRole('button', { name: 'Tensions' }))
    expect(screen.getByRole('button', { name: 'Tensions' })).toHaveAttribute('data-active', 'true')
    await user.keyboard('{Escape}')
    await waitFor(() => expect(screen.queryByRole('button', { name: 'Back to previous context' })).not.toBeInTheDocument())
    expect(screen.getByRole('button', { name: 'Architecture' })).toHaveAttribute('data-active', 'true')
    expect(window.location.search).not.toContain('maxNodes=')
  })

  it('shows the RPC error message when the graph API call fails', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        json: () =>
          Promise.resolve({
            jsonrpc: '2.0',
            id: 1,
            error: { code: -32000, message: 'no graph found; run init first' },
          }),
      }),
    )
    render(<App />)

    await waitFor(() => expect(screen.getByText('Error')).toBeInTheDocument())
    expect(screen.getByText('no graph found; run init first')).toBeInTheDocument()
    // The error banner deliberately sits below the view control, so an API
    // failure cannot cover the only control that lets a user change layout.
    expect(screen.getByTestId('graph-error')).toHaveClass('top-14')
    expect(screen.getByTestId('view-mode-control')).toHaveClass('top-3')
  })

  it('re-fetches with an updated node_labels filter when a legend entry is toggled', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    vi.mocked(fetch).mockClear()
    mockRpcResponse(FIXTURE_LAYOUT)

    await user.click(screen.getByRole('button', { name: 'Filters' }))
    await user.click(screen.getByText('Symbol'))

    await waitFor(() => expect(fetch).toHaveBeenCalled())
    const [, init] = vi.mocked(fetch).mock.calls[0]
    const body = JSON.parse((init as RequestInit).body as string)
    expect(body.params.arguments.node_labels).toEqual(['Symbol'])
    expect(window.location.search).toContain('labels=Symbol')
  })

  it('selecting a node shows its details in the detail panel', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    await user.click(screen.getByText('select-first-node'))

    // "a.rs" appears twice in the detail panel (node name and file path) --
    // the node id is the unambiguous check that the right node was selected.
    expect(screen.getAllByText('a.rs').length).toBeGreaterThan(0)
    expect(screen.getByText('artifact:a.rs')).toBeInTheDocument()
    expect(window.location.search).toContain('selected=artifact%3Aa.rs')
    await waitFor(() => expect(screen.getByText('Source excerpt')).toBeInTheDocument())
    expect(screen.getByText(/syntax-extraction.*High/)).toBeInTheDocument()
  })

  it('restores a saved investigation selection when its node is still in the layout', async () => {
    const stored = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
    })
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    await user.click(screen.getByText('select-first-node'))
    await user.click(screen.getByRole('button', { name: 'Saved' }))
    await user.click(screen.getByRole('button', { name: 'Save investigation' }))
    await user.click(screen.getByText('Clear'))
    expect(screen.queryByText('Click a node in the graph to inspect it.')).not.toBeInTheDocument()

    await user.click(screen.getByRole('button', { name: 'Investigation' }))
    expect(screen.getByText('artifact:a.rs')).toBeInTheDocument()
  })

  it('carries a dashboard tension through evidence, tracing, and a saved investigation', async () => {
    const stored = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => stored.get(key) ?? null,
      setItem: (key: string, value: string) => stored.set(key, value),
    })
    vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
      const tool = JSON.parse(init?.body as string).params.name
      const payload = tool === 'get_repository_tensions'
        ? [{ id: 'tension-1', category: 'CouplingHotspot', severity: 'High', confidence: 'High', affected_nodes: ['artifact:a.rs'], metric_inputs: { degree: 9 }, evidence_references: ['a.rs:1'], follow_up_queries: ['MATCH (a)'], explanation: 'high coupling around a.rs' }]
        : tool === 'trace_path'
          ? { root: { id: 'artifact:a.rs', label: 'Artifact', name: 'a.rs', file_path: 'a.rs', in_degree: 0, out_degree: 1 }, visited: [{ node: { id: 'artifact:a.rs', label: 'Artifact', name: 'a.rs', file_path: 'a.rs', in_degree: 0, out_degree: 1 }, hop: 0 }, { node: { id: 'symbol:a.rs#f', label: 'Symbol', name: 'f', file_path: 'a.rs', in_degree: 1, out_degree: 0 }, hop: 1 }], relations: [{ source: 'artifact:a.rs', target: 'symbol:a.rs#f', kind: 'Contains' }] }
          : tool === 'get_node_detail' ? FIXTURE_DETAIL : FIXTURE_LAYOUT
      return Promise.resolve({ ok: true, status: 200, json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, result: { content: [{ type: 'text', text: JSON.stringify(payload) }] } }) })
    }))
    const download = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
    const user = userEvent.setup()
    render(<App />)

    await user.click(await screen.findByRole('button', { name: /Coupling Hotspot.*High/ }))
    expect(window.location.search).toContain('tension=tension-1')
    await user.click(await screen.findByRole('button', { name: 'artifact:a.rs' }))
    await waitFor(() => expect(screen.getByText('Source excerpt')).toBeInTheDocument())
    await user.click(screen.getByRole('button', { name: 'Trace dependency/call paths' }))
    await waitFor(() => expect(screen.getByText('Related trace')).toBeInTheDocument())
    await user.click(screen.getByRole('button', { name: 'Saved' }))
    await user.click(screen.getByRole('button', { name: 'Save investigation' }))
    expect([...stored.values()].join('')).toContain('"selectedTension":{"id":"tension-1"')
    await user.click(screen.getByRole('button', { name: 'Export Investigation as JSON' }))
    expect(download).toHaveBeenCalledOnce()
  })

  it('renders an unavailable-source explanation without losing node details', async () => {
    mockRpcResponse(FIXTURE_LAYOUT, {
      ...FIXTURE_DETAIL,
      source: { status: 'missing', text: null, message: 'The source file is no longer present in this checkout.' },
    })
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    await user.click(screen.getByText('select-first-node'))

    await waitFor(() => expect(screen.getByText('The source file is no longer present in this checkout.')).toBeInTheDocument())
    expect(screen.getByText('artifact:a.rs')).toBeInTheDocument()
  })

  it('focusing a selected node re-fetches with it as center_node and updates the URL', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    await user.click(screen.getByText('select-first-node'))

    vi.mocked(fetch).mockClear()
    mockRpcResponse(FIXTURE_LAYOUT)
    await user.click(screen.getByText('Focus here'))

    await waitFor(() => expect(fetch).toHaveBeenCalled())
    const layoutCall = vi.mocked(fetch).mock.calls.find(([, init]) => JSON.parse((init as RequestInit).body as string).params.name === 'get_graph_layout')
    if (!layoutCall) throw new Error('expected a focused graph-layout request')
    const body = JSON.parse((layoutCall[1] as RequestInit).body as string)
    expect(body.params.arguments.center_node).toBe('artifact:a.rs')
    expect(window.location.search).toContain('center=artifact')
    // Focusing clears the prior selection (a fresh view, not still
    // "inspecting" the node that was just made the new center).
    expect(screen.queryByText('Click a node in the graph to inspect it.')).not.toBeInTheDocument()
  })

  it('holds a large budget request behind the large-graph guard instead of firing it immediately', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    vi.mocked(fetch).mockClear()

    await user.click(screen.getByRole('button', { name: 'Filters' }))
    const budgetInput = screen.getByPlaceholderText('150 (default)')
    await user.type(budgetInput, '5000')
    await user.tab()

    expect(screen.getByText(/5000 nodes may render slowly/)).toBeInTheDocument()
    expect(fetch).not.toHaveBeenCalled()

    mockRpcResponse(FIXTURE_LAYOUT)
    await user.click(screen.getByText('Load anyway'))

    await waitFor(() => expect(fetch).toHaveBeenCalled())
    const [, init] = vi.mocked(fetch).mock.calls[0]
    const body = JSON.parse((init as RequestInit).body as string)
    expect(body.params.arguments.max_nodes).toBe(5000)
    expect(window.location.search).toContain('maxNodes=5000')
  })
})
