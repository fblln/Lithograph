import { useCallback, useEffect, useState } from 'react'
import { getGraphLayout } from './api/graph'
import { getClusters, type ArchitectureCluster } from './api/architecture'
import { RpcError } from './api/rpc'
import type { LayoutResult, PositionedNode } from './graph/types'
import { GraphScene } from './graph/GraphScene'
import { useDragPositions } from './graph/useDragPositions'
import { parseUrlState, serializeUrlState } from './urlState'
import { TopBar } from './components/TopBar'
import { Sidebar } from './components/Sidebar'
import { DetailPanel } from './components/DetailPanel'
import { ViewModeToggle, type ViewMode } from './components/ViewModeToggle'

type Status = 'loading' | 'ready' | 'error'

const initialUrlState = parseUrlState(window.location.search)

function App() {
  const [layout, setLayout] = useState<LayoutResult | null>(null)
  const [status, setStatus] = useState<Status>('loading')
  const [error, setError] = useState<string | null>(null)
  const [centerNode, setCenterNode] = useState<string | undefined>(initialUrlState.centerNode)
  const [selected, setSelected] = useState<PositionedNode | null>(null)
  const [activeLabels, setActiveLabels] = useState<Set<string>>(new Set())
  const [viewMode, setViewMode] = useState<ViewMode>(initialUrlState.viewMode)
  const [maxNodes, setMaxNodes] = useState<number | undefined>(initialUrlState.maxNodes)
  const [clusters, setClusters] = useState<ArchitectureCluster[]>([])
  const dragPositions = useDragPositions(layout?.graph_snapshot_id ?? '')

  const load = useCallback(
    (request: { center_node?: string; node_labels?: string[]; max_nodes?: number }) => {
      setStatus('loading')
      getGraphLayout(request)
        .then((result) => {
          setLayout(result)
          setStatus('ready')
          setError(null)
        })
        .catch((cause: unknown) => {
          setStatus('error')
          setError(cause instanceof RpcError ? cause.message : String(cause))
        })
    },
    [],
  )

  useEffect(() => {
    load({ center_node: centerNode, node_labels: [...activeLabels], max_nodes: maxNodes })
  }, [centerNode, activeLabels, maxNodes, load])

  // Keeps the URL a shareable/bookmarkable snapshot of the current view
  // (LIT-24.17 AC1). `replaceState` rather than `pushState`: every field
  // here already has its own undo path in the UI (Clear, view toggle,
  // budget field), so it isn't worth spamming browser history on top of
  // that.
  useEffect(() => {
    const query = serializeUrlState({ centerNode, viewMode, maxNodes })
    window.history.replaceState(null, '', `${window.location.pathname}${query}`)
  }, [centerNode, viewMode, maxNodes])

  // Architecture clusters are a whole-graph property, not scoped to the
  // current focused/budgeted layout slice, so they're fetched once
  // independently of `load` rather than re-fetched on every layout change.
  useEffect(() => {
    getClusters()
      .then(setClusters)
      .catch(() => setClusters([]))
  }, [])

  function handleSelect(node: PositionedNode) {
    setSelected(node)
  }

  function handleFocus(node: PositionedNode) {
    setCenterNode(node.id)
    setSelected(null)
  }

  function handleToggleLabel(label: string) {
    setActiveLabels((previous) => {
      const next = new Set(previous)
      if (next.has(label)) next.delete(label)
      else next.add(label)
      return next
    })
  }

  return (
    <div className="flex h-full flex-col">
      <TopBar centerLabel={layout?.center_node ?? 'overview'} status={status} />
      <div className="flex min-h-0 flex-1">
        {layout && (
          <Sidebar
            layout={layout}
            activeLabels={activeLabels}
            onToggleLabel={handleToggleLabel}
            maxNodes={maxNodes}
            onApplyMaxNodes={setMaxNodes}
          />
        )}
        <main
          className="relative min-w-0 flex-1"
          style={{ background: 'var(--atlas-canvas)' }}
        >
          <div className="absolute top-3 right-3 z-10">
            <ViewModeToggle mode={viewMode} onChange={setViewMode} />
          </div>
          {status === 'error' && (
            <p className="absolute inset-x-0 top-4 mx-auto w-fit rounded bg-red-950 px-3 py-1.5 text-[12px] text-red-200">
              {error}
            </p>
          )}
          {layout && (
            <GraphScene
              layout={layout}
              viewMode={viewMode}
              clusters={clusters}
              selectedId={selected?.id ?? null}
              onSelect={handleSelect}
              dragPositions={dragPositions}
            />
          )}
        </main>
        <DetailPanel node={selected} onFocus={handleFocus} onClear={() => setSelected(null)} />
      </div>
    </div>
  )
}

export default App
