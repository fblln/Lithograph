import { useState } from 'react'

export interface BudgetControlProps {
  /** Currently applied max_nodes, or `undefined` for the server default. */
  value: number | undefined
  onApply: (maxNodes: number | undefined) => void
  edgeValue?: number
  onApplyEdges?: (maxEdges: number | undefined) => void
  availableNodes?: number
  availableEdges?: number
}

/**
 * Client-side large-graph guard (LIT-24.17 AC2): a request for more than
 * `LARGE_GRAPH_THRESHOLD` nodes is held back behind an explicit
 * confirmation rather than fired immediately. The server already hard-caps
 * every request at 2000 nodes (src/graph/layout.rs), so this isn't a
 * safety-of-the-server concern -- it's protecting the *browser*, since
 * instancing and drag/hull math for a much larger node count than the
 * default can visibly slow down interaction, and a user typing a big
 * number in a budget field deserves a chance to reconsider before that
 * happens rather than a silent stall.
 */
const LARGE_GRAPH_THRESHOLD = 500

export function BudgetControl({ value, onApply, edgeValue, onApplyEdges, availableNodes, availableEdges }: BudgetControlProps) {
  const [draft, setDraft] = useState(value === undefined ? '' : String(value))
  const [edgeDraft, setEdgeDraft] = useState(edgeValue === undefined ? '' : String(edgeValue))
  const [pending, setPending] = useState<number | null>(null)
  const [pendingFull, setPendingFull] = useState(false)

  function commit() {
    const trimmed = draft.trim()
    if (trimmed === '') {
      onApply(undefined)
      return
    }
    const parsed = Number(trimmed)
    if (!Number.isFinite(parsed) || parsed <= 0) return
    if (parsed > LARGE_GRAPH_THRESHOLD) {
      setPending(parsed)
      return
    }
    onApply(parsed)
  }

  function confirmLarge() {
    if (pending === null) return
    onApply(pending)
    setPending(null)
  }

  function commitEdges() {
    if (!onApplyEdges) return
    const trimmed = edgeDraft.trim()
    if (trimmed === '') { onApplyEdges(undefined); return }
    const parsed = Number(trimmed)
    if (Number.isFinite(parsed) && parsed > 0) onApplyEdges(parsed)
  }

  function confirmFull() {
    if (availableNodes !== undefined) onApply(availableNodes)
    if (availableEdges !== undefined) onApplyEdges?.(availableEdges)
    setDraft(availableNodes === undefined ? draft : String(availableNodes))
    setEdgeDraft(availableEdges === undefined ? edgeDraft : String(availableEdges))
    setPendingFull(false)
  }

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex items-center gap-2">
        <input
          type="number"
          min={1}
          value={draft}
          placeholder="150 (default)"
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === 'Enter') commit()
          }}
          className="w-24 rounded border px-2 py-1 text-[11px]"
          style={{
            background: 'var(--atlas-chip)',
            borderColor: 'var(--atlas-border)',
            color: 'var(--atlas-text-bright)',
          }}
        />
        <span className="text-[10px]" style={{ color: 'var(--atlas-text-dim)' }}>
          max nodes
        </span>
      </div>
      {onApplyEdges && <div className="flex items-center gap-2">
        <input type="number" min={1} value={edgeDraft} placeholder="400 (default)" onChange={(event) => setEdgeDraft(event.target.value)} onBlur={commitEdges} onKeyDown={(event) => { if (event.key === 'Enter') commitEdges() }} className="w-24 rounded border px-2 py-1 text-[11px]" style={{ background: 'var(--atlas-chip)', borderColor: 'var(--atlas-border)', color: 'var(--atlas-text-bright)' }} />
        <span className="text-[10px]" style={{ color: 'var(--atlas-text-dim)' }}>max relationships</span>
      </div>}
      {availableNodes !== undefined && availableEdges !== undefined && <button type="button" onClick={() => setPendingFull(true)} className="w-fit rounded border px-2 py-1 text-[10.5px]" style={{ borderColor: 'var(--atlas-warn)', color: 'var(--atlas-warn)' }}>Render full scoped graph</button>}
      {pendingFull && <div role="alertdialog" aria-label="Full graph performance warning" className="flex flex-col gap-1.5 rounded border p-2" style={{ borderColor: 'var(--atlas-warn)', background: 'oklch(0.22 0.02 75 / 0.15)' }}>
        <p className="text-[10.5px]" style={{ color: 'var(--atlas-text-bright)' }}>Rendering all {availableNodes} nodes and {availableEdges} relationships may slow this browser.</p>
        <div className="flex gap-2"><button type="button" onClick={confirmFull} className="rounded px-2 py-1 text-[10.5px] font-semibold" style={{ background: 'var(--atlas-warn)', color: 'var(--atlas-bg)' }}>Load full graph</button><button type="button" onClick={() => setPendingFull(false)} className="rounded px-2 py-1 text-[10.5px] font-semibold">Cancel</button></div>
      </div>}
      {pending !== null && (
        <div
          className="flex flex-col gap-1.5 rounded border p-2"
          style={{ borderColor: 'var(--atlas-warn)', background: 'oklch(0.22 0.02 75 / 0.15)' }}
        >
          <p className="text-[10.5px]" style={{ color: 'var(--atlas-text-bright)' }}>
            {pending} nodes may render slowly in this browser.
          </p>
          <div className="flex gap-2">
            <button
              type="button"
              onClick={confirmLarge}
              className="cursor-pointer rounded px-2 py-1 text-[10.5px] font-semibold"
              style={{ background: 'var(--atlas-warn)', color: 'var(--atlas-bg)' }}
            >
              Load anyway
            </button>
            <button
              type="button"
              onClick={() => {
                setPending(null)
                setDraft(value === undefined ? '' : String(value))
              }}
              className="cursor-pointer rounded px-2 py-1 text-[10.5px] font-semibold"
              style={{ background: 'transparent', color: 'var(--atlas-text-muted)' }}
            >
              Cancel
            </button>
          </div>
        </div>
      )}
    </div>
  )
}
