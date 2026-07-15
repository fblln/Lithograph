import { describe, expect, it } from 'vitest'
import type { ArchitectureSummary } from '../api/architecture'
import type { RepositoryTension } from '../api/tensions'
import { deriveClusterIdentities } from '../clusterIdentity'
import { computeClusterLayout } from './clusterLayout'
import type { LayoutResult } from './types'

const live = import.meta.env.VITE_RUN_LIVE_EXPLORER_DIAGNOSTICS === '1'

describe.skipIf(!live)('live explorer diagnostics', () => {
  it.each([
    ['polyglot', 'http://127.0.0.1:4317/rpc', 150, 400],
    ['full-stack-fastapi', 'http://127.0.0.1:4318/rpc', 1_000, 1_600],
  ])('covers and names the %s repository graph deterministically', async (name, endpoint, maxNodes, maxEdges) => {
    const [layout, architecture, tensions] = await Promise.all([
      callTool<LayoutResult>(endpoint, 'get_graph_layout', { max_nodes: maxNodes, max_edges: maxEdges }),
      callTool<ArchitectureSummary>(endpoint, 'get_architecture', { aspects: ['clusters', 'entry_points'] }),
      callTool<RepositoryTension[]>(endpoint, 'get_repository_tensions', {}),
    ])
    const wholeGraphLinks = architecture.cluster_links ?? []
    const first = computeClusterLayout(layout.nodes, architecture.clusters, layout.edges, wholeGraphLinks)
    const second = computeClusterLayout(layout.nodes, architecture.clusters, layout.edges, wholeGraphLinks)
    const identities = deriveClusterIdentities(first.clusters, layout.nodes, first.links, architecture.entry_points, tensions)
    const summary = {
      name,
      nodes: `${layout.budget.nodes_returned}/${layout.budget.nodes_available}`,
      relationships: `${layout.budget.edges_returned}/${layout.budget.edges_available}`,
      analyticalClusters: architecture.clusters.length,
      visualRegions: first.clusters.length,
      fallbackRegions: first.clusters.filter((cluster) => cluster.fallbackKey).length,
      partialRegions: [...identities.values()].filter((identity) => identity.partial).length,
      aggregateLinks: first.links.length,
      wholeGraphAggregateRelationships: wholeGraphLinks.reduce((total, link) => total + link.count, 0),
      names: [...identities.values()].slice(0, 12).map((identity) => identity.name),
      tensions: tensions.length,
    }
    console.info(`LIVE_EXPLORER_DIAGNOSTIC ${JSON.stringify(summary)}`)

    expect(first.positions.size).toBe(layout.nodes.length)
    expect(first.positions).toEqual(second.positions)
    expect(first.links).toEqual(second.links)
    expect(architecture.cluster_links).toEqual((await callTool<ArchitectureSummary>(endpoint, 'get_architecture', { aspects: ['clusters', 'entry_points'] })).cluster_links)
    expect(first.clusters.filter((cluster) => cluster.analyticalCluster)).toHaveLength(architecture.clusters.length)
    expect([...identities.values()].every((identity) => !identity.name.startsWith('cluster:'))).toBe(true)
    expect(layout.nodes.length).toBeGreaterThan(0)
  }, 30_000)
})

async function callTool<T>(endpoint: string, name: string, arguments_: Record<string, unknown>): Promise<T> {
  const response = await fetch(endpoint, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', id: 1, method: 'tools/call', params: { name, arguments: arguments_ } }),
  })
  const envelope = await response.json() as { result?: { content?: Array<{ text?: string }> }; error?: { message?: string } }
  if (envelope.error) throw new Error(envelope.error.message ?? 'RPC failed')
  const text = envelope.result?.content?.[0]?.text
  if (!text) throw new Error(`${name} returned no text payload`)
  return JSON.parse(text) as T
}
