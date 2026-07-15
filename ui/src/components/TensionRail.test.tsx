import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { TensionRail } from './TensionRail'

describe('TensionRail', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })

  it('switches heatmap labels and focuses a hotspot', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 't1', category: 'CouplingHotspot', severity: 'High', confidence: 'High', affected_nodes: ['symbol:a'], metric_inputs: { degree: 9 }, evidence_references: ['degree=9'], follow_up_queries: ['MATCH (n)'], explanation: 'tight coupling' }]) }] } }) }))
    const focus = vi.fn()
    const selectTension = vi.fn()
    const inspect = vi.fn()
    const useQuery = vi.fn()
    const user = userEvent.setup()
    render(<TensionRail onFocus={focus} onInspect={inspect} onUseQuery={useQuery} onSelectTension={selectTension} />)
    await waitFor(() => expect(screen.getByRole('button', { name: /High.*tight coupling/ })).toBeInTheDocument())
    await user.selectOptions(screen.getByLabelText('Tension heatmap mode'), 'category')
    expect(screen.getByRole('button', { name: /CouplingHotspot.*tight coupling/ })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /CouplingHotspot.*tight coupling/ }))
    expect(focus).toHaveBeenCalledWith('symbol:a')
    expect(selectTension).toHaveBeenCalledWith(expect.objectContaining({ id: 't1' }))
    expect(screen.getByText('Why this hotspot matters')).toBeInTheDocument()
    expect(screen.getAllByText('degree=9')).toHaveLength(2)
    expect(screen.getByText('MATCH (n)')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'symbol:a' }))
    expect(inspect).toHaveBeenCalledWith('symbol:a')
    await user.click(screen.getByRole('button', { name: 'MATCH (n)' }))
    expect(useQuery).toHaveBeenCalledWith('MATCH (n)')
  })

  it('hands a tension off to dependency tracing and exposes focusable trace nodes', async () => {
    vi.stubGlobal('fetch', vi.fn().mockImplementation((_: RequestInfo, init?: RequestInit) => {
      const tool = JSON.parse(init?.body as string).params.name
      const payload = tool === 'get_repository_tensions'
        ? [{ id: 't1', category: 'BlastRadius', severity: 'High', confidence: 'Medium', affected_nodes: ['symbol:a'], metric_inputs: { downstream: 4 }, evidence_references: ['symbol:a'], follow_up_queries: ['MATCH path'], explanation: 'large downstream surface' }]
        : { root: { id: 'symbol:a', label: 'Symbol', name: 'a', file_path: 'a.rs', in_degree: 0, out_degree: 1 }, visited: [{ node: { id: 'symbol:a', label: 'Symbol', name: 'a', file_path: 'a.rs', in_degree: 0, out_degree: 1 }, hop: 0 }, { node: { id: 'symbol:b', label: 'Symbol', name: 'b', file_path: 'b.rs', in_degree: 1, out_degree: 0 }, hop: 1 }], relations: [{ source: 'symbol:a', target: 'symbol:b', kind: 'Calls' }] }
      return Promise.resolve({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify(payload) }] } }) })
    }))
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<TensionRail onFocus={focus} />)
    await user.click(await screen.findByRole('button', { name: /High.*large downstream surface/ }))
    await user.click(screen.getByRole('button', { name: 'Trace dependency/call paths' }))
    await waitFor(() => expect(screen.getByText('Related trace')).toBeInTheDocument())
    expect(screen.getByText('1 affected nodes · 1 evidence relations')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'b' }))
    expect(focus).toHaveBeenLastCalledWith('symbol:b')
  })

  it('reopens a hotspot requested by a shared tension URL', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 'shared-cycle', category: 'DependencyCycle', severity: 'Critical', confidence: 'High', affected_nodes: ['symbol:cycle'], metric_inputs: {}, evidence_references: [], follow_up_queries: [], explanation: 'cycle' }]) }] } }) }))
    const focus = vi.fn()
    render(<TensionRail onFocus={focus} requestedTensionId="shared-cycle" />)
    await waitFor(() => expect(screen.getByText('Why this hotspot matters')).toBeInTheDocument())
    expect(focus).toHaveBeenCalledWith('symbol:cycle')
  })

  it('limits hotspots to the active tag scope', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([
      { id: 'inside', category: 'DependencyCycle', severity: 'High', confidence: 'High', affected_nodes: ['symbol:in'], metric_inputs: {}, evidence_references: [], follow_up_queries: [], explanation: 'inside scope' },
      { id: 'outside', category: 'DependencyCycle', severity: 'High', confidence: 'High', affected_nodes: ['symbol:out'], metric_inputs: {}, evidence_references: [], follow_up_queries: [], explanation: 'outside scope' },
    ]) }] } }) }))
    render(<TensionRail onFocus={() => {}} scopeNodeIds={['symbol:in']} />)
    expect(await screen.findByRole('button', { name: /inside scope/ })).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: /outside scope/ })).not.toBeInTheDocument()
  })

  it('collapses to a bounded summary so the graph remains usable', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, json: () => Promise.resolve({ result: { content: [{ text: JSON.stringify([{ id: 't1', category: 'CouplingHotspot', severity: 'High', confidence: 'High', affected_nodes: ['symbol:a'], metric_inputs: {}, evidence_references: [], follow_up_queries: [], explanation: 'tight coupling' }]) }] } }) }))
    const user = userEvent.setup()
    render(<TensionRail onFocus={() => {}} />)
    await screen.findByRole('button', { name: /High.*tight coupling/ })

    await user.click(screen.getByRole('button', { name: 'Collapse tension hotspots' }))

    expect(screen.queryByRole('button', { name: /High.*tight coupling/ })).not.toBeInTheDocument()
    expect(screen.getByText('1 signals · 1 high or critical')).toBeInTheDocument()
    expect(screen.getByRole('button', { name: 'Expand tension hotspots' })).toBeInTheDocument()
  })
})
