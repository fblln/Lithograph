import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ExplorerSearch } from './ExplorerSearch'

describe('ExplorerSearch', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })

  it('searches typed graph results and focuses the selected node', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 'symbol:app#run', label: 'Symbol', name: 'run', file_path: 'app.py', in_degree: 1, out_degree: 2 }]) }] } }) }))
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<ExplorerSearch onFocus={focus} />)
    await user.type(screen.getByLabelText('Search graph'), 'run')
    await waitFor(() => expect(screen.getByRole('button', { name: /run.*symbol/i })).toBeInTheDocument())
    await user.click(screen.getByRole('button', { name: /run.*symbol/i }))
    expect(focus).toHaveBeenCalledWith('symbol:app#run')
    expect(screen.getByLabelText('Search graph')).toHaveValue('')
  })

  it('supports keyboard result navigation, selection, and dismissal', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 'symbol:first', label: 'Symbol', name: 'first', file_path: 'a.py', in_degree: 0, out_degree: 0 }, { id: 'symbol:second', label: 'Symbol', name: 'second', file_path: 'b.py', in_degree: 0, out_degree: 0 }]) }] } }) }))
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<ExplorerSearch onFocus={focus} />)
    const input = screen.getByRole('combobox', { name: 'Search graph' })
    await user.type(input, 'symbol')
    await waitFor(() => expect(screen.getByRole('listbox')).toBeInTheDocument())
    await user.keyboard('{ArrowDown}{Enter}')
    expect(focus).toHaveBeenCalledWith('symbol:second')

    await user.type(input, 'symbol')
    await waitFor(() => expect(screen.getByRole('listbox')).toBeInTheDocument())
    await user.keyboard('{Escape}')
    expect(screen.queryByRole('listbox')).not.toBeInTheDocument()
  })

  it('surfaces matching tension labels and opens their graph context', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
      const tool = JSON.parse(init?.body as string).params.name
      const payload = tool === 'get_repository_tensions'
        ? [{ id: 'tension-cycle', category: 'DependencyCycle', severity: 'High', confidence: 'High', affected_nodes: ['symbol:cycle'], metric_inputs: {}, evidence_references: ['cycle evidence'], follow_up_queries: [], explanation: 'circular dependency' }]
        : []
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify(payload) }] } }) })
    }))
    const focus = vi.fn()
    const selectTension = vi.fn()
    const user = userEvent.setup()
    render(<ExplorerSearch onFocus={focus} onSelectTension={selectTension} />)
    await user.type(screen.getByLabelText('Search graph'), 'circular')
    const result = await screen.findByRole('button', { name: /circular dependency.*Tension: DependencyCycle/ })
    await user.click(result)
    expect(selectTension).toHaveBeenCalledWith(expect.objectContaining({ id: 'tension-cycle' }))
    expect(focus).toHaveBeenCalledWith('symbol:cycle')
  })

  it('limits graph search results to the active tag scope', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
      const tool = JSON.parse(init?.body as string).params.name
      const payload = tool === 'search_graph' ? [
        { id: 'artifact:in', label: 'Artifact', name: 'inside', file_path: 'in.rs', in_degree: 0, out_degree: 0 },
        { id: 'artifact:out', label: 'Artifact', name: 'outside', file_path: 'out.rs', in_degree: 0, out_degree: 0 },
      ] : []
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify(payload) }] } }) })
    }))
    const user = userEvent.setup()
    render(<ExplorerSearch onFocus={() => {}} scopeNodeIds={['artifact:in']} />)
    await user.type(screen.getByLabelText('Search graph'), 'side')
    expect(await screen.findByRole('button', { name: /inside.*Artifact/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /outside.*Artifact/ })).not.toBeInTheDocument()
  })

  it('identifies and focuses the containing cluster', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 'symbol:app#run', label: 'Symbol', name: 'run', file_path: 'src/api/app.py', in_degree: 1, out_degree: 2 }]) }] } }) }))
    const focusCluster = vi.fn()
    const user = userEvent.setup()
    const cluster = { id: 'cluster:api', members: ['symbol:app#run'], top_nodes: [], packages: [], edge_types: ['Calls'], cohesion: 0.4, incoming_pressure: 1, outgoing_pressure: 2 }
    render(<ExplorerSearch onFocus={() => {}} clusters={[cluster]} onFocusCluster={focusCluster} />)

    await user.type(screen.getByLabelText('Search graph'), 'run')
    const button = await screen.findByRole('button', { name: /Focus containing cluster/i })
    await user.click(button)

    expect(focusCluster).toHaveBeenCalledWith(cluster)
  })
})
