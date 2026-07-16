import type { ExplorerUrlState } from './urlState'
import type { QueryRow } from './api/query'
import type { HealthFinding } from './api/analytics'
import type { LayoutResult } from './graph/types'
import type { RepositoryTension } from './api/tensions'
import type { EdgeView, OverlayMode } from './components/GraphToolbar'

const STORAGE_KEY = 'lithograph-investigations:v1'

/** The query workbench state is data-only so it remains portable in exports. */
export interface InvestigationQueryState {
  query: string
  rows: QueryRow[]
}

/** The rendered graph slice is retained in the export for offline review. */
export type InvestigationSubgraph = Pick<LayoutResult, 'center_node' | 'nodes' | 'edges' | 'budget'>

export interface SavedInvestigation {
  version: 1
  id: string
  name: string
  graphSnapshotId: string
  urlState: ExplorerUrlState
  selectedNodeId?: string
  activeLabels: string[]
  /** Normalized graph metric values used by the canvas overlay. */
  metricValues?: Array<[string, number]>
  queryState?: InvestigationQueryState
  focusedSubgraph?: InvestigationSubgraph
  healthFindings?: HealthFinding[]
  selectedTension?: RepositoryTension
  overlayMode?: OverlayMode
  edgeView?: EdgeView
  notes: string
}

function loadAllInvestigations(): SavedInvestigation[] {
  try {
    const value: unknown = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]')
    return Array.isArray(value) ? value.filter(isInvestigation) : []
  } catch { return [] }
}

/** Loads saved views only for the selected allowlisted project. */
export function loadInvestigations(projectId = 'primary'): SavedInvestigation[] {
  return loadAllInvestigations().filter((item) => (item.urlState.projectId ?? 'primary') === projectId)
}

export function saveInvestigation(investigation: SavedInvestigation): void {
  const next = loadAllInvestigations().filter((item) => item.id !== investigation.id)
  next.push(investigation)
  localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
}

/** A portable, versioned report payload suitable for a download or clipboard. */
export function exportInvestigation(investigation: SavedInvestigation): string {
  return JSON.stringify({ format: 'lithograph-investigation-report', version: 1, investigation }, null, 2)
}

function isInvestigation(value: unknown): value is SavedInvestigation {
  return typeof value === 'object' && value !== null && (value as SavedInvestigation).version === 1 && typeof (value as SavedInvestigation).id === 'string'
}
