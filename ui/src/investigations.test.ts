import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { exportInvestigation, loadInvestigations, saveInvestigation } from './investigations'

describe('saved investigations', () => {
  let values = new Map<string, string>()
  beforeEach(() => {
    values = new Map()
    vi.stubGlobal('localStorage', { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value), clear: () => values.clear() })
  })
  afterEach(() => vi.unstubAllGlobals())
  it('persists a versioned investigation and replaces the same id', () => {
    saveInvestigation({ version: 1, id: 'x', name: 'Risk', graphSnapshotId: 'g1', urlState: { viewMode: 'radial' }, activeLabels: ['Symbol'], notes: 'first' })
    saveInvestigation({ version: 1, id: 'x', name: 'Risk', graphSnapshotId: 'g1', urlState: { viewMode: 'matrix' }, activeLabels: [], notes: 'updated' })
    expect(loadInvestigations()).toEqual([expect.objectContaining({ id: 'x', notes: 'updated', version: 1 })])
  })
  it('exports a portable versioned report', () => {
    const report = exportInvestigation({ version: 1, id: 'x', name: 'Risk', graphSnapshotId: 'g1', urlState: { viewMode: 'radial' }, activeLabels: [], focusedSubgraph: { center_node: null, nodes: [], edges: [], budget: { node_budget: 1, edge_budget: 1, nodes_available: 0, edges_available: 0, nodes_returned: 0, edges_returned: 0, nodes_truncated: false, edges_truncated: false } }, healthFindings: [{ id: 'h1', rule: 'cycle', severity: 'warning', affected_nodes: [], evidence: [], investigation_query: 'MATCH' }], notes: '' })
    expect(report).toContain('"format": "lithograph-investigation-report"')
    expect(report).toContain('"graphSnapshotId": "g1"')
    expect(report).toContain('"healthFindings"')
  })
  it('isolates saved investigations by project id', () => {
    saveInvestigation({ version: 1, id: 'primary:x', name: 'Main', graphSnapshotId: 'g1', urlState: { projectId: 'primary', viewMode: 'cluster' }, activeLabels: [], notes: '' })
    saveInvestigation({ version: 1, id: 'web:x', name: 'Web', graphSnapshotId: 'g2', urlState: { projectId: 'web', viewMode: 'cluster' }, activeLabels: [], notes: '' })
    expect(loadInvestigations('primary').map((item) => item.name)).toEqual(['Main'])
    expect(loadInvestigations('web').map((item) => item.name)).toEqual(['Web'])
  })
})
