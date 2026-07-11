import type { ViewMode } from './components/ViewModeToggle'

/**
 * The subset of App's state that makes a view worth bookmarking or
 * sharing: which node is focused, which layout mode is active, and what
 * node budget was requested. Filters (`node_labels`) are deliberately
 * left out for now -- they change too often (one click per toggle) to be
 * worth a URL write on every change; revisit if that turns out wrong.
 */
export interface ExplorerUrlState {
  centerNode?: string
  viewMode: ViewMode
  maxNodes?: number
}

export const DEFAULT_VIEW_MODE: ViewMode = 'radial'

/** Parses `location.search` (a leading `?...` or a bare query string) into explorer state, tolerating missing/invalid values by falling back to defaults rather than throwing. */
export function parseUrlState(search: string): ExplorerUrlState {
  const params = new URLSearchParams(search)
  const centerNode = params.get('center')
  const viewModeParam = params.get('view')
  const viewMode: ViewMode = viewModeParam === 'matrix' ? 'matrix' : DEFAULT_VIEW_MODE
  const maxNodesParam = params.get('maxNodes')
  const maxNodes = maxNodesParam === null ? undefined : Number(maxNodesParam)

  return {
    centerNode: centerNode ?? undefined,
    viewMode,
    maxNodes: maxNodes !== undefined && Number.isFinite(maxNodes) && maxNodes > 0 ? maxNodes : undefined,
  }
}

/** Inverse of `parseUrlState`: only writes params that differ from the default, so a plain overview view keeps a clean, empty URL. Returns an empty string (not `?`) when there's nothing to encode. */
export function serializeUrlState(state: ExplorerUrlState): string {
  const params = new URLSearchParams()
  if (state.centerNode) params.set('center', state.centerNode)
  if (state.viewMode !== DEFAULT_VIEW_MODE) params.set('view', state.viewMode)
  if (state.maxNodes !== undefined) params.set('maxNodes', String(state.maxNodes))

  const query = params.toString()
  return query ? `?${query}` : ''
}
