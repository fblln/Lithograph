import { useEffect, useState } from 'react'
import { getGraphAnalytics, type Analytics } from '../api/analytics'
import { getClusters, type ArchitectureCluster } from '../api/architecture'

const ignoreMetricValues = () => {}

export function AnalyticsPanel({ onFocusNode, onMetricValues = ignoreMetricValues, onSemanticLabels = () => {} }: { onFocusNode: (id: string) => void; onMetricValues?: (values: Map<string, number>) => void; onSemanticLabels?: (labels: string[]) => void }) {
  const [data, setData] = useState<Analytics | null>(null)
  const [metric, setMetric] = useState<'betweenness' | 'page_rank' | 'fan_in' | 'fan_out'>('betweenness')
  const [finding, setFinding] = useState<number | null>(null)
  const [clusters, setClusters] = useState<ArchitectureCluster[]>([])
  const [semantic, setSemantic] = useState('All classes')
  useEffect(() => { getGraphAnalytics().then(setData, () => setData(null)) }, [])
  useEffect(() => { getClusters().then(setClusters, () => setClusters([])) }, [])
  useEffect(() => { onSemanticLabels(semantic === 'Source code' ? ['Artifact', 'Module', 'Symbol'] : semantic === 'Configuration' ? ['Config', 'EnvVar'] : semantic === 'Documentation' ? ['Documentation'] : []) }, [semantic, onSemanticLabels])
  useEffect(() => {
    if (!data) return
    const maximum = Math.max(...data.nodes.map((node) => node[metric]), 1)
    onMetricValues(new Map(data.nodes.map((node) => [node.id, node[metric] / maximum])))
  }, [data, metric, onMetricValues])
  if (!data) return <section className="p-3">Loading analytics…</section>
  const ranked = [...data.nodes].sort((a, b) => b[metric] - a[metric]).slice(0, 10)
  return <section className="p-3 text-[11px]"><h2>Analytics overlays</h2><label>Semantic class <select value={semantic} onChange={(event) => setSemantic(event.target.value)}><option>All classes</option><option>Source code</option><option>Configuration</option><option>Documentation</option></select></label><label>Color and size by <select value={metric} onChange={(event) => setMetric(event.target.value as typeof metric)}><option value="betweenness">Betweenness</option><option value="page_rank">PageRank</option><option value="fan_in">Fan-in</option><option value="fan_out">Fan-out</option></select></label><p>Overlay ranking updates locally; it does not reload the graph.</p><ul>{ranked.map((node) => <li key={node.id}><button type="button" onClick={() => onFocusNode(node.id)}>{node.id}</button> · {node[metric].toFixed(3)}</li>)}</ul><h3>Cluster explorer</h3>{clusters.map((cluster) => <button key={cluster.id} type="button" onClick={() => onFocusNode(cluster.members[0])}>{cluster.id} · {cluster.members.length} members</button>)}<h3>Health dashboard</h3>{data.findings.map((item, index) => <button key={item.id} type="button" onClick={() => setFinding(index)}>{item.severity}: {item.rule}</button>)}{finding !== null && <div><h3>Why this result</h3><p>{data.findings[finding].evidence.join(', ')}</p><button type="button" onClick={() => onFocusNode(data.findings[finding].affected_nodes[0])}>Focus finding</button></div>}</section>
}
