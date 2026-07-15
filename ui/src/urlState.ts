import type { ViewMode } from './components/ViewModeToggle'

/**
 * The subset of App's state that makes a view worth bookmarking or
 * sharing: which node is focused or selected, which layout mode is active,
 * what node budget was requested, and any active filters.
 */
export interface ExplorerUrlState {
  centerNode?: string
  viewMode: ViewMode
  maxNodes?: number
  maxEdges?: number
  nodeLabels?: string[]
  selectedNode?: string
  tensionId?: string
  tagExpression?: string
  workspaceMode?: 'explore' | 'docs'
  docSectionId?: string
}

export const DEFAULT_VIEW_MODE: ViewMode = 'cluster'

/** Parses `location.search` (a leading `?...` or a bare query string) into explorer state, tolerating missing/invalid values by falling back to defaults rather than throwing. */
export function parseUrlState(search: string): ExplorerUrlState {
  const params = new URLSearchParams(search)
  const centerNode = params.get('center')
  const viewModeParam = params.get('view')
  const viewMode: ViewMode = viewModeParam === 'radial' || viewModeParam === 'matrix' ? viewModeParam : DEFAULT_VIEW_MODE
  const maxNodesParam = params.get('maxNodes')
  const maxNodes = maxNodesParam === null ? undefined : Number(maxNodesParam)
  const maxEdgesParam = params.get('maxEdges')
  const maxEdges = maxEdgesParam === null ? undefined : Number(maxEdgesParam)
  const nodeLabels = params.get('labels')?.split(',').filter(Boolean)
  const selectedNode = params.get('selected')
  const tensionId = params.get('tension')
  const tagExpression = params.get('tags')
  const workspaceMode = params.get('workspace') === 'docs' ? 'docs' : undefined
  const docSectionId = params.get('doc')

  return {
    centerNode: centerNode ?? undefined,
    viewMode,
    maxNodes: maxNodes !== undefined && Number.isFinite(maxNodes) && maxNodes > 0 ? maxNodes : undefined,
    ...(maxEdges !== undefined && Number.isFinite(maxEdges) && maxEdges > 0 ? { maxEdges } : {}),
    ...(nodeLabels && nodeLabels.length > 0 ? { nodeLabels } : {}),
    ...(selectedNode ? { selectedNode } : {}),
    ...(tensionId ? { tensionId } : {}),
    ...(tagExpression ? { tagExpression } : {}),
    ...(workspaceMode ? { workspaceMode } : {}),
    ...(docSectionId ? { docSectionId } : {}),
  }
}

/** Inverse of `parseUrlState`: only writes params that differ from the default, so a plain overview view keeps a clean, empty URL. Returns an empty string (not `?`) when there's nothing to encode. */
export function serializeUrlState(state: ExplorerUrlState): string {
  const params = new URLSearchParams()
  if (state.centerNode) params.set('center', state.centerNode)
  if (state.viewMode !== DEFAULT_VIEW_MODE) params.set('view', state.viewMode)
  if (state.maxNodes !== undefined) params.set('maxNodes', String(state.maxNodes))
  if (state.maxEdges !== undefined) params.set('maxEdges', String(state.maxEdges))
  if (state.nodeLabels && state.nodeLabels.length > 0) params.set('labels', [...state.nodeLabels].sort().join(','))
  if (state.selectedNode) params.set('selected', state.selectedNode)
  if (state.tensionId) params.set('tension', state.tensionId)
  if (state.tagExpression) params.set('tags', state.tagExpression)
  if (state.workspaceMode === 'docs') params.set('workspace', 'docs')
  if (state.docSectionId) params.set('doc', state.docSectionId)

  const query = params.toString()
  return query ? `?${query}` : ''
}
