import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { QueryWorkbench } from './QueryWorkbench'

function rpcMock(queryFailure = false) {
  vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
    const tool = JSON.parse(init?.body as string).params.name
    if (tool === 'query_graph' && queryFailure) return Promise.resolve({ ok: true, json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, error: { code: -32000, message: 'expected MATCH' } }) })
    const result = tool === 'get_graph_schema'
      ? { node_labels: [{ label: 'Artifact', count: 2 }], edge_types: [{ edge_type: 'Contains', count: 1 }], relationship_patterns: ['(Artifact)-[Contains]->(Symbol) [1x]'] }
      : [{ alias: 'a', id: 'artifact:a.rs', label: 'Artifact', name: 'a.rs', file_path: 'a.rs' }]
    return Promise.resolve({ ok: true, json: () => Promise.resolve({ jsonrpc: '2.0', id: 1, result: { content: [{ type: 'text', text: JSON.stringify(result) }] } }) })
  }))
}

describe('QueryWorkbench', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })
  it('runs, saves, and focuses a query result while exposing schema data', async () => {
    rpcMock()
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<QueryWorkbench onFocusNode={focus} />)
    await waitFor(() => expect(screen.getByText(/Node labels: Artifact/)).toBeInTheDocument())
    expect(screen.getByText('Properties: name, path')).toBeInTheDocument()
    expect(screen.getByText(/Communities: cluster overlays/)).toBeInTheDocument()
    await user.click(screen.getByText('Run query'))
    await waitFor(() => expect(screen.getByText('a.rs')).toBeInTheDocument())
    await user.click(screen.getByText('Save query'))
    expect(screen.getByText('Saved queries')).toBeInTheDocument()
    await user.click(screen.getByText('Focus'))
    expect(focus).toHaveBeenCalledWith('artifact:a.rs')
  })

  it('keeps the editor value and reports query errors', async () => {
    rpcMock(true)
    const user = userEvent.setup()
    render(<QueryWorkbench onFocusNode={vi.fn()} />)
    const editor = screen.getByLabelText('Graph query')
    await user.clear(editor)
    await user.type(editor, 'SELECT * FROM nodes')
    await user.click(screen.getByText('Run query'))
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('expected MATCH'))
    expect(editor).toHaveValue('SELECT * FROM nodes')
  })

  it('restores a saved query and its result rows', async () => {
    rpcMock()
    const { rerender } = render(<QueryWorkbench onFocusNode={vi.fn()} state={{ query: 'MATCH saved', rows: [{ alias: 'a', id: 'artifact:old.rs', label: 'Artifact', name: 'old.rs', file_path: 'old.rs' }] }} />)
    expect(screen.getByLabelText('Graph query')).toHaveValue('MATCH saved')
    expect(screen.getByText('old.rs')).toBeInTheDocument()

    rerender(<QueryWorkbench onFocusNode={vi.fn()} state={{ query: 'MATCH restored', rows: [{ alias: 'b', id: 'artifact:new.rs', label: 'Artifact', name: 'new.rs', file_path: 'new.rs' }] }} />)
    await waitFor(() => expect(screen.getByLabelText('Graph query')).toHaveValue('MATCH restored'))
    expect(screen.getByText('new.rs')).toBeInTheDocument()
  })

  it('clips restored query results to the current cluster scope', () => {
    rpcMock()
    render(<QueryWorkbench onFocusNode={vi.fn()} scopeNodeIds={['artifact:inside.rs']} state={{ query: 'MATCH scoped', rows: [{ alias: 'a', id: 'artifact:outside.rs', label: 'Artifact', name: 'outside.rs', file_path: 'outside.rs' }] }} />)
    expect(screen.getByLabelText('Query scope')).toHaveTextContent('1 cluster/tag nodes')
    expect(screen.getByRole('status')).toHaveTextContent('No query results are inside')
  })
})
