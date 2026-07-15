import { useState } from 'react'
import { impactAnalysis, tracePath, type TraceResult } from '../api/trace'
import { getClusters, type ArchitectureCluster } from '../api/architecture'

const EMPTY_SCOPE: string[] = []

export function TraceExplorer({ onFocusNode, scopeNodeIds = EMPTY_SCOPE }: { onFocusNode: (id: string) => void; scopeNodeIds?: string[] }) {
  const [query, setQuery] = useState('')
  const [result, setResult] = useState<TraceResult | null>(null)
  const [clusters, setClusters] = useState<ArchitectureCluster[]>([])
  const [error, setError] = useState<string | null>(null)
  async function run(kind: 'trace' | 'impact') { setError(null); try { const [trace, discovered] = await Promise.all([kind === 'trace' ? tracePath(query) : impactAnalysis(query), getClusters().catch(() => [])]); setResult(trace); setClusters(discovered) } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) } }
  const visibleVisited = result?.visited.filter((item) => scopeNodeIds.length === 0 || scopeNodeIds.includes(item.node.id)) ?? []
  const affected = visibleVisited.map((item) => item.node.id)
  const affectedClusters = clusters.filter((cluster) => cluster.members.some((member) => affected.includes(member)))
  const risk = affected.length > 20 || affectedClusters.length > 2 ? 'high' : affected.length > 5 || affectedClusters.length > 1 ? 'medium' : 'low'
  return <section className="p-3 text-[11px]"><h2>Trace & impact</h2>{scopeNodeIds.length > 0 && <p aria-label="Trace scope" style={{ color: 'var(--atlas-accent)' }}>Scoped to {scopeNodeIds.length} nodes</p>}<input aria-label="Trace node" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Node id, name, or changed file" /><div><button type="button" onClick={() => run('trace')}>Trace path</button><button type="button" onClick={() => run('impact')}>Analyze impact</button></div>{error && <p role="alert">{error}</p>}{result && <div><p>Root: {result.root.name} · {Math.max(0, visibleVisited.length - 1)} affected nodes in scope · {result.relations.length} evidence relations · risk: {risk}</p>{visibleVisited.length === 0 && <p role="status">No trace nodes are inside the current scope.</p>}<p>Affected clusters: {affectedClusters.length ? affectedClusters.map((cluster) => cluster.id).join(', ') : 'none'}</p><ul>{visibleVisited.map(({ node, hop }) => <li key={node.id}>hop {hop}: {node.name} <button type="button" onClick={() => onFocusNode(node.id)}>Focus</button></li>)}</ul></div>}</section>
}
