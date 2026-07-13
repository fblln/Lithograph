import { useState } from 'react'

export interface BudgetControlProps {
  /** Currently applied max_nodes, or `undefined` for the server default. */
  value: number | undefined
  onApply: (maxNodes: number | undefined) => void
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

export function BudgetControl({ value, onApply }: BudgetControlProps) {
  const [draft, setDraft] = useState(value === undefined ? '' : String(value))
  const [pending, setPending] = useState<number | null>(null)

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
