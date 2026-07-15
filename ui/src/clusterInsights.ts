import type { ArchitectureCluster } from './api/architecture'
import type { RepositoryTension } from './api/tensions'
import type { LayoutResult } from './graph/types'

export interface ClusterInsight {
  cluster: ArchitectureCluster
  visibleMembers: string[]
  bridgeNodes: string[]
  boundaryEdges: number
  conductance: number
  dominantKinds: string[]
  tensions: RepositoryTension[]
}

/** Derives current-slice boundary metrics without pretending truncated members were rendered. */
export function deriveClusterInsights(layout: LayoutResult, clusters: ArchitectureCluster[], tensions: RepositoryTension[]): ClusterInsight[] {
  const membership = new Map<string, string>()
  for (const cluster of clusters) for (const id of cluster.members) membership.set(id, cluster.id)
  const visible = new Set(layout.nodes.map((node) => node.id))
  return clusters.map((cluster) => {
    const boundary = layout.edges.filter((edge) =>
      (membership.get(edge.source) === cluster.id && membership.get(edge.target) !== cluster.id)
      || (membership.get(edge.target) === cluster.id && membership.get(edge.source) !== cluster.id),
    )
    const internal = layout.edges.filter((edge) => membership.get(edge.source) === cluster.id && membership.get(edge.target) === cluster.id).length
    const bridgeNodes = [...new Set(boundary.flatMap((edge) => [edge.source, edge.target]).filter((id) => membership.get(id) === cluster.id))].sort()
    const kindCounts = new Map<string, number>()
    for (const node of layout.nodes) if (membership.get(node.id) === cluster.id) kindCounts.set(node.label, (kindCounts.get(node.label) ?? 0) + 1)
    return {
      cluster,
      visibleMembers: cluster.members.filter((id) => visible.has(id)),
      bridgeNodes,
      boundaryEdges: boundary.length,
      conductance: boundary.length / Math.max(1, boundary.length + internal * 2),
      dominantKinds: [...kindCounts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0])).slice(0, 2).map(([kind]) => `kind:${kind}`),
      tensions: tensions.filter((tension) => tension.affected_nodes.some((id) => membership.get(id) === cluster.id)),
    }
  })
}
