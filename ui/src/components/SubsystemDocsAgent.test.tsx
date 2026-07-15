import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { SubsystemDocsAgent } from './SubsystemDocsAgent'

const generate = vi.fn()
const refine = vi.fn()
vi.mock('../api/subsystemDocs', () => ({
  generateSubsystemDocument: (...args: unknown[]) => generate(...args),
  refineSubsystemDocument: (...args: unknown[]) => refine(...args),
}))

function document(markdown: string, snapshot = 'g1') {
  return { subsystem_id: 'cluster-a', graph_snapshot_id: snapshot, prompt_version: 'v1', confidence: 'High', cited_nodes: ['node:a'], cited_edges: ['edge:a-b'], source_spans: [], unresolved_assumptions: [], markdown, resolved_tags: [] }
}

describe('SubsystemDocsAgent', () => {
  afterEach(() => { cleanup(); vi.clearAllMocks(); vi.unstubAllGlobals() })

  it('shows exact context, generates, refines, compares, accepts, reverts, and follows evidence', async () => {
    generate.mockResolvedValue(document('version one'))
    refine.mockResolvedValue(document('version two'))
    const onFocus = vi.fn()
    const user = userEvent.setup()
    render(<SubsystemDocsAgent context={{ scopeId: 'cluster-a', nodeIds: ['node:a', 'node:b'], edgeCount: 1, evidenceCount: 2, tensionCount: 1, graphSnapshotId: 'g1' }} onFocus={onFocus} />)
    expect(screen.getByLabelText('Agent context')).toHaveTextContent('2 nodes · 1 edges · 2 evidence refs · 1 tensions')
    await user.click(screen.getByRole('button', { name: 'Generate with graph agent' }))
    expect(await screen.findByText('version one')).toBeInTheDocument()
    await user.type(screen.getByLabelText('Refinement instruction'), 'add operations')
    await user.click(screen.getByRole('button', { name: 'Refine' }))
    expect(await screen.findByText('version two')).toBeInTheDocument()
    expect(refine).toHaveBeenCalledWith('cluster-a', ['node:a', 'node:b'], 'add operations')
    await user.click(screen.getByRole('button', { name: 'Compare' }))
    expect(screen.getByLabelText('Previous version')).toHaveTextContent('version one')
    await user.click(screen.getByRole('button', { name: 'Accept' }))
    expect(screen.getByText('accepted')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'node:a' }))
    expect(onFocus).toHaveBeenCalledWith('node:a')
    await user.click(screen.getByRole('button', { name: 'Revert' }))
    expect(screen.getByText('Version 1/2')).toBeInTheDocument()
  })

  it('warns and blocks acceptance when generated context is stale', async () => {
    generate.mockResolvedValue(document('old', 'old-snapshot'))
    const user = userEvent.setup()
    render(<SubsystemDocsAgent context={{ scopeId: 'cluster-a', nodeIds: ['node:a'], edgeCount: 0, evidenceCount: 0, tensionCount: 0, graphSnapshotId: 'g1' }} onFocus={() => {}} />)
    await user.click(screen.getByRole('button', { name: 'Generate with graph agent' }))
    expect(await screen.findByRole('alert')).toHaveTextContent('stale')
    expect(screen.getByRole('button', { name: 'Accept' })).toBeDisabled()
  })

  it('restores snapshot-bound versions with a saved investigation context', async () => {
    const values = new Map<string, string>()
    vi.stubGlobal('localStorage', { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) })
    generate.mockResolvedValue(document('persisted version'))
    const context = { scopeId: 'cluster-a', nodeIds: ['node:a'], edgeCount: 0, evidenceCount: 0, tensionCount: 0, graphSnapshotId: 'g1' }
    const user = userEvent.setup()
    const first = render(<SubsystemDocsAgent context={context} onFocus={() => {}} />)
    await user.click(screen.getByRole('button', { name: 'Generate with graph agent' }))
    expect(await screen.findByText('persisted version')).toBeInTheDocument()
    first.unmount()
    render(<SubsystemDocsAgent context={context} onFocus={() => {}} />)
    expect(await screen.findByText('persisted version')).toBeInTheDocument()
  })
})
