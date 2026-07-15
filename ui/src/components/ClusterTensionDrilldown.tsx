import { useEffect, useState } from 'react'
import type { ArchitectureCluster } from '../api/architecture'
import { getRepositoryTensions, type RepositoryTension } from '../api/tensions'
import { humanClusterNameFromEvidence } from '../clusterIdentity'

export function ClusterTensionDrilldown({ clusters, onFocus }: { clusters: ArchitectureCluster[]; onFocus: (id: string) => void }) {
  const [tensions, setTensions] = useState<RepositoryTension[]>([])
  useEffect(() => { getRepositoryTensions().then(setTensions, () => setTensions([])) }, [])
  const tense = clusters.map((cluster) => ({ cluster, tensions: tensions.filter((tension) => tension.affected_nodes.some((id) => cluster.members.includes(id))) })).filter((item) => item.tensions.length > 0).slice(0, 3)
  if (tense.length === 0) return null
  return <aside className="absolute right-3 bottom-3 z-10 w-60 rounded border p-2" style={{ background: 'var(--atlas-surface)', borderColor: 'var(--atlas-border)' }}><h2 className="text-[10px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Tense clusters</h2>{tense.map(({ cluster, tensions: matches }) => <section key={cluster.id} className="mt-2 border-t pt-1 text-[10px]" style={{ borderColor: 'var(--atlas-border)' }}><button type="button" className="font-semibold" title={cluster.id} onClick={() => onFocus(matches[0].affected_nodes[0] ?? cluster.members[0])}>{humanClusterNameFromEvidence(cluster)}</button><p>{matches.length} signal{matches.length === 1 ? '' : 's'} · highest {highestSeverity(matches)} · cohesion {cluster.cohesion.toFixed(2)}</p><p>Boundary pressure: in {cluster.incoming_pressure} / out {cluster.outgoing_pressure}</p><p>Relationships: {cluster.edge_types.join(', ') || 'none'}</p><p>Representative nodes: {(cluster.top_nodes as Array<{ name: string }>).map((node) => node.name).join(', ')}</p></section>)}</aside>
}

function highestSeverity(tensions: RepositoryTension[]): string {
  return [...tensions].sort((a, b) => severityRank(b.severity) - severityRank(a.severity))[0]?.severity ?? 'unknown'
}
function severityRank(value: string): number { return ({ critical: 4, high: 3, medium: 2, low: 1 }[value.toLowerCase()] ?? 0) }
