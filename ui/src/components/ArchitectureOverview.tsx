import { useMemo } from 'react'
import type { ArchitectureNodeSummary, ArchitectureSummary } from '../api/architecture'
import type { RepositoryTension } from '../api/tensions'
import type { LayoutResult } from '../graph/types'
import { deriveRepositoryAreas, type RepositoryArea } from '../architectureOverview'

export function ArchitectureOverview({ layout, architecture, tensions, scopedNodeIds, onScopeArea, onClearScope, onFocus, onSelectTension, onOpenFiles }: { layout: LayoutResult; architecture: ArchitectureSummary; tensions: RepositoryTension[]; scopedNodeIds: string[]; onScopeArea: (area: RepositoryArea) => void; onClearScope: () => void; onFocus: (id: string) => void; onSelectTension: (tension: RepositoryTension) => void; onOpenFiles: () => void }) {
  const activeScope = new Set(scopedNodeIds)
  const visibleEntryPoints = activeScope.size ? architecture.entry_points.filter((entry) => activeScope.has(entry.id)) : architecture.entry_points
  const visibleHotspots = activeScope.size ? architecture.hotspots.filter((node) => activeScope.has(node.id)) : architecture.hotspots
  const visibleTensions = activeScope.size ? tensions.filter((tension) => tension.affected_nodes.some((id) => activeScope.has(id))) : tensions
  const areas = useMemo(() => deriveRepositoryAreas(layout, visibleEntryPoints, visibleTensions), [layout, visibleEntryPoints, visibleTensions])
  const scopedArea = areas.find((area) => area.nodeIds.length === activeScope.size && area.nodeIds.every((id) => activeScope.has(id)))
  const crossAreaEdges = areas.reduce((total, area) => total + area.outgoing, 0)
  const attention = visibleTensions.slice().sort((a, b) => severityRank(b.severity) - severityRank(a.severity) || a.id.localeCompare(b.id)).slice(0, 3)

  return <section aria-label="Architecture overview" className="p-3 text-[11px]">
    <div className="mb-3">
      <p className="text-[9px] font-bold uppercase tracking-[0.14em]" style={{ color: 'var(--atlas-accent)' }}>Start here</p>
      <h2 className="mt-1 text-[14px] font-semibold" style={{ color: 'var(--atlas-text-bright)' }}>How this application is organized</h2>
      <p className="mt-1 leading-relaxed" style={{ color: 'var(--atlas-text-muted)' }}>Open an area to simplify the graph, then select a node for source evidence. The path above the graph always takes you back.</p>
    </div>
    <div className="mb-4 grid grid-cols-2 gap-1.5" aria-label="Repository summary">
      <Summary value={areas.length} label="major areas" />
      <Summary value={visibleEntryPoints.length} label="entry points" />
      <Summary value={crossAreaEdges} label="cross-area links" />
      <Summary value={visibleTensions.length} label="attention signals" tone={visibleTensions.length ? 'warn' : undefined} />
    </div>

    <Section title="Major areas" hint="Directories and subsystems in this graph slice">
      {scopedArea && <button type="button" onClick={onClearScope} className="mb-2 w-full rounded border px-2 py-1.5 text-left" style={{ borderColor: 'var(--atlas-accent)', color: 'var(--atlas-accent)' }}>← Show the whole application</button>}
      <div className="space-y-1.5">{areas.slice(0, 8).map((area) => <button key={area.id} type="button" aria-label={`Open ${area.name} area`} onClick={() => onScopeArea(area)} className="w-full rounded border p-2 text-left" style={{ borderColor: scopedArea?.id === area.id ? 'var(--atlas-accent)' : 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}>
        <span className="flex items-start justify-between gap-2"><strong className="truncate" style={{ color: 'var(--atlas-text-bright)' }}>{area.name}</strong><span className="whitespace-nowrap" style={{ color: 'var(--atlas-text-faint)' }}>{area.nodeCount} nodes</span></span>
        <span className="mt-0.5 block truncate" title={area.id} style={{ color: 'var(--atlas-text-dim)' }}>{area.id === 'root' ? 'top-level files' : area.id} · {area.fileCount} files</span>
        <span className="mt-1 block" style={{ color: 'var(--atlas-text-muted)' }}>{connectionSummary(area)}{area.entryPoints.length ? ` · ${area.entryPoints.length} entry point${area.entryPoints.length === 1 ? '' : 's'}` : ''}{area.tensionCount ? ` · ${area.tensionCount} attention` : ''}</span>
      </button>)}</div>
      {areas.length > 8 && <p className="mt-2" style={{ color: 'var(--atlas-text-faint)' }}>{areas.length - 8} smaller areas are available in Files.</p>}
      <button type="button" onClick={onOpenFiles} className="mt-2" style={{ color: 'var(--atlas-accent)' }}>Browse files and directories →</button>
    </Section>

    <Section title="Entry points" hint="Good places to begin following execution">
      {visibleEntryPoints.length ? <ItemList items={visibleEntryPoints.slice(0, 6)} onFocus={onFocus} describe={(entry) => `${humanKind(entry.label)}${entry.file_path ? ` · ${entry.file_path}` : ''}`} /> : <Empty>No explicit commands or containers were detected in this slice.</Empty>}
    </Section>

    <Section title="Important connections" hint="Highly connected code that shapes the application">
      {visibleHotspots.length ? <ItemList items={visibleHotspots.slice(0, 5)} onFocus={onFocus} describe={(node) => `${humanKind(node.label)} · ${node.in_degree} in / ${node.out_degree} out`} /> : <Empty>No high-connectivity nodes were detected.</Empty>}
    </Section>

    <Section title="Needs attention" hint="Evidence-backed signals, not automatic defects">
      {attention.length ? <div className="space-y-1.5">{attention.map((tension) => <button key={tension.id} type="button" onClick={() => onSelectTension(tension)} className="w-full rounded border p-2 text-left" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}><strong style={{ color: 'var(--atlas-warn)' }}>{humanKind(tension.category)} · {tension.severity}</strong><span className="mt-0.5 block" style={{ color: 'var(--atlas-text-muted)' }}>{tension.affected_nodes.length} affected node{tension.affected_nodes.length === 1 ? '' : 's'} · inspect supporting evidence</span></button>)}</div> : <Empty>No repository tensions match the current scope.</Empty>}
    </Section>
  </section>
}

function humanKind(value: string): string {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[-_]+/g, ' ').replace(/^\w/, (letter) => letter.toUpperCase())
}

function connectionSummary(area: RepositoryArea): string {
  if (area.connectedAreas.length === 0) return 'self-contained in this slice'
  const targets = area.connectedAreas.slice(0, 2).join(', ')
  return `${area.incoming} in / ${area.outgoing} out · connects to ${targets}${area.connectedAreas.length > 2 ? ` +${area.connectedAreas.length - 2}` : ''}`
}

function severityRank(value: string): number { return ({ critical: 4, high: 3, medium: 2, low: 1 }[value.toLowerCase()] ?? 0) }

function Summary({ value, label, tone }: { value: number; label: string; tone?: 'warn' }) { return <div className="rounded border p-2" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}><strong className="text-[16px]" style={{ color: tone === 'warn' ? 'var(--atlas-warn)' : 'var(--atlas-text-bright)' }}>{value}</strong><span className="ml-1" style={{ color: 'var(--atlas-text-dim)' }}>{label}</span></div> }
function Section({ title, hint, children }: { title: string; hint: string; children: React.ReactNode }) { return <section className="mb-4"><h3 className="text-[10px] font-bold uppercase tracking-wide" style={{ color: 'var(--atlas-text-dim)' }}>{title}</h3><p className="mb-2 mt-0.5" style={{ color: 'var(--atlas-text-faint)' }}>{hint}</p>{children}</section> }
function Empty({ children }: { children: React.ReactNode }) { return <p role="status" className="rounded border p-2" style={{ borderColor: 'var(--atlas-border)', color: 'var(--atlas-text-muted)' }}>{children}</p> }
function ItemList({ items, onFocus, describe }: { items: ArchitectureNodeSummary[]; onFocus: (id: string) => void; describe: (item: ArchitectureNodeSummary) => string }) { return <ul className="space-y-1">{items.map((item) => <li key={item.id}><button type="button" onClick={() => onFocus(item.id)} className="w-full rounded px-2 py-1.5 text-left hover:bg-[var(--atlas-hover)]"><strong className="block truncate" title={item.name} style={{ color: 'var(--atlas-text-bright)' }}>{item.name}</strong><span className="block truncate" style={{ color: 'var(--atlas-text-dim)' }}>{describe(item)}</span></button></li>)}</ul> }
