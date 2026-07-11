import { afterEach, describe, expect, it } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import { StatsPanel } from './StatsPanel'
import type { LayoutResult } from '../graph/types'

function layout(overrides: Partial<LayoutResult> = {}): LayoutResult {
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
        in_degree: 1,
        out_degree: 3,
        x: 0,
        y: 0,
        hop: 0,
      },
      {
        id: 'b',
        label: 'Artifact',
        name: 'b.rs',
        file_path: 'b.rs',
        in_degree: 0,
        out_degree: 0,
        x: 0,
        y: 0,
        hop: 1,
      },
    ],
    edges: [
      { source: 'a', target: 'b', kind: 'Calls' },
      { source: 'a', target: 'b', kind: 'Calls' },
      { source: 'b', target: 'a', kind: 'Imports' },
    ],
    budget: {
      node_budget: 150,
      edge_budget: 400,
      nodes_available: 2,
      edges_available: 3,
      nodes_returned: 2,
      edges_returned: 3,
      nodes_truncated: false,
      edges_truncated: false,
    },
    ...overrides,
  }
}

describe('StatsPanel', () => {
  afterEach(() => {
    cleanup()
  })

  it('shows edge-kind counts sorted by descending frequency', () => {
    render(<StatsPanel layout={layout()} />)
    const calls = screen.getByText('Calls')
    const imports = screen.getByText('Imports')
    expect(calls).toBeInTheDocument()
    expect(imports).toBeInTheDocument()
    expect(screen.getByText('2')).toBeInTheDocument() // Calls count
  })

  it('ranks the most-connected node first by total degree', () => {
    render(<StatsPanel layout={layout()} />)
    // "a" has degree 4 (1 in + 3 out), "b" has degree 0 -- both listed,
    // "a" should appear before "b" in document order.
    const names = screen.getAllByText(/\.rs$/).map((el) => el.textContent)
    expect(names.indexOf('a.rs')).toBeLessThan(names.indexOf('b.rs'))
  })

  it('shows a graceful empty state instead of empty lists', () => {
    render(<StatsPanel layout={layout({ nodes: [], edges: [] })} />)
    expect(screen.getByText('No edges in the current view.')).toBeInTheDocument()
    expect(screen.getByText('No nodes in the current view.')).toBeInTheDocument()
  })
})
