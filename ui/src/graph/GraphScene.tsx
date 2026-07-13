import { useMemo } from 'react'
import { Canvas } from '@react-three/fiber'
import { OrbitControls } from '@react-three/drei'
import type { ArchitectureCluster } from '../api/architecture'
import type { ViewMode } from '../components/ViewModeToggle'
import type { LayoutResult, PositionedNode } from './types'
import type { DragPositions } from './useDragPositions'
import { NodeCloud } from './NodeCloud'
import { EdgeLines } from './EdgeLines'
import { ClusterHulls } from './ClusterHulls'
import { nodeWorldPosition } from './positions'
import { computeMatrixPositions } from './matrixLayout'

export interface GraphSceneProps {
  layout: LayoutResult
  viewMode: ViewMode
  clusters: ArchitectureCluster[]
  selectedId: string | null
  onSelect: (node: PositionedNode) => void
  dragPositions: DragPositions
}

export function GraphScene({
  layout,
  viewMode,
  clusters,
  selectedId,
  onSelect,
  dragPositions,
}: GraphSceneProps) {
  const basePositions = useMemo(() => {
    if (viewMode === 'matrix') return computeMatrixPositions(layout.nodes)
    return new Map(layout.nodes.map((node) => [node.id, nodeWorldPosition(node)] as const))
  }, [layout.nodes, viewMode])

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

  return (
    <Canvas camera={{ position: [6, 6, 6], fov: 50 }}>
      <color attach="background" args={['#070709']} />
      <ambientLight intensity={0.7} />
      <pointLight position={[10, 10, 10]} intensity={0.6} />
      <ClusterHulls clusters={clusters} positions={positions} />
      <EdgeLines edges={layout.edges} positions={positions} />
      <NodeCloud
        nodes={layout.nodes}
        positions={positions}
        selectedId={selectedId}
        onSelect={onSelect}
        onDragEnd={dragPositions.setOverride}
      />
      <OrbitControls makeDefault enableDamping dampingFactor={0.15} />
    </Canvas>
  )
}
