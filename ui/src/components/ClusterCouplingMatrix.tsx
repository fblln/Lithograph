import { useMemo } from 'react'
import type { ArchitectureCluster } from '../api/architecture'
import type { LayoutEdge } from '../graph/types'
import { computeClusterCoupling } from '../clusterCoupling'
import { humanClusterNameFromEvidence } from '../clusterIdentity'

export function ClusterCouplingMatrix({ clusters, edges, onInspect }: { clusters: ArchitectureCluster[]; edges: LayoutEdge[]; onInspect: (source: ArchitectureCluster, target: ArchitectureCluster) => void }) {
  const cells = useMemo(() => computeClusterCoupling(clusters, edges), [clusters, edges])
  const maximum = Math.max(...cells.map((cell) => cell.count), 1)
  if (clusters.length === 0) return <div role="status" className="absolute inset-0 grid place-items-center text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>No architecture clusters are available for this graph.</div>
  return <section aria-label="Cluster coupling matrix" className="absolute inset-0 overflow-auto p-20 pt-24">
    <div className="mx-auto w-fit rounded-lg border p-4" style={{ borderColor: 'var(--atlas-border-strong)', background: 'color-mix(in srgb, var(--atlas-canvas) 92%, transparent)' }}>
      <h2 className="text-sm font-semibold" style={{ color: 'var(--atlas-text-bright)' }}>Directed cluster coupling</h2>
      <p className="mb-4 text-[10.5px]" style={{ color: 'var(--atlas-text-muted)' }}>Rows are source clusters; columns are targets. Diagonal cells are internal edges. Select a cell to inspect it on the graph.</p>
      <div className="grid gap-1" style={{ gridTemplateColumns: `minmax(9rem, auto) repeat(${clusters.length}, minmax(3.5rem, 1fr))` }}>
        <span />
        {clusters.map((cluster) => <span key={`heading:${cluster.id}`} title={cluster.id} className="truncate px-1 text-center text-[9px] font-bold">{humanClusterNameFromEvidence(cluster)}</span>)}
        {clusters.flatMap((source) => [
          <span key={`row:${source.id}`} title={source.id} className="truncate self-center pr-2 text-right text-[9px] font-bold">{humanClusterNameFromEvidence(source)}</span>,
          ...cells.filter((cell) => cell.source.id === source.id).map((cell) => <button key={`${cell.source.id}:${cell.target.id}`} type="button" aria-label={`${humanClusterNameFromEvidence(cell.source)} to ${humanClusterNameFromEvidence(cell.target)}: ${cell.count} relationships`} onClick={() => onInspect(cell.source, cell.target)} className="aspect-square min-h-12 rounded border text-[11px] font-semibold" style={{ borderColor: cell.source.id === cell.target.id ? 'var(--atlas-border-strong)' : 'var(--atlas-accent)', background: `color-mix(in srgb, var(--atlas-accent) ${Math.round(8 + cell.count / maximum * 72)}%, var(--atlas-panel-header))`, color: 'var(--atlas-text-bright)' }}>{cell.count}</button>),
        ])}
      </div>
    </div>
  </section>
}
