import type { ArchitectureNodeSummary } from './api/architecture'
import type { RepositoryTension } from './api/tensions'
import type { ClusterLink, VisualCluster } from './graph/clusterLayout'
import type { PositionedNode } from './graph/types'
import { deriveDisplayRootPrefix, stripDisplayRoot } from './displayRoot'

export interface ClusterIdentity {
  id: string
  name: string
  responsibility: string
  memberCount: number
  visibleMemberCount: number
  fileCount: number
  dominantKinds: string[]
  entryPoints: ArchitectureNodeSummary[]
  incoming: ClusterLink[]
  outgoing: ClusterLink[]
  boundaryInterpretation: string
  tensionCount: number
  highestSeverity?: string
  partial: boolean
}

export function deriveClusterIdentities(
  visualClusters: VisualCluster[],
  nodes: PositionedNode[],
  links: ClusterLink[],
  entryPoints: ArchitectureNodeSummary[] = [],
  tensions: RepositoryTension[] = [],
): Map<string, ClusterIdentity> {
  const displayRootPrefix = deriveDisplayRootPrefix(nodes)
  const displayNodes = nodes.map((node) => ({ ...node, name: stripDisplayRoot(node.name, displayRootPrefix), file_path: node.file_path ? stripDisplayRoot(node.file_path, displayRootPrefix) : null }))
  const nodesById = new Map(displayNodes.map((node) => [node.id, node]))
  const entryPointsById = new Map(entryPoints.map((node) => [node.id, node]))
  const membership = new Map<string, string>()
  for (const cluster of visualClusters) for (const member of cluster.members) membership.set(member, cluster.id)

  const derived = visualClusters.map((cluster) => {
    const visibleNodes = (cluster.renderedMembers ?? cluster.members).map((id) => nodesById.get(id)).filter((node): node is PositionedNode => node !== undefined)
    const paths = visibleNodes.map((node) => node.file_path).filter((path): path is string => Boolean(path))
    const kindCounts = countBy(visibleNodes.map((node) => node.label))
    const dominantKinds = [...kindCounts]
      .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
      .slice(0, 3)
      .map(([kind]) => humanize(kind))
    const clusterEntryPoints = cluster.members
      .map((id) => entryPointsById.get(id))
      .filter((node): node is ArchitectureNodeSummary => node !== undefined)
    const incoming = links.filter((link) => link.target === cluster.id)
    const outgoing = links.filter((link) => link.source === cluster.id)
    const matches = tensions.filter((tension) => tension.affected_nodes.some((id) => membership.get(id) === cluster.id))
    const highestSeverity = [...matches]
      .sort((a, b) => severityRank(b.severity) - severityRank(a.severity) || a.id.localeCompare(b.id))[0]?.severity
    const name = clusterName(cluster, visibleNodes)
    const partial = cluster.members.length < cluster.totalMembers
    return [cluster, {
      id: cluster.id,
      name,
      responsibility: responsibilityFor(name, paths, dominantKinds, clusterEntryPoints.length),
      memberCount: cluster.totalMembers,
      visibleMemberCount: (cluster.renderedMembers ?? cluster.members).length,
      fileCount: new Set(paths).size,
      dominantKinds,
      entryPoints: clusterEntryPoints,
      incoming,
      outgoing,
      boundaryInterpretation: boundaryInterpretation(cluster, incoming, outgoing),
      tensionCount: matches.length,
      highestSeverity,
      partial,
    } satisfies ClusterIdentity] as const
  })
  const duplicateNames = countBy(derived.map(([, identity]) => identity.name))
  return new Map(derived.map(([cluster, identity]) => {
    if ((duplicateNames.get(identity.name) ?? 0) < 2) return [cluster.id, identity]
    const qualifier = clusterQualifier(cluster, nodesById)
    const name = `${identity.name} · ${qualifier}`
    return [cluster.id, { ...identity, name, responsibility: identity.responsibility.replace(identity.name, name) }]
  }))
}

export function humanClusterNameFromEvidence(
  cluster: { id: string; members: string[]; packages?: string[]; top_nodes?: unknown[] },
): string {
  const pseudoVisual: VisualCluster = {
    id: cluster.id,
    members: [...cluster.members],
    totalMembers: cluster.members.length,
    center: [0, 0, 0],
    radius: 1,
    analyticalCluster: {
      id: cluster.id,
      members: cluster.members,
      packages: cluster.packages ?? [],
      top_nodes: cluster.top_nodes ?? [],
      edge_types: [],
      cohesion: 0,
      incoming_pressure: 0,
      outgoing_pressure: 0,
    },
  }
  const rawNodes = topNodes(cluster.top_nodes).map((node) => ({ ...node, x: 0, y: 0, hop: 0 }))
  const displayRootPrefix = deriveDisplayRootPrefix(rawNodes)
  const nodes = rawNodes.map((node) => ({ ...node, name: stripDisplayRoot(node.name, displayRootPrefix), file_path: node.file_path ? stripDisplayRoot(node.file_path, displayRootPrefix) : null }))
  return clusterName(pseudoVisual, nodes)
}

function clusterName(cluster: VisualCluster, nodes: PositionedNode[]): string {
  if (cluster.fallbackKey) return fallbackName(cluster.fallbackKey)
  const paths = nodes.map((node) => node.file_path).filter((path): path is string => Boolean(path))
  const joined = `${paths.join(' ')} ${cluster.id}`.toLowerCase()
  if (/\b(frontend|web|client|ui)\b/.test(joined)) return 'Web frontend'
  if (/\b(routes?|api|fastapi)\b/.test(joined) && /\.py\b|python/.test(joined)) return 'Python API'
  if (/\b(worker|jobs?|queue)\b/.test(joined)) return 'Worker runtime'
  if (/\b(config|settings|\.env|pyproject|package\.json|cargo\.toml)\b/.test(joined)) return 'Configuration and dependencies'
  if (/\b(docs?|readme|tooling|scripts?)\b/.test(joined)) return 'Documentation and tooling'
  if (/\btests?|specs?|fixtures?\b/.test(joined)) return 'Tests and fixtures'
  const pathParts = paths.flatMap((path) => path.split('/').slice(0, -1)).filter((part) => !['src', 'lib', 'app'].includes(part.toLowerCase()))
  const dominantPath = mostCommon(pathParts)
  if (dominantPath) return `${dominantPath} subsystem`
  const representative = nodes
    .sort((a, b) => (b.in_degree + b.out_degree) - (a.in_degree + a.out_degree) || a.id.localeCompare(b.id))[0]
  if (representative) return `${shortName(representative.name)} subsystem`
  const rawFallback = cluster.id.split(/[/:#]/).filter(Boolean).at(-1)
  return rawFallback ? `${titleCase(rawFallback)} subsystem` : 'Architecture subsystem'
}

function fallbackName(key: string): string {
  if (key === 'dependencies') return 'External dependencies'
  if (key === 'external-references') return 'Unresolved external references'
  if (key === 'configuration') return 'Configuration'
  if (key === 'documentation-tooling') return 'Documentation and tooling'
  if (key === 'path:repository-root') return 'Repository root'
  const value = key.replace(/^(path|kind):/, '').split('/').at(-1) ?? key
  if (/web|frontend|client|ui/i.test(value)) return 'Web frontend'
  if (/python|api|backend/i.test(value)) return 'Python API'
  if (/worker|job|queue/i.test(value)) return 'Worker runtime'
  return `${value} area`
}

function clusterQualifier(cluster: VisualCluster, nodesById: Map<string, PositionedNode>): string {
  const representative = cluster.members
    .map((id) => nodesById.get(id))
    .filter((node): node is PositionedNode => node !== undefined)
    .sort((a, b) => (b.in_degree + b.out_degree) - (a.in_degree + a.out_degree) || a.id.localeCompare(b.id))[0]
  const source = representative?.file_path?.split('/').at(-1)
    ?? representative?.name
    ?? cluster.fallbackKey?.replace(/^(path|kind):/, '').split('/').at(-1)
    ?? cluster.id.split(/[/:#]/).filter(Boolean).at(-1)
    ?? 'region'
  const withoutExtension = source.replace(/\.(tsx?|jsx?|py|rs|toml|json|ya?ml|md|html|css)$/i, '')
  return withoutExtension.replace(/^_+|_+$/g, '') || 'Region'
}

function responsibilityFor(name: string, paths: string[], kinds: string[], entryPointCount: number): string {
  const kindPhrase = kinds.length ? kinds.slice(0, 2).join(' and ').toLowerCase() : 'repository elements'
  const location = mostCommon(paths.map((path) => path.split('/')[0]).filter(Boolean))
  const entryPhrase = entryPointCount > 0 ? `, including ${entryPointCount} entry point${entryPointCount === 1 ? '' : 's'}` : ''
  return `${name} groups ${kindPhrase}${location ? ` around ${location}` : ''}${entryPhrase}.`
}

function boundaryInterpretation(cluster: VisualCluster, incoming: ClusterLink[], outgoing: ClusterLink[]): string {
  const analytical = cluster.analyticalCluster
  const incomingCount = incoming.reduce((sum, link) => sum + link.count, 0)
  const outgoingCount = outgoing.reduce((sum, link) => sum + link.count, 0)
  if (analytical) {
    const pressure = analytical.incoming_pressure + analytical.outgoing_pressure
    if (analytical.cohesion >= 0.65 && pressure <= 2) return 'Cohesive and relatively self-contained in the current evidence.'
    if (analytical.cohesion < 0.25) return 'Loosely connected; treat this analytical grouping as a navigation hypothesis.'
    if (pressure >= 8) return 'High boundary pressure; review its cross-subsystem responsibilities.'
  }
  if (incomingCount + outgoingCount === 0) return 'No cross-region relationships are visible in the current slice.'
  if (incomingCount > outgoingCount * 2) return 'Primarily depended on by other regions.'
  if (outgoingCount > incomingCount * 2) return 'Primarily depends on services owned by other regions.'
  return 'Shares responsibilities across several region boundaries.'
}

function topNodes(value: unknown[] | undefined): Array<Omit<PositionedNode, 'x' | 'y' | 'hop'>> {
  if (!Array.isArray(value)) return []
  return value.filter((item): item is Omit<PositionedNode, 'x' | 'y' | 'hop'> => {
    if (!item || typeof item !== 'object') return false
    const node = item as Record<string, unknown>
    return typeof node.id === 'string' && typeof node.label === 'string' && typeof node.name === 'string'
      && typeof node.in_degree === 'number' && typeof node.out_degree === 'number'
  })
}

function countBy(values: string[]): Map<string, number> {
  const counts = new Map<string, number>()
  for (const value of values) counts.set(value, (counts.get(value) ?? 0) + 1)
  return counts
}

function mostCommon(values: string[]): string | undefined {
  return [...countBy(values)].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))[0]?.[0]
}

function shortName(value: string): string {
  return value.split(/[/:#]/).filter(Boolean).at(-1) ?? value
}

function humanize(value: string): string {
  return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[-_]+/g, ' ').replace(/^\w/, (letter) => letter.toUpperCase())
}

function titleCase(value: string): string {
  return humanize(value).replace(/\b\w/g, (letter) => letter.toUpperCase())
}

function severityRank(value: string): number {
  return ({ critical: 4, high: 3, medium: 2, low: 1 }[value.toLowerCase()] ?? 0)
}
