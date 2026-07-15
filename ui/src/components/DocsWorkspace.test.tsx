import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { DocsWorkspace } from './DocsWorkspace'

const getGraphDocument = vi.fn()
const regenerateGraphDocument = vi.fn()
vi.mock('../api/docs', () => ({
  getGraphDocument: () => getGraphDocument(),
  regenerateGraphDocument: (sectionIds?: string[]) => regenerateGraphDocument(sectionIds),
}))

const result = {
  document: {
    id: 'architecture-ops',
    graph_snapshot_id: 'snapshot-old',
    schema_version: 1,
    sections: [
      { id: 'overview', kind: 'SystemOverview', title: 'System Overview', source_query_ids: ['graph:overview'], evidence_references: ['artifact:README.md'], affected_nodes: ['artifact:README.md'], affected_edges: [], confidence: 'High', graph_snapshot_id: 'snapshot-old', deep_link_target: 'graph://focus?section=overview', tags: [{ id: 'tag:system', namespace: 'topic', value: 'system', source: 'Architecture', confidence: 'High' }] },
      { id: 'risks', kind: 'Risk', title: 'Risk / Tension Summary', source_query_ids: ['graph:tensions'], evidence_references: [], affected_nodes: ['symbol:risk'], affected_edges: [], confidence: 'High', graph_snapshot_id: 'snapshot-old', deep_link_target: 'graph://focus?section=risks', tags: [] },
    ],
  },
  markdown: '# Architecture and Operations\n\n## System Overview\n\n<!-- evidence -->\n\nOverview body.\n\n## Risk / Tension Summary\n\nRisk body.',
  freshness: 'current' as const,
  regenerated: false,
}

describe('DocsWorkspace', () => {
  afterEach(() => { cleanup(); vi.clearAllMocks() })

  it('navigates addressable sections and focuses graph evidence', async () => {
    getGraphDocument.mockResolvedValue(result)
    const user = userEvent.setup()
    const onSelectSection = vi.fn()
    const onFocus = vi.fn()
    render(<DocsWorkspace currentSnapshotId="snapshot-old" selectedSectionId="overview" onSelectSection={onSelectSection} onFocus={onFocus} />)
    expect(await screen.findByText('Overview body.')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Risk / Tension Summary' }))
    expect(onSelectSection).toHaveBeenCalledWith('risks')
    await user.click(screen.getByRole('button', { name: 'artifact:README.md' }))
    expect(onFocus).toHaveBeenCalledWith('artifact:README.md')
  })

  it('scopes related content and regenerates stale documents', async () => {
    getGraphDocument.mockResolvedValue(result)
    regenerateGraphDocument.mockResolvedValue({ ...result, document: { ...result.document, graph_snapshot_id: 'snapshot-new' }, regenerated: true })
    const user = userEvent.setup()
    render(<DocsWorkspace currentSnapshotId="snapshot-new" relatedEntityId="symbol:risk" onSelectSection={() => {}} onFocus={() => {}} />)
    expect(await screen.findByText('Risk body.')).toBeInTheDocument()
    expect(screen.queryByRole('button', { name: 'System Overview' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Regenerate section' }))
    expect(regenerateGraphDocument).toHaveBeenCalledWith(['risks'])
    expect(await screen.findByText('fresh')).toBeInTheDocument()
  })

  it('shows an explicit missing-section state', async () => {
    getGraphDocument.mockResolvedValue({ ...result, document: { ...result.document, sections: [] } })
    render(<DocsWorkspace currentSnapshotId="snapshot-old" selectedSectionId="missing" onSelectSection={() => {}} onFocus={() => {}} />)
    expect(await screen.findByRole('status')).toHaveTextContent('No documentation section is available')
  })

  it('filters sections by document tags and links a tag to its graph scope', async () => {
    getGraphDocument.mockResolvedValue(result)
    const onTagScope = vi.fn()
    const user = userEvent.setup()
    render(<DocsWorkspace currentSnapshotId="snapshot-old" onSelectSection={() => {}} onFocus={() => {}} onTagScope={onTagScope} />)
    await user.click(await screen.findByRole('button', { name: '#topic:system' }))
    expect(screen.queryByRole('button', { name: 'Risk / Tension Summary' })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'topic:system' }))
    expect(onTagScope).toHaveBeenCalledWith('topic:system', ['artifact:README.md'])
  })
})
