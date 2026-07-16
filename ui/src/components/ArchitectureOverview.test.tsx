import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ArchitectureOverview } from './ArchitectureOverview'
import { deriveRepositoryAreas } from '../architectureOverview'
import type { LayoutResult } from '../graph/types'
import type { ArchitectureSummary } from '../api/architecture'

const layout: LayoutResult = {
  graph_snapshot_id: 'snapshot', algorithm_version: 1, center_node: null,
  nodes: [
    { id: 'web', label: 'Artifact', name: 'App.tsx', file_path: 'web/src/App.tsx', in_degree: 0, out_degree: 1, x: 0, y: 0, hop: 0 },
    { id: 'api', label: 'Artifact', name: 'service.py', file_path: 'src/api/service.py', in_degree: 1, out_degree: 0, x: 1, y: 1, hop: 0 },
    { id: 'run', label: 'Command', name: 'run server', file_path: 'src/api/service.py', in_degree: 0, out_degree: 1, x: 2, y: 2, hop: 0 },
  ],
  edges: [{ source: 'web', target: 'api', kind: 'Calls' }, { source: 'run', target: 'api', kind: 'Calls' }],
  budget: { node_budget: 150, edge_budget: 400, nodes_available: 3, edges_available: 2, nodes_returned: 3, edges_returned: 2, nodes_truncated: false, edges_truncated: false },
}
const architecture: ArchitectureSummary = {
  clusters: [],
  entry_points: [{ id: 'run', label: 'Command', name: 'run server', file_path: 'src/api/service.py', in_degree: 0, out_degree: 1 }],
  hotspots: [{ id: 'api', label: 'Artifact', name: 'service.py', file_path: 'src/api/service.py', in_degree: 1, out_degree: 0 }],
}

describe('ArchitectureOverview', () => {
  afterEach(cleanup)

  it('derives stable human areas and directed cross-area dependencies', () => {
    const areas = deriveRepositoryAreas(layout, architecture.entry_points, [])
    expect(areas.map((area) => area.id)).toEqual(['src/api', 'web'])
    expect(areas[0]).toMatchObject({ name: 'Api', nodeCount: 2, fileCount: 1, incoming: 1, outgoing: 0 })
    expect(areas[1]).toMatchObject({ name: 'Web', incoming: 0, outgoing: 1, connectedAreas: ['Api'] })
  })

  it('answers first-run questions and scopes or focuses from the answer', async () => {
    const user = userEvent.setup()
    const onScope = vi.fn()
    const onFocus = vi.fn()
    render(<ArchitectureOverview layout={layout} architecture={architecture} tensions={[]} scopedNodeIds={[]} onScopeArea={onScope} onClearScope={vi.fn()} onFocus={onFocus} onSelectTension={vi.fn()} onOpenFiles={vi.fn()} />)
    expect(screen.getByText('How this application is organized')).toBeInTheDocument()
    expect(screen.getByText('entry points')).toBeInTheDocument()
    expect(screen.getByText(/connects to Api/)).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Open Web area' }))
    await user.click(screen.getByRole('button', { name: /run server/ }))
    expect(onScope).toHaveBeenCalledWith(expect.objectContaining({ id: 'web', nodeIds: ['web'] }))
    expect(onFocus).toHaveBeenCalledWith('run')
  })

  it('derives areas below a common hash root without changing node ids', () => {
    const hash = '0123456789abcdef0123456789abcdef'
    const rooted = {
      ...layout,
      nodes: layout.nodes.map((node) => ({ ...node, id: `${node.id}:${hash}`, name: `.cache/${hash}/${node.file_path}`, file_path: `.cache/${hash}/${node.file_path}` })),
      edges: [],
    }
    const areas = deriveRepositoryAreas(rooted, [], [])
    expect(areas.map((area) => area.id)).toEqual(['src/api', 'web'])
    expect(areas.flatMap((area) => area.nodeIds)).toContain(`web:${hash}`)
    expect(areas.map((area) => area.name).join(' ')).not.toContain(hash)
  })
})
