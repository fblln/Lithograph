import { useLayoutEffect, useMemo, useRef } from 'react'
import * as THREE from 'three'
import { useThree, type ThreeEvent } from '@react-three/fiber'
import type { PositionedNode } from './types'
import { colorForLabel } from './palette'

const BASE_RADIUS = 0.12
const SELECTED_SCALE = 1.8
const DRAG_SCALE = 1.4

export interface NodeCloudProps {
  nodes: PositionedNode[]
  positions: Map<string, [number, number, number]>
  selectedId: string | null
  onSelect: (node: PositionedNode) => void
  /** Called once a drag ends, with the node's new world position. */
  onDragEnd: (nodeId: string, position: [number, number, number]) => void
  metricValues?: Map<string, number>
  emphasizedIds?: Set<string>
}

const tempObject = new THREE.Object3D()
const tempColor = new THREE.Color()
const fallbackPosition: [number, number, number] = [0, 0, 0]

/**
 * All nodes as one `InstancedMesh`: a single draw call regardless of node
 * count, the same technique noted in decision-1 as the reason graph-ui
 * renders smoothly at scale (raw per-node `<mesh>` components do not).
 *
 * Dragging: on pointer-down over an instance, a horizontal plane is fixed
 * at that node's current height and the pointer is captured; while
 * captured, the node's live instance matrix follows the pointer ray's
 * intersection with that plane (visual feedback during the drag), and
 * `onDragEnd` fires once on release with the final position -- the caller
 * (not this component) decides whether/how that override persists.
 */
export function NodeCloud({ nodes, positions, selectedId, onSelect, onDragEnd, metricValues, emphasizedIds }: NodeCloudProps) {
  const meshRef = useRef<THREE.InstancedMesh>(null)
  const { raycaster } = useThree()
  const dragPlane = useMemo(() => new THREE.Plane(new THREE.Vector3(0, 1, 0), 0), [])
  const dragState = useRef<{ nodeId: string; index: number; pointerId: number } | null>(null)

  const resolved = useMemo(
    () => nodes.map((node) => positions.get(node.id) ?? fallbackPosition),
    [nodes, positions],
  )

  useLayoutEffect(() => {
    const mesh = meshRef.current
    if (!mesh) return
    nodes.forEach((node, index) => {
      const [x, y, z] = resolved[index]
      const metric = metricValues?.get(node.id) ?? 0
      const unrelated = emphasizedIds && !emphasizedIds.has(node.id)
      const scale = node.id === selectedId ? SELECTED_SCALE : (unrelated ? 0.72 : 1 + Math.min(1, metric) * 0.8)
      tempObject.position.set(x, y, z)
      tempObject.scale.setScalar(scale)
      tempObject.updateMatrix()
      mesh.setMatrixAt(index, tempObject.matrix)
      tempColor.set(colorForLabel(node.label)).lerp(new THREE.Color('#ff6b6b'), Math.min(1, metric))
      if (unrelated) tempColor.lerp(new THREE.Color('#20232c'), 0.76)
      mesh.setColorAt(index, tempColor)
    })
    mesh.instanceMatrix.needsUpdate = true
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true
  }, [emphasizedIds, nodes, resolved, selectedId, metricValues])

  function handleClick(event: ThreeEvent<MouseEvent>) {
    event.stopPropagation()
    // A drag that moved the pointer still fires a click on release; only
    // treat this as a selection when no drag was in progress.
    if (dragState.current) return
    const index = event.instanceId
    if (index === undefined) return
    const node = nodes[index]
    if (node) onSelect(node)
  }

  function handlePointerDown(event: ThreeEvent<PointerEvent>) {
    const index = event.instanceId
    if (index === undefined) return
    const node = nodes[index]
    const mesh = meshRef.current
    if (!node || !mesh) return
    event.stopPropagation()
    const [, y] = resolved[index]
    dragPlane.constant = -y
    dragState.current = { nodeId: node.id, index, pointerId: event.pointerId }
    ;(event.target as Element).setPointerCapture(event.pointerId)
  }

  function handlePointerMove(event: ThreeEvent<PointerEvent>) {
    const drag = dragState.current
    const mesh = meshRef.current
    if (!drag || !mesh) return
    event.stopPropagation()
    const point = new THREE.Vector3()
    if (!raycaster.ray.intersectPlane(dragPlane, point)) return
    tempObject.position.copy(point)
    tempObject.scale.setScalar(DRAG_SCALE)
    tempObject.updateMatrix()
    mesh.setMatrixAt(drag.index, tempObject.matrix)
    mesh.instanceMatrix.needsUpdate = true
  }

  function handlePointerUp(event: ThreeEvent<PointerEvent>) {
    const drag = dragState.current
    if (!drag) return
    event.stopPropagation()
    ;(event.target as Element).releasePointerCapture(drag.pointerId)
    const point = new THREE.Vector3()
    dragState.current = null
    if (raycaster.ray.intersectPlane(dragPlane, point)) {
      onDragEnd(drag.nodeId, [point.x, point.y, point.z])
    }
  }

  if (nodes.length === 0) return null

  return (
    <instancedMesh
      ref={meshRef}
      args={[undefined, undefined, nodes.length]}
      onClick={handleClick}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      <sphereGeometry args={[BASE_RADIUS, 12, 12]} />
      <meshStandardMaterial />
    </instancedMesh>
  )
}
