import { useMemo } from 'react'
import { Canvas } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import type { ArchitectureCluster, ArchitectureClusterLink, ArchitectureNodeSummary } from '../api/architecture'
import type { RepositoryTension } from '../api/tensions'
import type { ViewMode } from '../components/ViewModeToggle'
import type { LayoutResult, PositionedNode } from './types'
import type { DragPositions } from './useDragPositions'
import { NodeCloud } from './NodeCloud'
import { EdgeLines } from './EdgeLines'
import { ClusterHulls } from './ClusterHulls'
import { nodeWorldPosition } from './positions'
import { computeMatrixPositions } from './matrixLayout'
import { computeClusterLayout, type VisualCluster } from './clusterLayout'
import { cameraFrameForPositions } from './cameraFrame'
import type { EdgeView } from '../components/GraphToolbar'
import { deriveClusterIdentities } from '../clusterIdentity'
import { NodeLabels } from './NodeLabels'

export interface GraphSceneProps {
  layout: LayoutResult
  viewMode: ViewMode
  clusters: ArchitectureCluster[]
  selectedId: string | null
  onSelect: (node: PositionedNode) => void
  dragPositions: DragPositions
  metricValues?: Map<string, number>
  interClusterOnly?: boolean
  edgeView?: EdgeView
  zoom?: number
  entryPoints?: ArchitectureNodeSummary[]
  tensions?: RepositoryTension[]
  selectedClusterId?: string | null
  onSelectCluster?: (cluster: VisualCluster) => void
  onEnterCluster?: (cluster: VisualCluster) => void
  edgeKinds?: Set<string>
  clusterLinks?: ArchitectureClusterLink[]
  onFocusNode?: (node: PositionedNode) => void
}

export function GraphScene({
  layout,
  viewMode,
  clusters,
  selectedId,
  onSelect,
  dragPositions,
  metricValues,
  interClusterOnly = false,
  edgeView = 'nodes',
  zoom = 1,
  entryPoints = [],
  tensions = [],
  selectedClusterId = null,
  onSelectCluster,
  onEnterCluster,
  edgeKinds = new Set(),
  clusterLinks = [],
  onFocusNode,
}: GraphSceneProps) {
  const visibleEdges = useMemo(
    () => edgeKinds.size === 0 ? layout.edges : layout.edges.filter((edge) => edgeKinds.has(edge.kind)),
    [edgeKinds, layout.edges],
  )
  const completeLinks = useMemo(() => clusterLinks.map((link) => {
    const underlying = edgeKinds.size === 0 ? link.underlying : link.underlying.filter((edge) => edgeKinds.has(edge.kind))
    const counts = new Map<string, number>()
    for (const edge of underlying) counts.set(edge.kind, (counts.get(edge.kind) ?? 0) + 1)
    return { ...link, count: underlying.length, underlying, kinds: [...counts].map(([kind, count]) => ({ kind, count })).sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind)) }
  }).filter((link) => link.count > 0), [clusterLinks, edgeKinds])
  const clusterLayout = useMemo(
    () => computeClusterLayout(layout.nodes, clusters, visibleEdges, completeLinks),
    [clusters, completeLinks, layout.nodes, visibleEdges],
  )
  const basePositions = useMemo(() => {
    if (viewMode === 'matrix') return computeMatrixPositions(layout.nodes)
    if (viewMode === 'cluster') return clusterLayout.positions
    return new Map(layout.nodes.map((node) => [node.id, nodeWorldPosition(node)] as const))
  }, [clusterLayout.positions, layout.nodes, viewMode])

  // Drag overrides win over whichever base layout is active, and are kept
  // in one map so NodeCloud and EdgeLines never disagree about where a
  // node actually is.
  const positions = useMemo(() => {
    const merged = new Map(basePositions)
    for (const node of layout.nodes) {
      const override = dragPositions.getOverride(node.id)
      if (override) merged.set(node.id, override)
    }
    return merged
  }, [basePositions, layout.nodes, dragPositions])

  const camera = useMemo(() => {
    const framedPositions = new Map(positions)
    if (viewMode === 'cluster') {
      for (const cluster of clusterLayout.clusters) framedPositions.set(`cluster-center:${cluster.id}`, cluster.center)
    }
    const frame = cameraFrameForPositions(framedPositions)
    return {
      ...frame,
      position: frame.position.map((value, index) => frame.target[index] + (value - frame.target[index]) * zoom) as [number, number, number],
    }
  }, [clusterLayout.clusters, positions, viewMode, zoom])
  const { target, ...cameraProps } = camera
  const clusterIdentities = useMemo(
    () => deriveClusterIdentities(clusterLayout.clusters, layout.nodes, clusterLayout.links, entryPoints, tensions),
    [clusterLayout.clusters, clusterLayout.links, entryPoints, layout.nodes, tensions],
  )
  const selectedClusterMembers = useMemo(() => {
    if (!selectedClusterId) return undefined
    const cluster = clusterLayout.clusters.find((candidate) => candidate.id === selectedClusterId)
    return cluster ? new Set(cluster.members) : undefined
  }, [clusterLayout.clusters, selectedClusterId])
  const entryPointIds = useMemo(() => new Set(entryPoints.map((node) => node.id)), [entryPoints])
  const edgeRendering = useMemo(() => {
    if (edgeView === 'clusters') {
      const clusterPositions = new Map(clusterLayout.clusters.map((cluster) => [cluster.id, cluster.center] as const))
      const edges = clusterLayout.links.map((link) => ({
        source: link.source,
        target: link.target,
        kind: link.kinds[0]?.kind ?? 'Relationship',
        count: link.count,
        kinds: link.kinds,
      }))
      return { edges, positions: clusterPositions }
    }
    if (!interClusterOnly) return { edges: visibleEdges, positions }
    return { edges: visibleEdges.filter((edge) => clusterLayout.membership.get(edge.source) !== clusterLayout.membership.get(edge.target)), positions }
  }, [clusterLayout.clusters, clusterLayout.links, clusterLayout.membership, edgeView, interClusterOnly, positions, visibleEdges])

  return (
    <Canvas key={`${layout.graph_snapshot_id}:${viewMode}:${zoom}`} camera={cameraProps}>
      <color attach="background" args={['#070709']} />
      <ambientLight intensity={0.7} />
      <pointLight position={[10, 10, 10]} intensity={0.6} />
      {viewMode === 'cluster' && <ClusterHulls clusters={clusterLayout.clusters} positions={positions} identities={clusterIdentities} selectedId={selectedClusterId} onSelect={onSelectCluster} onEnter={onEnterCluster} />}
      <EdgeLines edges={edgeRendering.edges} positions={edgeRendering.positions} selectedId={edgeView === 'clusters' ? selectedClusterId : selectedId} />
      <NodeCloud
        nodes={layout.nodes}
        positions={positions}
        selectedId={selectedId}
        onSelect={onSelect}
        onDragEnd={dragPositions.setOverride}
        metricValues={metricValues}
        emphasizedIds={selectedClusterMembers}
      />
      <NodeLabels nodes={layout.nodes} positions={positions} selectedId={selectedId} entryPointIds={entryPointIds} clusterMemberIds={selectedClusterMembers} onSelect={onSelect} onFocus={onFocusNode} />
      <OrbitControls makeDefault target={target} enableDamping dampingFactor={0.15} />
    </Canvas>
  )
}
