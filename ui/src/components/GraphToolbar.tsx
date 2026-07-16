import { ViewModeToggle, type ViewMode } from './ViewModeToggle'

export type OverlayMode = 'kind' | 'centrality' | 'blast' | 'tension'
export type EdgeView = 'nodes' | 'clusters'

export function GraphToolbar({ viewMode, overlayMode, edgeView, zoom, layoutCustomized, truncated, edgeCountsTruncated = false, omittedNodes = 0, omittedEdges = 0, availableEdgeKinds = [], edgeKindCounts = new Map(), activeEdgeKinds = new Set(), showUnprovenEdges = true, onViewMode, onOverlayMode, onEdgeView, onToggleEdgeKind = () => {}, onShowUnprovenEdges = () => {}, onResetLayout, onZoom, onRaiseBudget }: { viewMode: ViewMode; overlayMode: OverlayMode; edgeView: EdgeView; zoom: number; layoutCustomized: boolean; truncated: boolean; edgeCountsTruncated?: boolean; omittedNodes?: number; omittedEdges?: number; availableEdgeKinds?: string[]; edgeKindCounts?: ReadonlyMap<string, number>; activeEdgeKinds?: Set<string>; showUnprovenEdges?: boolean; onViewMode: (mode: ViewMode) => void; onOverlayMode: (mode: OverlayMode) => void; onEdgeView: (mode: EdgeView) => void; onToggleEdgeKind?: (kind: string) => void; onShowUnprovenEdges?: (show: boolean) => void; onResetLayout: () => void; onZoom: (zoom: number) => void; onRaiseBudget: () => void }) {
  return <>
    <div aria-label="Graph controls" className="absolute top-3 left-3 z-10 flex max-w-[calc(100%-1.5rem)] flex-wrap items-center gap-2">
      <ViewModeToggle mode={viewMode} onChange={onViewMode} />
      <Segment label="Color" options={['kind', 'centrality', 'blast', 'tension']} value={overlayMode} onChange={(value) => onOverlayMode(value as OverlayMode)} />
      <Segment label="Edges" options={['nodes', 'clusters']} value={edgeView} onChange={(value) => onEdgeView(value as EdgeView)} />
      {availableEdgeKinds.length > 0 && <details className="relative rounded border px-2 py-1 text-[10px]" style={{ borderColor: 'var(--atlas-border-strong)', background: 'var(--atlas-panel-header)' }}><summary className="cursor-pointer">Relationship kinds · current slice{activeEdgeKinds.size ? ` (${activeEdgeKinds.size})` : ''}</summary><div className="absolute left-0 top-full z-20 mt-1 grid max-h-64 min-w-60 gap-1 overflow-auto rounded border p-2 shadow-xl" style={{ borderColor: 'var(--atlas-border-strong)', background: 'var(--atlas-surface)' }}><p className="mb-1 text-[9px]" style={{ color: 'var(--atlas-text-faint)' }}>Counts describe returned relationships in the current slice{edgeCountsTruncated ? '; additional relationships are truncated.' : '.'}</p>{availableEdgeKinds.map((kind) => <label key={kind} className="flex cursor-pointer items-center gap-2 whitespace-nowrap"><input aria-label={`${humanKind(kind)}: ${edgeKindCounts.get(kind) ?? 0} in current slice`} type="checkbox" checked={activeEdgeKinds.size === 0 || activeEdgeKinds.has(kind)} onChange={() => onToggleEdgeKind(kind)} /><span className="h-2 w-2 rounded-full" style={{ background: relationshipColor(kind) }} /><span className="flex-1">{humanKind(kind)}</span><span style={{ color: 'var(--atlas-text-dim)' }}>{edgeKindCounts.get(kind) ?? 0}</span></label>)}</div></details>}
      <label className="flex items-center gap-1 rounded border px-2 py-1 text-[10px]" style={{ borderColor: 'var(--atlas-border-strong)', background: 'var(--atlas-panel-header)' }}><input type="checkbox" checked={showUnprovenEdges} onChange={(event) => onShowUnprovenEdges(event.target.checked)} />Show syntax-only and fallback edges</label>
      {layoutCustomized && <button type="button" onClick={onResetLayout} className="rounded border px-2.5 py-1.5 text-[10.5px]" style={{ borderColor: 'var(--atlas-border-strong)', background: 'var(--atlas-panel-header)' }}>Reset layout</button>}
    </div>
    <div aria-label="Edge resolution legend" className="absolute top-14 left-3 z-10 flex gap-3 rounded border px-2 py-1 text-[9px]" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}><LegendLine label="Proven" opacity={0.9} /><LegendLine label="Syntax only" opacity={0.58} dashed /><LegendLine label="Fallback" opacity={0.34} dashed /></div>
    {truncated && <div role="status" className="absolute top-14 left-1/2 z-10 flex -translate-x-1/2 items-center gap-2 rounded border px-3 py-1.5 text-[10.5px]" style={{ borderColor: 'var(--atlas-warn)', background: 'var(--atlas-panel-header)', color: 'var(--atlas-warn)' }}>The current graph slice is truncated by its budget: {omittedNodes} nodes and {omittedEdges} relationships are not rendered.<button type="button" onClick={onRaiseBudget}>Reveal more</button></div>}
    <div aria-label="Camera controls" className="absolute right-3 bottom-3 z-10 flex flex-col gap-1"><button type="button" aria-label="Zoom in" onClick={() => onZoom(Math.max(0.35, zoom * 0.8))}>+</button><button type="button" aria-label="Zoom out" onClick={() => onZoom(Math.min(3, zoom * 1.25))}>−</button><button type="button" aria-label="Reset view" onClick={() => onZoom(1)}>✥</button></div>
  </>
}

function LegendLine({ label, opacity, dashed = false }: { label: string; opacity: number; dashed?: boolean }) {
  return <span className="flex items-center gap-1"><span aria-hidden="true" className="w-5" style={{ borderTop: `2px ${dashed ? 'dashed' : 'solid'} currentColor`, opacity }} />{label}</span>
}

function humanKind(value: string): string { return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[-_]+/g, ' ') }
function relationshipColor(kind: string): string {
  const normalized = kind.toLowerCase()
  if (normalized.includes('call') || normalized.includes('run')) return '#f1a65a'
  if (normalized.includes('import') || normalized.includes('depend') || normalized.includes('use')) return '#6ea8fe'
  if (normalized.includes('data') || normalized.includes('read') || normalized.includes('write')) return '#64d8b1'
  if (normalized.includes('contain') || normalized.includes('member') || normalized.includes('belong') || normalized.includes('has')) return '#a998dc'
  if (normalized.includes('config') || normalized.includes('env') || normalized.includes('bind')) return '#e3cf68'
  return '#788195'
}

function Segment({ label, options, value, onChange }: { label: string; options: string[]; value: string; onChange: (value: string) => void }) {
  return <div className="flex items-center gap-1"><span className="text-[9px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>{label === 'Color' ? 'Lens' : label}</span><div className="flex rounded p-0.5" style={{ background: 'var(--atlas-panel-header)', border: '1px solid var(--atlas-border-strong)' }}>{options.map((option) => <button key={option} type="button" data-active={value === option} onClick={() => onChange(option)} className="rounded px-2 py-1 text-[10px] capitalize" style={{ background: value === option ? 'var(--atlas-accent)' : 'transparent', color: value === option ? 'var(--atlas-canvas)' : 'var(--atlas-text-muted)' }}>{option === 'kind' ? 'Architecture' : option === 'tension' ? 'Tensions' : option === 'blast' ? 'Impact' : option}</button>)}</div></div>
}
