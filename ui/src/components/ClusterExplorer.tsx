import { useMemo, useState, type Dispatch, type SetStateAction } from 'react'
import type { ArchitectureCluster, ArchitectureNodeSummary } from '../api/architecture'
import type { LayoutResult } from '../graph/types'
import type { RepositoryTension } from '../api/tensions'
import { deriveClusterInsights, type ClusterInsight } from '../clusterInsights'
import { computeClusterLayout } from '../graph/clusterLayout'
import { deriveClusterIdentities } from '../clusterIdentity'
import { ProvenanceTags } from './ProvenanceTags'

export function ClusterExplorer({ layout, clusters, entryPoints = [], tensions = [], scopedClusterId, interClusterOnly, onScope, onInterClusterOnly, onFocus, onRelatedEntity }: { layout: LayoutResult; clusters: ArchitectureCluster[]; entryPoints?: ArchitectureNodeSummary[]; tensions?: RepositoryTension[]; scopedClusterId?: string; interClusterOnly: boolean; onScope: (cluster?: ArchitectureCluster) => void; onInterClusterOnly: (enabled: boolean) => void; onFocus: (id: string) => void; onRelatedEntity: (id: string) => void }) {
  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [pinned, setPinned] = useState<Set<string>>(new Set())
  const [compared, setCompared] = useState<string[]>([])
  const insights = useMemo(() => deriveClusterInsights(layout, clusters, tensions), [clusters, layout, tensions])
  const visualLayout = useMemo(() => computeClusterLayout(layout.nodes, clusters, layout.edges), [clusters, layout.edges, layout.nodes])
  const identities = useMemo(() => deriveClusterIdentities(visualLayout.clusters, layout.nodes, visualLayout.links, entryPoints, tensions), [entryPoints, layout.nodes, tensions, visualLayout.clusters, visualLayout.links])
  const ordered = [...insights].sort((a, b) => Number(pinned.has(b.cluster.id)) - Number(pinned.has(a.cluster.id)) || b.cluster.members.length - a.cluster.members.length || a.cluster.id.localeCompare(b.cluster.id))

  function toggleInSet(setter: Dispatch<SetStateAction<Set<string>>>, id: string) {
    setter((previous) => { const next = new Set(previous); if (next.has(id)) next.delete(id); else next.add(id); return next })
  }

  function toggleCompare(id: string) {
    setCompared((previous) => previous.includes(id) ? previous.filter((item) => item !== id) : [...previous.slice(-1), id])
  }

  if (clusters.length === 0) return <section className="p-3 text-[11px]" role="status">No architecture clusters were detected.</section>
  const comparedInsights = compared.map((id) => insights.find((item) => item.cluster.id === id)).filter((item): item is ClusterInsight => item !== undefined)

  return <section aria-label="Cluster map" className="p-2">
    <div className="mb-2 flex items-center gap-2"><h2 className="flex-1 text-[9.5px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Cluster map</h2><button type="button" aria-pressed={interClusterOnly} onClick={() => onInterClusterOnly(!interClusterOnly)} className="rounded px-1.5 py-1 text-[9px]" style={{ background: interClusterOnly ? 'var(--atlas-accent)' : 'var(--atlas-chip)' }}>Boundary edges</button></div>
    {scopedClusterId && <button type="button" onClick={() => onScope(undefined)} className="mb-2 text-[10px]" style={{ color: 'var(--atlas-accent)' }}>Clear cluster isolation</button>}
    {comparedInsights.length === 2 && <div aria-label="Cluster comparison" className="mb-2 rounded border p-2 text-[9.5px]" style={{ borderColor: 'var(--atlas-accent)' }}><strong>{identities.get(comparedInsights[0].cluster.id)?.name ?? comparedInsights[0].cluster.id}</strong> vs <strong>{identities.get(comparedInsights[1].cluster.id)?.name ?? comparedInsights[1].cluster.id}</strong><p>{comparedInsights[0].cluster.members.length} / {comparedInsights[1].cluster.members.length} members · cohesion {comparedInsights[0].cluster.cohesion.toFixed(2)} / {comparedInsights[1].cluster.cohesion.toFixed(2)}</p></div>}
    <div className="space-y-2">{ordered.map((insight) => {
      const { cluster } = insight
      const identity = identities.get(cluster.id)
      const isExpanded = expanded.has(cluster.id)
      const hiddenMembers = cluster.members.length - insight.visibleMembers.length
      return <article key={cluster.id} data-scoped={scopedClusterId === cluster.id} className="rounded border p-2 text-[10px]" style={{ borderColor: scopedClusterId === cluster.id ? 'var(--atlas-accent)' : 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}>
        <div className="flex items-center gap-1"><button type="button" aria-label={`${isExpanded ? 'Collapse' : 'Expand'} ${identity?.name ?? cluster.id}`} aria-expanded={isExpanded} onClick={() => toggleInSet(setExpanded, cluster.id)} className="w-4">{isExpanded ? '−' : '+'}</button><button type="button" className="min-w-0 flex-1 truncate text-left font-semibold" title={cluster.id} onClick={() => { onScope(cluster); onRelatedEntity(cluster.id) }}>{identity?.name ?? cluster.id}</button><button type="button" aria-label={`${pinned.has(cluster.id) ? 'Unpin' : 'Pin'} ${identity?.name ?? cluster.id}`} aria-pressed={pinned.has(cluster.id)} onClick={() => toggleInSet(setPinned, cluster.id)}>⌖</button><button type="button" aria-label={`Compare ${identity?.name ?? cluster.id}`} aria-pressed={compared.includes(cluster.id)} onClick={() => toggleCompare(cluster.id)}>⇄</button></div>
        {identity && <p className="mt-1" style={{ color: 'var(--atlas-text-muted)' }}>{identity.responsibility}</p>}
        <p style={{ color: 'var(--atlas-text-dim)' }}>{cluster.members.length} members · {identity?.fileCount ?? 0} files · cohesion {cluster.cohesion.toFixed(2)} · conductance {insight.conductance.toFixed(2)}</p>
        <p>{cluster.packages.slice(0, 3).join(', ') || 'no package'} · {identity?.dominantKinds.join(', ') || insight.dominantKinds.join(', ') || 'no dominant kind'}</p>
        <p>{identity?.entryPoints.length ?? 0} entry points · {identity?.incoming.length ?? 0} incoming / {identity?.outgoing.length ?? 0} outgoing dependencies</p>
        <p>{insight.bridgeNodes.length} bridge nodes · {insight.boundaryEdges} boundary edges · {insight.tensions.length} tensions{identity?.highestSeverity ? ` · highest ${identity.highestSeverity}` : ''}</p>
        {identity && <p style={{ color: 'var(--atlas-text-dim)' }}>{identity.boundaryInterpretation}</p>}
        <ProvenanceTags tags={cluster.tags ?? []} label={`Cluster provenance tags for ${cluster.id}`} />
        {isExpanded && <div className="mt-2 border-t pt-2" style={{ borderColor: 'var(--atlas-border)' }}>
          <p className="mb-1 font-semibold">Bridge nodes</p>
          <div className="flex flex-wrap gap-1">{insight.bridgeNodes.slice(0, 8).map((id) => <button key={id} type="button" onClick={() => onFocus(id)} title={id} className="max-w-full truncate rounded px-1.5 py-0.5" style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-accent)' }}>{shortName(id)}</button>)}</div>
          <p className="mb-1 mt-2 font-semibold">Visible members</p>
          <div className="flex flex-wrap gap-1">{insight.visibleMembers.slice(0, 12).map((id) => <button key={id} type="button" onClick={() => onFocus(id)} title={id} className="max-w-full truncate rounded px-1.5 py-0.5" style={{ background: 'var(--atlas-chip)' }}>{shortName(id)}</button>)}</div>
          {hiddenMembers > 0 && <><p role="status" className="mt-1" style={{ color: 'var(--atlas-warn)' }}>{hiddenMembers} members are outside the current graph budget.</p><button type="button" onClick={() => onScope(cluster)} className="mt-1" style={{ color: 'var(--atlas-accent)' }}>Show all nodes in this cluster →</button></>}
          <details className="mt-2"><summary className="cursor-pointer" style={{ color: 'var(--atlas-text-faint)' }}>Technical details</summary><code className="mt-1 block break-all">{cluster.id}</code></details>
        </div>}
      </article>
    })}</div>
  </section>
}

function shortName(id: string): string { return id.split(/[/:#]/).filter(Boolean).at(-1) ?? id }
