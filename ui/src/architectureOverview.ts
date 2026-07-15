import type { ArchitectureNodeSummary } from './api/architecture'
import type { RepositoryTension } from './api/tensions'
import type { LayoutResult, PositionedNode } from './graph/types'

export interface RepositoryArea {
  id: string
  name: string
  nodeIds: string[]
  nodeCount: number
  fileCount: number
  incoming: number
  outgoing: number
  connectedAreas: string[]
  entryPoints: ArchitectureNodeSummary[]
  tensionCount: number
}

export function deriveRepositoryAreas(layout: LayoutResult, entryPoints: ArchitectureNodeSummary[], tensions: RepositoryTension[]): RepositoryArea[] {
  const areaByNode = new Map<string, string>()
  const groups = new Map<string, PositionedNode[]>()
  for (const node of layout.nodes) {
    // Package, container, and unresolved nodes often have no repository path.
    // They still participate in the graph, but presenting them as a giant
    // fictional "root" subsystem would hide the application's real shape.
    if (!node.file_path) continue
    const area = areaForPath(node.file_path)
    areaByNode.set(node.id, area)
    groups.set(area, [...(groups.get(area) ?? []), node])
  }

  return [...groups].map(([id, nodes]) => {
    const connected = new Set<string>()
    let incoming = 0
    let outgoing = 0
    for (const edge of layout.edges) {
      const sourceArea = areaByNode.get(edge.source)
      const targetArea = areaByNode.get(edge.target)
      if (!sourceArea || !targetArea || sourceArea === targetArea) continue
      if (sourceArea === id) { outgoing += 1; connected.add(targetArea) }
      if (targetArea === id) { incoming += 1; connected.add(sourceArea) }
    }
    const nodeIds = nodes.map((node) => node.id).sort()
    const ids = new Set(nodeIds)
    return {
      id,
      name: humanAreaName(id),
      nodeIds,
      nodeCount: nodes.length,
      fileCount: new Set(nodes.map((node) => node.file_path).filter((path): path is string => Boolean(path))).size,
      incoming,
      outgoing,
      connectedAreas: [...connected].sort().map(humanAreaName),
      entryPoints: entryPoints.filter((entry) => areaForPath(entry.file_path) === id),
      tensionCount: tensions.filter((tension) => tension.affected_nodes.some((nodeId) => ids.has(nodeId))).length,
    }
  }).sort((a, b) => b.nodeCount - a.nodeCount || a.id.localeCompare(b.id))
}

function areaForPath(path: string | null): string {
  if (!path) return 'root'
  const parts = path.split('/').filter(Boolean)
  if (parts.length <= 1) return 'root'
  if (['src', 'apps', 'packages', 'services', 'crates'].includes(parts[0]) && parts[1]) return `${parts[0]}/${parts[1]}`
  return parts[0]
}

function humanAreaName(id: string): string {
  if (id === 'root') return 'Repository root'
  const value = id.split('/').at(-1) ?? id
  return value.replace(/[-_.]+/g, ' ').replace(/\b\w/g, (letter) => letter.toUpperCase())
}
