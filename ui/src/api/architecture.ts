import { callTool } from './rpc'

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
}

export interface ArchitectureSummary {
  clusters: ArchitectureCluster[]
}

export async function getClusters(): Promise<ArchitectureCluster[]> {
  const summary = await callTool<ArchitectureSummary>('get_architecture', { aspects: ['clusters'] })
  return summary.clusters
}
