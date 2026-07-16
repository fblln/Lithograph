import { callTool } from './rpc'
import type { GraphTag } from './tags'

/**
 * Mirrors the JSON shape of `get_architecture`'s response for the
 * `clusters` aspect (Rust server, same `tools/call` envelope as
 * `get_graph_layout` -- see `./rpc`). Field names match the server's serde
 * output verbatim (snake_case). Only the `clusters` aspect's shape is
 * modeled precisely here; the summary carries other aspects (schema,
 * hotspots, ...) that this client does not need.
 */
export interface ArchitectureCluster {
  id: string
  members: string[]
  top_nodes: unknown[]
  packages: string[]
  edge_types: string[]
  cohesion: number
  incoming_pressure: number
  outgoing_pressure: number
  tags?: GraphTag[]
}

export interface ArchitectureNodeSummary {
  id: string
  label: string
  name: string
  file_path: string | null
  in_degree: number
  out_degree: number
}

export interface ArchitectureSummary {
  clusters: ArchitectureCluster[]
  cluster_links?: ArchitectureClusterLink[]
  entry_points: ArchitectureNodeSummary[]
  hotspots: ArchitectureNodeSummary[]
}

export interface ArchitectureClusterLink {
  source: string
  target: string
  count: number
  kinds: Array<{ kind: string; count: number }>
  underlying: Array<{ source: string; target: string; kind: string }>
}

export async function getArchitectureSummary(): Promise<ArchitectureSummary> {
  const summary = await callTool<unknown>('get_architecture', { aspects: ['clusters', 'entry_points'] })
  if (typeof summary !== 'object' || summary === null) return { clusters: [], cluster_links: [], entry_points: [], hotspots: [] }
  const value = summary as Partial<ArchitectureSummary>
  return {
    clusters: Array.isArray(value.clusters) ? value.clusters : [],
    cluster_links: Array.isArray(value.cluster_links) ? value.cluster_links : [],
    entry_points: Array.isArray(value.entry_points) ? value.entry_points : [],
    hotspots: Array.isArray(value.hotspots) ? value.hotspots : [],
  }
}

export async function getClusters(): Promise<ArchitectureCluster[]> {
  return (await getArchitectureSummary()).clusters
}
