import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { GraphToolbar } from './GraphToolbar'

describe('GraphToolbar', () => {
  afterEach(cleanup)

  it('connects graph, overlay, edge, layout, and camera controls', async () => {
    const user = userEvent.setup()
    const onViewMode = vi.fn()
    const onOverlayMode = vi.fn()
    const onEdgeView = vi.fn()
    const onResetLayout = vi.fn()
    const onZoom = vi.fn()
    render(<GraphToolbar viewMode="radial" overlayMode="kind" edgeView="nodes" zoom={1} layoutCustomized truncated onViewMode={onViewMode} onOverlayMode={onOverlayMode} onEdgeView={onEdgeView} onResetLayout={onResetLayout} onZoom={onZoom} onRaiseBudget={vi.fn()} />)

    await user.click(screen.getByRole('button', { name: 'Cluster' }))
    await user.click(screen.getByRole('button', { name: 'Impact' }))
    await user.click(screen.getByRole('button', { name: 'clusters' }))
    await user.click(screen.getByRole('button', { name: 'Reset layout' }))
    await user.click(screen.getByRole('button', { name: 'Zoom in' }))

    expect(onViewMode).toHaveBeenCalledWith('cluster')
    expect(onOverlayMode).toHaveBeenCalledWith('blast')
    expect(onEdgeView).toHaveBeenCalledWith('clusters')
    expect(onResetLayout).toHaveBeenCalledOnce()
    expect(onZoom).toHaveBeenCalledWith(0.8)
    expect(screen.getByRole('status')).toHaveTextContent('The current graph slice is truncated by its budget')
  })

  it('hides conditional controls for an untouched complete layout', () => {
    render(<GraphToolbar viewMode="radial" overlayMode="kind" edgeView="nodes" zoom={1} layoutCustomized={false} truncated={false} onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)

    expect(screen.queryByRole('button', { name: 'Reset layout' })).not.toBeInTheDocument()
    expect(screen.queryByText(/current graph slice is truncated/)).not.toBeInTheDocument()
  })

  it('exposes discoverable relationship-kind filters', async () => {
    const user = userEvent.setup()
    const onToggleEdgeKind = vi.fn()
    const counts = new Map([['Calls', 3], ['Imports', 1]])
    const { rerender } = render(<GraphToolbar viewMode="cluster" overlayMode="kind" edgeView="clusters" zoom={1} layoutCustomized={false} truncated={false} availableEdgeKinds={['Calls', 'Imports']} edgeKindCounts={counts} activeEdgeKinds={new Set()} onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onToggleEdgeKind={onToggleEdgeKind} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)
    await user.click(screen.getByText('Relationship kinds · current slice'))
    expect(screen.getByText('Counts describe returned relationships in the current slice.')).toBeInTheDocument()
    expect(screen.getByRole('checkbox', { name: 'Calls: 3 in current slice' }).closest('label')).toHaveTextContent('Calls3')
    await user.click(screen.getByRole('checkbox', { name: 'Calls: 3 in current slice' }))
    expect(onToggleEdgeKind).toHaveBeenCalledWith('Calls')

    rerender(<GraphToolbar viewMode="cluster" overlayMode="kind" edgeView="clusters" zoom={1} layoutCustomized={false} truncated={false} availableEdgeKinds={['Calls', 'Imports']} edgeKindCounts={counts} activeEdgeKinds={new Set(['Imports'])} onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onToggleEdgeKind={onToggleEdgeKind} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)
    expect(screen.getByRole('checkbox', { name: 'Calls: 3 in current slice' }).closest('label')).toHaveTextContent('Calls3')
  })

  it('marks current-slice relationship counts as truncated independently of node truncation', async () => {
    const user = userEvent.setup()
    render(<GraphToolbar viewMode="cluster" overlayMode="kind" edgeView="clusters" zoom={1} layoutCustomized={false} truncated edgeCountsTruncated availableEdgeKinds={['Calls']} edgeKindCounts={new Map([['Calls', 2]])} onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)
    await user.click(screen.getByText('Relationship kinds · current slice'))
    expect(screen.getByText('Counts describe returned relationships in the current slice; additional relationships are truncated.')).toBeInTheDocument()
  })

  it('explains resolution styles and toggles unproven edges', async () => {
    const user = userEvent.setup()
    const onShowUnprovenEdges = vi.fn()
    render(<GraphToolbar viewMode="cluster" overlayMode="kind" edgeView="nodes" zoom={1} layoutCustomized={false} truncated={false} showUnprovenEdges onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onShowUnprovenEdges={onShowUnprovenEdges} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)

    expect(screen.getByLabelText('Edge resolution legend')).toHaveTextContent('Proven')
    expect(screen.getByLabelText('Edge resolution legend')).toHaveTextContent('Syntax only')
    expect(screen.getByLabelText('Edge resolution legend')).toHaveTextContent('Fallback')
    await user.click(screen.getByRole('checkbox', { name: 'Show syntax-only and fallback edges' }))
    expect(onShowUnprovenEdges).toHaveBeenCalledWith(false)
  })
})
