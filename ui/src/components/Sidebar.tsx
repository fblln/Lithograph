import { useState } from 'react'
import type { LayoutResult } from '../graph/types'
import { colorForLabel } from '../graph/palette'
import { StatsPanel } from './StatsPanel'
import { BudgetControl } from './BudgetControl'

export interface SidebarProps {
  layout: LayoutResult
  activeLabels: Set<string>
  onToggleLabel: (label: string) => void
  maxNodes: number | undefined
  onApplyMaxNodes: (maxNodes: number | undefined) => void
}

type Tab = 'filters' | 'stats'

const TABS: Array<{ id: Tab; label: string }> = [
  { id: 'filters', label: 'Filters' },
  { id: 'stats', label: 'Stats' },
]

export function Sidebar({
  layout,
  activeLabels,
  onToggleLabel,
  maxNodes,
  onApplyMaxNodes,
}: SidebarProps) {
  const [tab, setTab] = useState<Tab>('filters')

  const counts = new Map<string, number>()
  for (const node of layout.nodes) {
    counts.set(node.label, (counts.get(node.label) ?? 0) + 1)
  }
  const labels = [...counts.keys()].sort((a, b) => a.localeCompare(b))

  return (
    <aside
      className="flex w-64 flex-none flex-col overflow-y-auto border-r"
      style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-surface)' }}
    >
      <section className="border-b p-3" style={{ borderColor: 'var(--atlas-border)' }}>
        <h2
          className="mb-2 text-[9.5px] font-bold tracking-wide uppercase"
          style={{ color: 'var(--atlas-text-dim)' }}
        >
          Graph budget
        </h2>
        <p className="text-[10.5px]" style={{ color: 'var(--atlas-text-muted)' }}>
          rendering {layout.budget.nodes_returned} of {layout.budget.nodes_available} nodes
          {layout.budget.nodes_truncated ? ' (truncated)' : ''}
        </p>
        <p className="mb-2 text-[10.5px]" style={{ color: 'var(--atlas-text-muted)' }}>
          rendering {layout.budget.edges_returned} of {layout.budget.edges_available} edges
          {layout.budget.edges_truncated ? ' (truncated)' : ''}
        </p>
        <BudgetControl value={maxNodes} onApply={onApplyMaxNodes} />
      </section>

      <div className="flex border-b" style={{ borderColor: 'var(--atlas-border)' }}>
        {TABS.map((option) => {
          const active = option.id === tab
          return (
            <button
              key={option.id}
              type="button"
              data-active={active}
              onClick={() => setTab(option.id)}
              className="flex-1 border-0 border-b-2 py-2.5 text-[10.5px] font-semibold tracking-wide uppercase"
              style={{
                background: 'transparent',
                cursor: 'pointer',
                color: active ? 'var(--atlas-text-bright)' : 'var(--atlas-text-dim)',
                borderBottomColor: active ? 'var(--atlas-accent)' : 'transparent',
              }}
            >
              {option.label}
            </button>
          )
        })}
      </div>

      {tab === 'filters' ? (
        <section className="p-3">
          <h2
            className="mb-2 text-[9.5px] font-bold tracking-wide uppercase"
            style={{ color: 'var(--atlas-text-dim)' }}
          >
            Node kinds
          </h2>
          <ul className="flex flex-col gap-0.5">
            {labels.map((label) => {
              const active = activeLabels.size === 0 || activeLabels.has(label)
              return (
                <li key={label}>
                  <button
                    type="button"
                    onClick={() => onToggleLabel(label)}
                    className="flex w-full cursor-pointer items-center gap-2 rounded px-0.5 py-1 text-left"
                    style={{ opacity: active ? 1 : 0.4 }}
                  >
                    <span
                      className="h-2 w-2 flex-none rounded-full"
                      style={{ background: colorForLabel(label) }}
                    />
                    <span className="flex-1 text-[11.5px]" style={{ color: 'var(--atlas-text-bright)' }}>
                      {label}
                    </span>
                    <span className="text-[10px]" style={{ color: 'var(--atlas-text-dim)' }}>
                      {counts.get(label)}
                    </span>
                  </button>
                </li>
              )
            })}
          </ul>
        </section>
      ) : (
        <StatsPanel layout={layout} />
      )}
    </aside>
  )
}
