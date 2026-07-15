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
    expect(screen.getByRole('status')).toHaveTextContent('Graph budget is truncating this view')
  })

  it('hides conditional controls for an untouched complete layout', () => {
    render(<GraphToolbar viewMode="radial" overlayMode="kind" edgeView="nodes" zoom={1} layoutCustomized={false} truncated={false} onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)

    expect(screen.queryByRole('button', { name: 'Reset layout' })).not.toBeInTheDocument()
    expect(screen.queryByText(/Graph budget is truncating/)).not.toBeInTheDocument()
  })

  it('exposes discoverable relationship-kind filters', async () => {
    const user = userEvent.setup()
    const onToggleEdgeKind = vi.fn()
    render(<GraphToolbar viewMode="cluster" overlayMode="kind" edgeView="clusters" zoom={1} layoutCustomized={false} truncated={false} availableEdgeKinds={['Calls', 'Imports']} activeEdgeKinds={new Set()} onViewMode={vi.fn()} onOverlayMode={vi.fn()} onEdgeView={vi.fn()} onToggleEdgeKind={onToggleEdgeKind} onResetLayout={vi.fn()} onZoom={vi.fn()} onRaiseBudget={vi.fn()} />)
    await user.click(screen.getByText('Relationship kinds'))
    await user.click(screen.getByRole('checkbox', { name: 'Calls' }))
    expect(onToggleEdgeKind).toHaveBeenCalledWith('Calls')
  })
})
