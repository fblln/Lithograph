import { useEffect, useState } from 'react'
import type { LayoutResult } from '../graph/types'
import { colorForLabel } from '../graph/palette'
import { StatsPanel } from './StatsPanel'
import { BudgetControl } from './BudgetControl'
import { QueryWorkbench } from './QueryWorkbench'
import { AnalyticsPanel } from './AnalyticsPanel'
import { SavedInvestigations } from './SavedInvestigations'
import { FileModuleTree } from './FileModuleTree'
import { TagExplorer } from './TagExplorer'
import { ClusterExplorer } from './ClusterExplorer'
import type { ArchitectureCluster, ArchitectureSummary } from '../api/architecture'
import type { InvestigationQueryState, SavedInvestigation } from '../investigations'
import type { RepositoryTension } from '../api/tensions'
import { ArchitectureOverview } from './ArchitectureOverview'
import type { RepositoryArea } from '../architectureOverview'

export interface SidebarProps {
  layout: LayoutResult
  activeLabels: Set<string>
  onToggleLabel: (label: string) => void
  maxNodes: number | undefined
  onApplyMaxNodes: (maxNodes: number | undefined) => void
  maxEdges?: number
  onApplyMaxEdges?: (maxEdges: number | undefined) => void
  onFocusNode?: (id: string) => void
  onMetricValues?: (values: Map<string, number>) => void
  onSemanticLabels?: (labels: string[]) => void
  queryState?: InvestigationQueryState
  onQueryStateChange?: (state: InvestigationQueryState) => void
  investigationState?: Omit<SavedInvestigation, 'id' | 'name' | 'notes'>
  onRestoreInvestigation?: (value: SavedInvestigation) => void
  requestedTab?: SidebarTab
  tagExpression?: string
  onTagExpressionChange?: (expression: string, nodeIds: string[]) => void
  clusters?: ArchitectureCluster[]
  scopedClusterId?: string
  interClusterOnly?: boolean
  onClusterScope?: (cluster?: ArchitectureCluster) => void
  onInterClusterOnly?: (enabled: boolean) => void
  onRelatedEntity?: (id: string) => void
  scopeNodeIds?: string[]
  architecture?: ArchitectureSummary
  tensions?: RepositoryTension[]
  onAreaScope?: (area: RepositoryArea) => void
  onClearArchitectureScope?: () => void
  onSelectTension?: (tension: RepositoryTension) => void
}

export type SidebarTab = 'overview' | 'files' | 'clusters' | 'tags' | 'filters' | 'stats' | 'query' | 'analytics' | 'saved'

const TABS: Array<{ id: SidebarTab; label: string }> = [
  { id: 'overview', label: 'Overview' },
  { id: 'files', label: 'Files' },
  { id: 'clusters', label: 'Clusters' },
  { id: 'tags', label: 'Tags' },
  { id: 'filters', label: 'Filters' },
  { id: 'stats', label: 'Stats' },
  { id: 'query', label: 'Query' },
  { id: 'analytics', label: 'Analytics' },
  { id: 'saved', label: 'Saved' },
]

export function Sidebar({
  layout,
  activeLabels,
  onToggleLabel,
  maxNodes,
  onApplyMaxNodes,
  maxEdges,
  onApplyMaxEdges = () => {},
  onFocusNode = () => {},
  onMetricValues = () => {},
  onSemanticLabels = () => {},
  queryState,
  onQueryStateChange = () => {},
  investigationState,
  onRestoreInvestigation = () => {},
  requestedTab,
  tagExpression = '',
  onTagExpressionChange = () => {},
  clusters = [],
  scopedClusterId,
  interClusterOnly = false,
  onClusterScope = () => {},
  onInterClusterOnly = () => {},
  onRelatedEntity = () => {},
  scopeNodeIds,
  architecture = { clusters: [], entry_points: [], hotspots: [] },
  tensions = [],
  onAreaScope = () => {},
  onClearArchitectureScope = () => {},
  onSelectTension = () => {},
}: SidebarProps) {
  const [tab, setTab] = useState<SidebarTab>('overview')
  useEffect(() => { if (requestedTab) setTab(requestedTab) }, [requestedTab])

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
      <section className="border-b px-3 py-2.5" style={{ borderColor: 'var(--atlas-border)' }}>
        <div className="grid grid-cols-3 gap-1 text-center"><SummaryCell value={layout.budget.nodes_available} label="nodes" /><SummaryCell value={layout.budget.edges_available} label="edges" /><SummaryCell value={layout.budget.nodes_returned} label="shown" tone="accent" /></div>
        <div className="mt-2 h-1.5 overflow-hidden rounded" style={{ background: 'var(--atlas-canvas)' }}><div className="h-full" style={{ width: `${Math.min(100, layout.budget.nodes_available ? (layout.budget.nodes_returned / layout.budget.nodes_available) * 100 : 0)}%`, background: 'var(--atlas-accent)' }} /></div>
        <p className="mt-1 text-[9.5px]" style={{ color: 'var(--atlas-text-faint)' }}>bounded graph slice{layout.budget.nodes_truncated ? ' · truncated' : ''}</p>
      </section>

      <div className="flex overflow-x-auto border-b" style={{ borderColor: 'var(--atlas-border)' }}>
        {TABS.map((option) => {
          const active = option.id === tab
          return (
            <button
              key={option.id}
              type="button"
              data-active={active}
              onClick={() => setTab(option.id)}
              className="min-w-14 flex-none border-0 border-b-2 px-2 py-2.5 text-[10.5px] font-semibold tracking-wide uppercase"
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

      {tab === 'overview' ? <ArchitectureOverview layout={layout} architecture={architecture} tensions={tensions} scopedNodeIds={scopeNodeIds ?? []} onScopeArea={onAreaScope} onClearScope={onClearArchitectureScope} onFocus={onFocusNode} onSelectTension={onSelectTension} onOpenFiles={() => setTab('files')} /> : tab === 'files' ? <FileModuleTree nodes={layout.nodes} onFocusNode={onFocusNode} /> : tab === 'clusters' ? <ClusterExplorer layout={layout} clusters={clusters} entryPoints={architecture.entry_points} tensions={tensions} scopedClusterId={scopedClusterId} interClusterOnly={interClusterOnly} onScope={onClusterScope} onInterClusterOnly={onInterClusterOnly} onFocus={onFocusNode} onRelatedEntity={onRelatedEntity} /> : tab === 'tags' ? <TagExplorer expression={tagExpression} onChange={onTagExpressionChange} /> : tab === 'saved' && investigationState ? <SavedInvestigations current={investigationState} onRestore={onRestoreInvestigation} /> : tab === 'analytics' ? <AnalyticsPanel onFocusNode={onFocusNode} onMetricValues={onMetricValues} onSemanticLabels={onSemanticLabels} /> : tab === 'query' ? <QueryWorkbench onFocusNode={onFocusNode} state={queryState} onStateChange={onQueryStateChange} scopeNodeIds={scopeNodeIds} /> : tab === 'filters' ? (
        <section className="p-3">
          <h2 className="mb-2 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Graph budget</h2>
          <BudgetControl value={maxNodes} onApply={onApplyMaxNodes} edgeValue={maxEdges} onApplyEdges={onApplyMaxEdges} availableNodes={layout.budget.nodes_available} availableEdges={layout.budget.edges_available} />
          <h2
            className="mb-2 mt-4 text-[9.5px] font-bold tracking-wide uppercase"
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

function SummaryCell({ value, label, tone }: { value: number; label: string; tone?: 'accent' }) {
  return <div><div className="text-[14px] font-bold leading-none" style={{ color: tone === 'accent' ? 'var(--atlas-accent)' : 'var(--atlas-text-bright)' }}>{value}</div><div className="mt-0.5 text-[8.5px] font-semibold tracking-wide uppercase" style={{ color: 'var(--atlas-text-faint)' }}>{label}</div></div>
}
