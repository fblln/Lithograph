import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { MutedClusterCount } from './ClusterHulls'
import { MutedGraphNoiseIndicator } from './GraphScene'
import { computeClusterLayout } from './clusterLayout'

describe('MutedClusterCount', () => {
  afterEach(cleanup)

  it('shows excluded unresolved/external members as muted metadata', () => {
    const { rerender } = render(<MutedClusterCount count={2} />)
    expect(screen.getByText('+2 unresolved/external')).toHaveStyle({ color: 'var(--atlas-text-faint)' })
    rerender(<MutedClusterCount count={0} />)
    expect(screen.queryByText(/unresolved\/external/)).not.toBeInTheDocument()
  })

  it('reports a pure unresolved cluster without drawing an empty hull', () => {
    const result = computeClusterLayout(
      [{ id: 'u', label: 'Unresolved', name: 'missing', file_path: null, in_degree: 0, out_degree: 0, x: 0, y: 0, hop: 0 }],
      [{ id: 'noise', members: ['u'], top_nodes: [], packages: [], edge_types: [], cohesion: 0, incoming_pressure: 0, outgoing_pressure: 0 }],
      [],
    )
    render(<MutedGraphNoiseIndicator count={result.mutedNodeCount ?? 0} />)

    expect(result.clusters).toEqual([])
    expect(result.membership.get('u')).toBe('noise')
    expect(screen.getByLabelText('Suppressed graph noise')).toHaveTextContent('1 unresolved/external node hidden')
  })
})
