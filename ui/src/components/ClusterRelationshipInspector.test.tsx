import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { ClusterRelationshipInspector } from './ClusterRelationshipInspector'

describe('ClusterRelationshipInspector', () => {
  afterEach(cleanup)
  it('shows directed aggregate counts and expands to inspect underlying relationships', async () => {
    const user = userEvent.setup()
    const edge = { source: 'a', target: 'b', kind: 'Calls' }
    const onInspectRelationship = vi.fn()
    render(<ClusterRelationshipInspector
      selectedClusterId="api"
      clusterLayout={{ positions: new Map(), membership: new Map(), clusters: [], links: [{ source: 'api', target: 'web', count: 2, kinds: [{ kind: 'Calls', count: 2 }], underlying: [edge] }] }}
      identities={new Map([
        ['api', { id: 'api', name: 'Python API', responsibility: 'Serves requests.', memberCount: 4, visibleMemberCount: 3, fileCount: 2, dominantKinds: ['Route'], entryPoints: [], incoming: [], outgoing: [], boundaryInterpretation: 'Shared.', tensionCount: 1, highestSeverity: 'High', partial: true }],
        ['web', { id: 'web', name: 'Web frontend', responsibility: 'Renders UI.', memberCount: 2, visibleMemberCount: 2, fileCount: 1, dominantKinds: ['Artifact'], entryPoints: [], incoming: [], outgoing: [], boundaryInterpretation: 'Shared.', tensionCount: 0, partial: false }],
      ])}
      onInspectRelationship={onInspectRelationship}
    />)
    expect(screen.getByText('2 relationships · Calls 2')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /Web frontend/ }))
    await user.click(screen.getByRole('button', { name: /Calls.*a.*b/ }))
    expect(onInspectRelationship).toHaveBeenCalledWith(edge)
  })
})
