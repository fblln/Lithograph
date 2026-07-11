import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import App from './App'
import type { LayoutResult, PositionedNode } from './graph/types'

// GraphScene renders into a WebGL canvas via @react-three/fiber, which
// jsdom cannot provide -- stub it so this test exercises App's real data
// flow (fetch -> state -> panels) without needing a real GPU context. The
// stub exposes a plain button that calls `onSelect` with the layout's
// first node, standing in for a real click on a rendered 3D node.
vi.mock('./graph/GraphScene', () => ({
  GraphScene: ({
    layout,
    onSelect,
  }: {
    layout: LayoutResult
    onSelect: (node: PositionedNode) => void
  }) => (
    <div data-testid="graph-scene">
      {layout.nodes.length} nodes
      <button type="button" onClick={() => onSelect(layout.nodes[0])}>
        select-first-node
      </button>
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

function mockRpcResponse(result: LayoutResult) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok: true,
      status: 200,
      json: () =>
        Promise.resolve({
          jsonrpc: '2.0',
          id: 1,
          result: { content: [{ type: 'text', text: JSON.stringify(result) }] },
        }),
    }),
  )
}

describe('App', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })

  it('loads the overview layout and renders the fetched node count', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    expect(screen.getByTestId('graph-scene')).toHaveTextContent('2 nodes')
    expect(screen.getByText('Artifact')).toBeInTheDocument()
    expect(screen.getByText('Symbol')).toBeInTheDocument()
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
  })

  it('re-fetches with an updated node_labels filter when a legend entry is toggled', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    vi.mocked(fetch).mockClear()
    mockRpcResponse(FIXTURE_LAYOUT)

    await user.click(screen.getByText('Symbol'))

    await waitFor(() => expect(fetch).toHaveBeenCalled())
    const [, init] = vi.mocked(fetch).mock.calls[0]
    const body = JSON.parse((init as RequestInit).body as string)
    expect(body.params.arguments.node_labels).toEqual(['Symbol'])
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
    const [, init] = vi.mocked(fetch).mock.calls[0]
    const body = JSON.parse((init as RequestInit).body as string)
    expect(body.params.arguments.center_node).toBe('artifact:a.rs')
    expect(window.location.search).toContain('center=artifact')
    // Focusing clears the prior selection (a fresh view, not still
    // "inspecting" the node that was just made the new center).
    expect(screen.getByText('Click a node in the graph to inspect it.')).toBeInTheDocument()
  })

  it('holds a large budget request behind the large-graph guard instead of firing it immediately', async () => {
    mockRpcResponse(FIXTURE_LAYOUT)
    const user = userEvent.setup()
    render(<App />)

    await waitFor(() => expect(screen.getByText('Ready')).toBeInTheDocument())
    vi.mocked(fetch).mockClear()

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
