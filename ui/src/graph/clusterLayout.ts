import type { ArchitectureCluster } from '../api/architecture'
import type { LayoutEdge, PositionedNode } from './types'

export type WorldPosition = [number, number, number]

export interface VisualCluster {
  id: string
  analyticalCluster?: ArchitectureCluster
  members: string[]
  totalMembers: number
  fallbackKey?: string
  center: WorldPosition
  radius: number
}

export interface ClusterLink {
  source: string
  target: string
  count: number
  kinds: Array<{ kind: string; count: number }>
  underlying: LayoutEdge[]
}

export interface ClusterLayoutResult {
  positions: Map<string, WorldPosition>
  clusters: VisualCluster[]
  membership: Map<string, string>
  links: ClusterLink[]
}

const GLOBAL_ITERATIONS = 180
const LOCAL_ITERATIONS = 110
const EPSILON = 1e-6

/**
 * Builds one deterministic visual-cluster model for the whole rendered
 * slice. Analytical communities remain primary. Nodes that are not assigned
 * by community detection are placed in evidence-based fallback regions so a
 * bounded overview never turns most of a repository into unlabeled debris.
 */
export function computeClusterLayout(
  nodes: PositionedNode[],
  clusters: ArchitectureCluster[],
  edges: LayoutEdge[] = [],
  wholeGraphLinks: ClusterLink[] = [],
): ClusterLayoutResult {
  const orderedNodes = [...nodes].sort((a, b) => a.id.localeCompare(b.id))
  const nodeIds = new Set(orderedNodes.map((node) => node.id))
  const membership = new Map<string, string>()
  const visualClusters: VisualCluster[] = []

  for (const cluster of [...clusters].sort((a, b) => a.id.localeCompare(b.id))) {
    const normalizedCluster = {
      ...cluster,
      members: [...cluster.members].sort(),
      packages: [...cluster.packages].sort(),
      edge_types: [...cluster.edge_types].sort(),
    }
    const visibleMembers = normalizedCluster.members
      .filter((id) => nodeIds.has(id) && !membership.has(id))
      .sort()
    for (const id of visibleMembers) membership.set(id, cluster.id)
    visualClusters.push({
      id: cluster.id,
      analyticalCluster: normalizedCluster,
      members: visibleMembers,
      totalMembers: normalizedCluster.members.length,
      center: [0, 0, 0],
      radius: clusterRadius(Math.max(1, visibleMembers.length)),
    })
  }

  const fallbackGroups = new Map<string, string[]>()
  for (const node of orderedNodes) {
    if (membership.has(node.id)) continue
    const key = fallbackGroupKey(node)
    const members = fallbackGroups.get(key) ?? []
    members.push(node.id)
    fallbackGroups.set(key, members)
  }
  for (const [fallbackKey, members] of [...fallbackGroups].sort(([a], [b]) => a.localeCompare(b))) {
    const id = `visual:${fallbackKey}`
    members.sort()
    for (const member of members) membership.set(member, id)
    visualClusters.push({
      id,
      members,
      totalMembers: members.length,
      fallbackKey,
      center: [0, 0, 0],
      radius: clusterRadius(members.length),
    })
  }

  const links = mergeClusterLinks(aggregateClusterLinks(edges, membership), wholeGraphLinks)
  const centers = simulateClusterCenters(visualClusters, links)
  const positions = new Map<string, WorldPosition>()
  const visibleEdges = [...edges].sort(compareEdges)

  for (const cluster of visualClusters) {
    cluster.center = centers.get(cluster.id) ?? [0, 0, 0]
    const memberSet = new Set(cluster.members)
    const internalEdges = visibleEdges.filter(
      (edge) => memberSet.has(edge.source) && memberSet.has(edge.target),
    )
    const local = simulateLocalMembers(cluster.members, internalEdges, cluster.radius)
    for (const [id, [x, y, z]] of local) {
      positions.set(id, [cluster.center[0] + x, y, cluster.center[2] + z])
    }
  }

  return { positions, clusters: visualClusters, membership, links }
}

function mergeClusterLinks(rendered: ClusterLink[], wholeGraph: ClusterLink[]): ClusterLink[] {
  const merged = new Map(rendered.map((link) => [`${link.source}\0${link.target}`, link]))
  for (const link of wholeGraph) merged.set(`${link.source}\0${link.target}`, link)
  return [...merged.values()].sort((a, b) =>
    a.source.localeCompare(b.source) || a.target.localeCompare(b.target),
  )
}

/** Compatibility wrapper retained for callers that only need node positions. */
export function computeClusterPositions(
  nodes: PositionedNode[],
  clusters: ArchitectureCluster[],
  edges: LayoutEdge[] = [],
): Map<string, WorldPosition> {
  return computeClusterLayout(nodes, clusters, edges).positions
}

export function aggregateClusterLinks(
  edges: LayoutEdge[],
  membership: Map<string, string>,
): ClusterLink[] {
  const grouped = new Map<string, LayoutEdge[]>()
  for (const edge of [...edges].sort(compareEdges)) {
    const source = membership.get(edge.source)
    const target = membership.get(edge.target)
    if (!source || !target || source === target) continue
    const key = `${source}\0${target}`
    const values = grouped.get(key) ?? []
    values.push(edge)
    grouped.set(key, values)
  }
  return [...grouped]
    .sort(([a], [b]) => a.localeCompare(b))
    .map(([key, underlying]) => {
      const [source, target] = key.split('\0')
      const kindCounts = new Map<string, number>()
      for (const edge of underlying) {
        kindCounts.set(edge.kind, (kindCounts.get(edge.kind) ?? 0) + 1)
      }
      const kinds = [...kindCounts]
        .map(([kind, count]) => ({ kind, count }))
        .sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind))
      return { source, target, count: underlying.length, kinds, underlying }
    })
}

function simulateClusterCenters(
  clusters: VisualCluster[],
  links: ClusterLink[],
): Map<string, WorldPosition> {
  if (clusters.length === 0) return new Map()
  if (clusters.length === 1) return new Map([[clusters[0].id, [0, 0, 0]]])
  const state = new Map<string, { x: number; z: number; vx: number; vz: number }>()
  const scale = Math.max(3.5, Math.sqrt(clusters.length) * 2.4)
  for (const cluster of clusters) {
    const [seedX, seedZ] = seededPair(cluster.id)
    state.set(cluster.id, { x: seedX * scale, z: seedZ * scale, vx: 0, vz: 0 })
  }

  for (let iteration = 0; iteration < GLOBAL_ITERATIONS; iteration += 1) {
    const cooling = 1 - iteration / GLOBAL_ITERATIONS
    for (let leftIndex = 0; leftIndex < clusters.length; leftIndex += 1) {
      for (let rightIndex = leftIndex + 1; rightIndex < clusters.length; rightIndex += 1) {
        const left = clusters[leftIndex]
        const right = clusters[rightIndex]
        const a = state.get(left.id)!
        const b = state.get(right.id)!
        let dx = b.x - a.x
        let dz = b.z - a.z
        let distance = Math.hypot(dx, dz)
        if (distance < EPSILON) {
          const [jx, jz] = seededPair(`${left.id}\0${right.id}`)
          dx = jx || 0.01
          dz = jz || -0.01
          distance = Math.hypot(dx, dz)
        }
        const minimum = left.radius + right.radius + 1.1
        const repulsion = 0.075 * (left.radius + right.radius + 1) / Math.max(0.3, distance)
        const collision = distance < minimum ? (minimum - distance) * 0.18 : 0
        const force = (repulsion + collision) * cooling
        const fx = (dx / distance) * force
        const fz = (dz / distance) * force
        a.vx -= fx
        a.vz -= fz
        b.vx += fx
        b.vz += fz
      }
    }

    for (const link of links) {
      const source = state.get(link.source)
      const target = state.get(link.target)
      if (!source || !target) continue
      const dx = target.x - source.x
      const dz = target.z - source.z
      const distance = Math.max(EPSILON, Math.hypot(dx, dz))
      const desired = 2.5 + 3.5 / Math.sqrt(link.count + 1)
      const force = (distance - desired) * Math.min(0.055, 0.018 + link.count * 0.004) * cooling
      const fx = (dx / distance) * force
      const fz = (dz / distance) * force
      source.vx += fx
      source.vz += fz
      target.vx -= fx
      target.vz -= fz
    }

    for (const cluster of clusters) {
      const item = state.get(cluster.id)!
      item.vx -= item.x * 0.006 * cooling
      item.vz -= item.z * 0.006 * cooling
      item.vx *= 0.72
      item.vz *= 0.72
      item.x += item.vx
      item.z += item.vz
    }
  }

  // Remove floating-point drift so snapshots remain compact and exact.
  return new Map(clusters.map((cluster) => {
    const item = state.get(cluster.id)!
    return [cluster.id, [round(item.x), 0, round(item.z)] as WorldPosition]
  }))
}

function simulateLocalMembers(
  members: string[],
  edges: LayoutEdge[],
  radius: number,
): Map<string, WorldPosition> {
  if (members.length === 0) return new Map()
  if (members.length === 1) return new Map([[members[0], [0, 0, 0]]])
  const ordered = [...members].sort()
  const state = new Map<string, { x: number; z: number; vx: number; vz: number }>()
  for (const id of ordered) {
    const [seedX, seedZ] = seededPair(id)
    state.set(id, { x: seedX * radius * 0.62, z: seedZ * radius * 0.62, vx: 0, vz: 0 })
  }

  for (let iteration = 0; iteration < LOCAL_ITERATIONS; iteration += 1) {
    const cooling = 1 - iteration / LOCAL_ITERATIONS
    for (let leftIndex = 0; leftIndex < ordered.length; leftIndex += 1) {
      // A bounded, stable neighbor window keeps large clusters linear while
      // still preventing local piles. The ordering and window are fixed.
      const limit = Math.min(ordered.length, leftIndex + 25)
      for (let rightIndex = leftIndex + 1; rightIndex < limit; rightIndex += 1) {
        const a = state.get(ordered[leftIndex])!
        const b = state.get(ordered[rightIndex])!
        let dx = b.x - a.x
        let dz = b.z - a.z
        let distance = Math.hypot(dx, dz)
        if (distance < EPSILON) {
          const [jx, jz] = seededPair(`${ordered[leftIndex]}\0${ordered[rightIndex]}`)
          dx = jx || 0.01
          dz = jz || 0.01
          distance = Math.hypot(dx, dz)
        }
        const force = 0.018 * cooling / Math.max(0.12, distance * distance)
        const fx = (dx / distance) * force
        const fz = (dz / distance) * force
        a.vx -= fx
        a.vz -= fz
        b.vx += fx
        b.vz += fz
      }
    }
    for (const edge of edges) {
      const source = state.get(edge.source)
      const target = state.get(edge.target)
      if (!source || !target) continue
      const dx = target.x - source.x
      const dz = target.z - source.z
      const distance = Math.max(EPSILON, Math.hypot(dx, dz))
      const force = (distance - 0.55) * 0.025 * cooling
      const fx = (dx / distance) * force
      const fz = (dz / distance) * force
      source.vx += fx
      source.vz += fz
      target.vx -= fx
      target.vz -= fz
    }
    for (const id of ordered) {
      const item = state.get(id)!
      item.vx -= item.x * 0.016 * cooling
      item.vz -= item.z * 0.016 * cooling
      item.vx *= 0.69
      item.vz *= 0.69
      item.x += item.vx
      item.z += item.vz
    }
  }
  return new Map(ordered.map((id) => {
    const item = state.get(id)!
    return [id, [round(item.x), 0, round(item.z)] as WorldPosition]
  }))
}

function fallbackGroupKey(node: PositionedNode): string {
  if (!node.file_path) {
    if (node.label === 'Package') return 'dependencies'
    if (node.label === 'Unresolved') return 'external-references'
    return `kind:${slug(node.label)}`
  }
  const parts = node.file_path.split('/').filter(Boolean)
  const file = parts.at(-1)?.toLowerCase() ?? ''
  if (parts.length === 1 && /(^|\.)(toml|json|ya?ml|lock|env)$/.test(file)) return 'configuration'
  if (parts[0] === 'docs' || file.endsWith('.md')) return 'documentation-tooling'
  if (parts[0] === 'src' && parts.length > 2) return `path:${parts.slice(0, 2).join('/')}`
  return `path:${parts.length > 1 ? parts[0] : 'repository-root'}`
}

function clusterRadius(memberCount: number): number {
  return Math.max(0.75, Math.sqrt(memberCount) * 0.28 + 0.45)
}

function seededPair(value: string): [number, number] {
  const first = hashString(value)
  const second = hashString(`${value}\0layout`)
  return [((first & 0xffff) / 0x7fff) - 1, ((second & 0xffff) / 0x7fff) - 1]
}

function hashString(value: string): number {
  let hash = 2166136261
  for (let index = 0; index < value.length; index += 1) {
    hash ^= value.charCodeAt(index)
    hash = Math.imul(hash, 16777619)
  }
  return hash >>> 0
}

function compareEdges(a: LayoutEdge, b: LayoutEdge): number {
  return a.source.localeCompare(b.source)
    || a.target.localeCompare(b.target)
    || a.kind.localeCompare(b.kind)
}

function round(value: number): number {
  return Math.round(value * 1_000_000) / 1_000_000
}

function slug(value: string): string {
  return value.replace(/([a-z])([A-Z])/g, '$1-$2').replace(/[^a-zA-Z0-9]+/g, '-').toLowerCase()
}
