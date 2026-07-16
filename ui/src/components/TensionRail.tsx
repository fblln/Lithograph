import { useEffect, useState } from 'react'
import { impactAnalysis, tracePath, type TraceResult } from '../api/trace'
import { getRepositoryTensions, type RepositoryTension } from '../api/tensions'
import { ProvenanceTags } from './ProvenanceTags'

type HeatmapMode = 'severity' | 'category' | 'confidence' | 'blast' | 'coupling' | 'cycles' | 'boundary'
const EMPTY_SCOPE: string[] = []

export function TensionRail({ onFocus, onInspect = onFocus, onUseQuery = () => {}, requestedTensionId, onSelectTension = () => {}, scopeNodeIds = EMPTY_SCOPE, repositoryTensions }: { onFocus: (id: string) => void; onInspect?: (id: string) => void; onUseQuery?: (query: string) => void; requestedTensionId?: string; onSelectTension?: (tension: RepositoryTension) => void; scopeNodeIds?: string[]; repositoryTensions?: RepositoryTension[] }) {
  const [tensions, setTensions] = useState<RepositoryTension[]>([])
  const [selected, setSelected] = useState<RepositoryTension | null>(null)
  const [mode, setMode] = useState<HeatmapMode>('severity')
  const [trace, setTrace] = useState<TraceResult | null>(null)
  const [traceError, setTraceError] = useState<string | null>(null)
  const [collapsed, setCollapsed] = useState(false)

  useEffect(() => {
    if (repositoryTensions) { setTensions(repositoryTensions); return }
    getRepositoryTensions().then(setTensions, () => setTensions([]))
  }, [repositoryTensions])
  useEffect(() => {
    const requested = requestedTensionId ? tensions.find((tension) => tension.id === requestedTensionId) : undefined
    if (requested && selected?.id !== requested.id) {
      setSelected(requested)
      if (requested.affected_nodes[0]) onFocus(requested.affected_nodes[0])
    }
  }, [requestedTensionId, tensions, selected?.id, onFocus])
  const scope = new Set(scopeNodeIds)
  const visible = tensions.filter((tension) => {
    if (scope.size > 0 && !tension.affected_nodes.some((id) => scope.has(id))) return false
    if (mode === 'blast') return tension.category === 'BlastRadius'
    if (mode === 'coupling') return tension.category === 'CouplingHotspot'
    if (mode === 'cycles') return tension.category === 'DependencyCycle'
    if (mode === 'boundary') return tension.category === 'BoundaryViolation'
    return true
  }).sort((a, b) => severityRank(b.severity) - severityRank(a.severity) || b.affected_nodes.length - a.affected_nodes.length || a.id.localeCompare(b.id))
  const severeCount = visible.filter((tension) => severityRank(tension.severity) >= 3).length

  function select(tension: RepositoryTension) {
    setSelected(tension)
    onSelectTension(tension)
    setTrace(null)
    setTraceError(null)
    if (tension.affected_nodes[0]) onFocus(tension.affected_nodes[0])
  }

  async function investigate(kind: 'trace' | 'impact') {
    if (!selected?.affected_nodes[0]) return
    setTraceError(null)
    try { setTrace(await (kind === 'trace' ? tracePath(selected.affected_nodes[0]) : impactAnalysis(selected.affected_nodes[0]))) }
    catch (cause) { setTraceError(cause instanceof Error ? cause.message : String(cause)) }
  }

  return <aside data-visual-role="graph-overlay" className="absolute bottom-3 left-3 z-10 max-h-[42%] w-72 overflow-auto rounded border p-2" style={{ background: 'var(--atlas-surface)', borderColor: 'var(--atlas-border)' }}>
    <div className="flex items-center justify-between gap-2"><h2 className="text-[10px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Tension hotspots</h2><button type="button" aria-expanded={!collapsed} aria-label={collapsed ? 'Expand tension hotspots' : 'Collapse tension hotspots'} onClick={() => setCollapsed((value) => !value)} className="rounded px-1.5 py-0.5 text-[10px]">{collapsed ? 'Show' : 'Hide'}</button></div>
    {collapsed ? <p className="mt-1 text-[10px]" style={{ color: 'var(--atlas-text-muted)' }}>{visible.length} signals · {severeCount} high or critical</p> : <>
    <p className="mt-1 text-[10px]" style={{ color: 'var(--atlas-text-muted)' }}>{visible.length} deterministic signals · {severeCount} high or critical. Signals guide review; they are not automatic defects.</p>
    <label className="mt-1 block text-[10px]">Heatmap <select aria-label="Tension heatmap mode" value={mode} onChange={(event) => setMode(event.target.value as HeatmapMode)}><option value="severity">Severity</option><option value="category">Category</option><option value="confidence">Confidence</option><option value="blast">Blast radius</option><option value="coupling">Coupling</option><option value="cycles">Cycles</option><option value="boundary">Boundary pressure</option></select></label>
    {visible.length === 0 ? <p className="mt-1 text-[10px]" style={{ color: 'var(--atlas-text-muted)' }}>No matching graph tensions.</p> : <ul className="mt-1">{visible.slice(0, 4).map((tension) => <li key={tension.id}><button type="button" className="w-full py-1 text-left text-[10px]" onClick={() => select(tension)}><strong>{mode === 'severity' ? tension.severity : mode === 'category' ? tension.category : mode === 'confidence' ? tension.confidence : tension.category}</strong> · {tension.explanation}</button></li>)}</ul>}
    {selected && <TensionDetail tension={selected} trace={trace} traceError={traceError} onFocus={onFocus} onInspect={onInspect} onUseQuery={onUseQuery} onInvestigate={investigate} />}
    </>}
  </aside>
}

function severityRank(value: string): number { return ({ critical: 4, high: 3, medium: 2, low: 1 }[value.toLowerCase()] ?? 0) }

function TensionDetail({ tension, trace, traceError, onFocus, onInspect, onUseQuery, onInvestigate }: { tension: RepositoryTension; trace: TraceResult | null; traceError: string | null; onFocus: (id: string) => void; onInspect: (id: string) => void; onUseQuery: (query: string) => void; onInvestigate: (kind: 'trace' | 'impact') => void }) {
  const metrics = Object.entries(tension.metric_inputs ?? {})
  return <section className="mt-2 border-t pt-2 text-[10px]" style={{ borderColor: 'var(--atlas-border)' }}>
    <h3>Why this hotspot matters</h3><p>{tension.explanation}</p>
    <p>{tension.category} · {tension.severity} severity · {tension.confidence} confidence</p>
    <ProvenanceTags tags={tension.tags ?? []} label={`Tension provenance tags for ${tension.id}`} />
    <h4 className="mt-1 font-semibold">Affected nodes</h4><ul>{tension.affected_nodes.length ? tension.affected_nodes.map((id) => <li key={id}><button type="button" onClick={() => onInspect(id)}>{id}</button></li>) : <li>Cluster-level tension</li>}</ul>
    <h4 className="mt-1 font-semibold">Contributing metrics</h4><p>{metrics.length ? metrics.map(([name, value]) => `${name}=${value}`).join(', ') : 'No metric inputs recorded.'}</p>
    <h4 className="mt-1 font-semibold">Evidence</h4><ul>{tension.evidence_references.length ? tension.evidence_references.map((evidence) => <li key={evidence}>{evidence}</li>) : <li>No direct evidence reference recorded.</li>}</ul>
    <h4 className="mt-1 font-semibold">Next queries</h4><ul>{tension.follow_up_queries.length ? tension.follow_up_queries.map((query) => <li key={query}><button type="button" onClick={() => onUseQuery(query)}><code>{query}</code></button></li>) : <li>No follow-up query recorded.</li>}</ul>
    <div className="mt-2 flex gap-2"><button type="button" disabled={!tension.affected_nodes[0]} onClick={() => onInvestigate('trace')}>Trace dependency/call paths</button><button type="button" disabled={!tension.affected_nodes[0]} onClick={() => onInvestigate('impact')}>Analyze changed-file impact</button></div>
    {traceError && <p role="alert">{traceError}</p>}
    {trace && <section className="mt-2"><h4>Related trace</h4><p>{trace.visited.length - 1} affected nodes · {trace.relations.length} evidence relations</p><ul>{trace.visited.map(({ node, hop }) => <li key={node.id}>hop {hop}: <button type="button" onClick={() => onFocus(node.id)}>{node.name}</button></li>)}</ul></section>}
  </section>
}
