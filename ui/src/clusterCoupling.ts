import type { ArchitectureCluster } from './api/architecture'
import type { LayoutEdge } from './graph/types'

export interface ClusterCouplingCell {
  source: ArchitectureCluster
  target: ArchitectureCluster
  count: number
}

export function computeClusterCoupling(clusters: ArchitectureCluster[], edges: LayoutEdge[]): ClusterCouplingCell[] {
  const membership = new Map<string, ArchitectureCluster>()
  for (const cluster of clusters) for (const member of cluster.members) membership.set(member, cluster)
  const counts = new Map<string, number>()
  for (const edge of edges) {
    const source = membership.get(edge.source)
    const target = membership.get(edge.target)
    if (!source || !target) continue
    const key = `${source.id}\0${target.id}`
    counts.set(key, (counts.get(key) ?? 0) + 1)
  }
  return clusters.flatMap((source) => clusters.map((target) => ({ source, target, count: counts.get(`${source.id}\0${target.id}`) ?? 0 })))
}
