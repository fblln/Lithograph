import { useLayoutEffect, useMemo, useRef } from 'react'
import * as THREE from 'three'
import type { LayoutEdge } from './types'
import { edgeFadeOpacity } from './positions'
import { EDGE_RESOLUTION_STYLE, edgeResolution, type EdgeResolution } from './edgeProvenance'

export interface EdgeLinesProps {
  edges: LayoutEdge[]
  positions: Map<string, [number, number, number]>
  selectedId?: string | null
}

const DIM_COLOR = new THREE.Color('#242834')
const Y_AXIS = new THREE.Vector3(0, 1, 0)
const tempObject = new THREE.Object3D()
const tempColor = new THREE.Color()

/** Directional, relationship-aware edge rendering. Lines remain batched in
 * one geometry, while arrowheads are one instanced mesh, so direction and
 * kind remain legible without giving up the existing graph budget. */
export function EdgeLines({ edges, positions, selectedId = null }: EdgeLinesProps) {
  const rendered = useMemo(() => edges.flatMap((edge) => {
    const source = positions.get(edge.source)
    const target = positions.get(edge.target)
    if (!source || !target) return []
    const related = !selectedId || edge.source === selectedId || edge.target === selectedId
    return [{ edge, source, target, related }]
  }), [edges, positions, selectedId])

  const groups = useMemo(() => (Object.keys(EDGE_RESOLUTION_STYLE) as EdgeResolution[]).map((resolution) => ({ resolution, rendered: rendered.filter(({ edge }) => edgeResolution(edge) === resolution) })).filter((group) => group.rendered.length > 0), [rendered])
  const opacity = selectedId ? 0.88 : Math.max(0.24, edgeFadeOpacity(edges.length))
  if (rendered.length === 0) return null

  return <>
    {groups.map((group) => <ResolutionLines key={group.resolution} resolution={group.resolution} rendered={group.rendered} opacity={opacity} />)}
    <Arrowheads rendered={rendered} selectedId={selectedId} />
  </>
}

function ResolutionLines({ resolution, rendered, opacity }: { resolution: EdgeResolution; rendered: Array<{ edge: LayoutEdge; source: [number, number, number]; target: [number, number, number]; related: boolean }>; opacity: number }) {
  const lineRef = useRef<THREE.LineSegments>(null)
  const geometry = useMemo(() => {
    const flatPositions = new Float32Array(rendered.length * 6)
    const flatColors = new Float32Array(rendered.length * 6)
    rendered.forEach(({ edge, source, target, related }, index) => {
      flatPositions.set([...source, ...target], index * 6)
      const color = related ? edgeColor(edge.kind) : DIM_COLOR
      flatColors.set([color.r, color.g, color.b, color.r, color.g, color.b], index * 6)
    })
    const buffer = new THREE.BufferGeometry()
    buffer.setAttribute('position', new THREE.BufferAttribute(flatPositions, 3))
    buffer.setAttribute('color', new THREE.BufferAttribute(flatColors, 3))
    return buffer
  }, [rendered])
  useLayoutEffect(() => { lineRef.current?.computeLineDistances?.() }, [geometry])
  const style = EDGE_RESOLUTION_STYLE[resolution]
  return <lineSegments ref={lineRef} geometry={geometry} userData={{ resolution, label: style.label }}>
    {resolution === 'HybridResolved'
      ? <lineBasicMaterial vertexColors transparent opacity={opacity * style.opacity} />
      : <lineDashedMaterial vertexColors transparent opacity={opacity * style.opacity} dashSize={style.dashSize} gapSize={style.gapSize} />}
  </lineSegments>
}

function Arrowheads({ rendered, selectedId }: { rendered: Array<{ edge: LayoutEdge; source: [number, number, number]; target: [number, number, number]; related: boolean }>; selectedId: string | null }) {
  const meshRef = useRef<THREE.InstancedMesh>(null)
  useLayoutEffect(() => {
    const mesh = meshRef.current
    if (!mesh) return
    rendered.forEach(({ edge, source, target, related }, index) => {
      const start = new THREE.Vector3(...source)
      const end = new THREE.Vector3(...target)
      const direction = end.clone().sub(start)
      const length = direction.length()
      if (length < 1e-6) return
      direction.normalize()
      tempObject.position.copy(start.lerp(end, 0.76))
      tempObject.quaternion.setFromUnitVectors(Y_AXIS, direction)
      const weight = Math.min(1.8, 0.78 + Math.log2(edge.count ?? 1) * 0.18)
      tempObject.scale.set(weight, weight, weight)
      tempObject.updateMatrix()
      mesh.setMatrixAt(index, tempObject.matrix)
      mesh.setColorAt(index, tempColor.copy(related ? edgeColor(edge.kind) : DIM_COLOR))
    })
    mesh.instanceMatrix.needsUpdate = true
    if (mesh.instanceColor) mesh.instanceColor.needsUpdate = true
  }, [rendered])
  return <instancedMesh ref={meshRef} args={[undefined, undefined, rendered.length]}>
    <coneGeometry args={[0.045, 0.13, 6]} />
    <meshBasicMaterial transparent opacity={selectedId ? 0.92 : 0.7} />
  </instancedMesh>
}

function edgeColor(kind: string): THREE.Color {
  const normalized = kind.toLowerCase()
  if (normalized.includes('call') || normalized.includes('run')) return new THREE.Color('#f1a65a')
  if (normalized.includes('import') || normalized.includes('depend') || normalized.includes('use')) return new THREE.Color('#6ea8fe')
  if (normalized.includes('data') || normalized.includes('read') || normalized.includes('write')) return new THREE.Color('#64d8b1')
  if (normalized.includes('contain') || normalized.includes('member') || normalized.includes('belong') || normalized.includes('has')) return new THREE.Color('#a998dc')
  if (normalized.includes('config') || normalized.includes('env') || normalized.includes('bind')) return new THREE.Color('#e3cf68')
  return new THREE.Color('#788195')
}
