import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { Sidebar } from './Sidebar'
import type { LayoutResult } from '../graph/types'

function layout(): LayoutResult {
  return {
    graph_snapshot_id: 'blake3:test',
    algorithm_version: 1,
    center_node: null,
    nodes: [
      {
        id: 'a',
        label: 'Artifact',
        name: 'a.rs',
        file_path: 'a.rs',
        in_degree: 0,
        out_degree: 1,
        x: 0,
        y: 0,
        hop: 0,
      },
    ],
    edges: [{ source: 'a', target: 'a', kind: 'Contains' }],
    budget: {
      node_budget: 150,
      edge_budget: 400,
      nodes_available: 1,
      edges_available: 1,
      nodes_returned: 1,
      edges_returned: 1,
      nodes_truncated: false,
      edges_truncated: false,
    },
  }
}

describe('Sidebar', () => {
  afterEach(() => {
    cleanup()
  })

  it('opens with an architecture explanation and keeps the file navigator one step away', async () => {
    const user = userEvent.setup()
    render(
      <Sidebar
        layout={layout()}
        activeLabels={new Set()}
        onToggleLabel={vi.fn()}
        maxNodes={undefined}
        onApplyMaxNodes={vi.fn()}
      />,
    )
    expect(screen.getByText('How this application is organized')).toBeInTheDocument()
    expect(screen.getByText('Major areas')).toBeInTheDocument()
    expect(screen.getByText('bounded graph slice')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Browse files and directories →' }))
    expect(screen.getByRole('button', { name: /a.rs.*Artifact/ })).toBeInTheDocument()
  })

  it('switches to the Stats tab and shows edge-kind rollups instead', async () => {
    const user = userEvent.setup()
    render(
      <Sidebar
        layout={layout()}
        activeLabels={new Set()}
        onToggleLabel={vi.fn()}
        maxNodes={undefined}
        onApplyMaxNodes={vi.fn()}
      />,
    )

    await user.click(screen.getByRole('button', { name: 'Stats' }))

    expect(screen.getByText('Edge kinds')).toBeInTheDocument()
    expect(screen.queryByText('Node kinds')).not.toBeInTheDocument()
  })
})
