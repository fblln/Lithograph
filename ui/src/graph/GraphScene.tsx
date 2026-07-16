import { useLayoutEffect, useMemo, useRef, useState } from 'react'
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
import { aggregateEdgeConfidence, edgeResolution, filterVisibleEdges } from './edgeProvenance'

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
  showUnprovenEdges?: boolean
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
  showUnprovenEdges = true,
  clusterLinks = [],
  onFocusNode,
}: GraphSceneProps) {
  const viewportRef = useRef<HTMLDivElement>(null)
  const [viewportAspect, setViewportAspect] = useState(16 / 10)
  useLayoutEffect(() => {
    const viewport = viewportRef.current
    if (!viewport) return

    const updateAspect = (width: number, height: number) => {
      if (width <= 0 || height <= 0) return
      const next = width / height
      setViewportAspect((current) => Math.abs(current - next) < 0.001 ? current : next)
    }
    const updateFromElement = () => {
      const bounds = viewport.getBoundingClientRect()
      updateAspect(bounds.width, bounds.height)
    }

    updateFromElement()
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', updateFromElement)
      return () => window.removeEventListener('resize', updateFromElement)
    }
    const observer = new ResizeObserver((entries) => {
      const bounds = entries[0]?.contentRect
      if (bounds) updateAspect(bounds.width, bounds.height)
    })
    observer.observe(viewport)
    return () => observer.disconnect()
  }, [])

  const visibleEdges = useMemo(() => filterVisibleEdges(layout.edges, edgeKinds, showUnprovenEdges), [edgeKinds, layout.edges, showUnprovenEdges])
  const completeLinks = useMemo(() => clusterLinks.map((link) => {
    const underlying = filterVisibleEdges(link.underlying, edgeKinds, showUnprovenEdges)
    const counts = new Map<string, number>()
    for (const edge of underlying) counts.set(edge.kind, (counts.get(edge.kind) ?? 0) + 1)
    return { ...link, count: underlying.length, underlying, kinds: [...counts].map(([kind, count]) => ({ kind, count })).sort((a, b) => b.count - a.count || a.kind.localeCompare(b.kind)) }
  }).filter((link) => link.count > 0), [clusterLinks, edgeKinds, showUnprovenEdges])
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
    const frame = cameraFrameForPositions(framedPositions, viewportAspect)
    // Debug surface for the AC8 geometry oracle: lets a browser probe explain
    // a framing regression from real numbers instead of guessing at spans.
    if (typeof window !== 'undefined' && new URLSearchParams(window.location.search).has('visualDiagnostics')) {
      ;(window as unknown as Record<string, unknown>).__LITHOGRAPH_CAMERA_DEBUG__ = {
        frame,
        aspect: viewportAspect,
        zoom,
        positionCount: framedPositions.size,
      }
    }
    return {
      ...frame,
      position: frame.position.map((value, index) => frame.target[index] + (value - frame.target[index]) * zoom) as [number, number, number],
    }
  }, [clusterLayout.clusters, positions, viewMode, viewportAspect, zoom])
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
        id: `${link.source}->${link.target}`,
        source: link.source,
        target: link.target,
        kind: link.kinds[0]?.kind ?? 'Relationship',
        resolution: link.underlying.some((edge) => edgeResolution(edge) === 'Fallback') ? 'Fallback' as const : link.underlying.every((edge) => edgeResolution(edge) === 'HybridResolved') ? 'HybridResolved' as const : 'SyntaxOnly' as const,
        confidence: aggregateEdgeConfidence(link.underlying),
        resolver_strategy: null,
        count: link.count,
        kinds: link.kinds,
      }))
      return { edges, positions: clusterPositions }
    }
    if (!interClusterOnly) return { edges: visibleEdges, positions }
    return { edges: visibleEdges.filter((edge) => clusterLayout.membership.get(edge.source) !== clusterLayout.membership.get(edge.target)), positions }
  }, [clusterLayout.clusters, clusterLayout.links, clusterLayout.membership, edgeView, interClusterOnly, positions, visibleEdges])

  return (
    <div ref={viewportRef} data-testid="graph-scene-viewport" data-camera-aspect={viewportAspect} className="relative h-full w-full">
      <MutedGraphNoiseIndicator count={clusterLayout.mutedNodeCount ?? 0} />
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
    </div>
  )
}

export function MutedGraphNoiseIndicator({ count }: { count: number }) {
  if (count === 0) return null
  return <div aria-label="Suppressed graph noise" className="absolute right-3 top-3 z-10 rounded px-2 py-1 text-[9px]" style={{ background: 'var(--atlas-panel-header)', color: 'var(--atlas-text-faint)' }}>{count} unresolved/external node{count === 1 ? '' : 's'} hidden from cluster regions</div>
}
