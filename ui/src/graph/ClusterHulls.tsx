import { Html } from '@react-three/drei'
import { useMemo, useState } from 'react'
import * as THREE from 'three'
import type { ClusterIdentity } from '../clusterIdentity'
import type { VisualCluster } from './clusterLayout'
import { adaptiveHull2D } from './adaptiveHull'

export interface ClusterHullsProps {
  clusters: VisualCluster[]
  positions: Map<string, [number, number, number]>
  identities?: Map<string, ClusterIdentity>
  selectedId?: string | null
  onSelect?: (cluster: VisualCluster) => void
  onEnter?: (cluster: VisualCluster) => void
}

const FLOOR_OFFSET = 0.055

interface HullEntry {
  cluster: VisualCluster
  shape: THREE.Shape
  outline: THREE.BufferGeometry
  labelPosition: [number, number, number]
  y: number
  color: string
}

/** Adaptive, selectable cluster regions driven by the same visual membership
 * as the force layout. Even empty and partial analytical clusters retain a
 * labeled body so overview completeness is explicit. */
export function ClusterHulls({ clusters, positions, identities, selectedId, onSelect, onEnter }: ClusterHullsProps) {
  const [hoveredId, setHoveredId] = useState<string | null>(null)
  const hulls = useMemo<HullEntry[]>(() => clusters.map((cluster) => {
    const worldPositions = cluster.members
      .map((memberId) => positions.get(memberId))
      .filter((position): position is [number, number, number] => position !== undefined)
    const flatPoints: [number, number][] = worldPositions.length > 0
      ? worldPositions.map(([x, , z]) => [x, z])
      : [[cluster.center[0], cluster.center[2]]]
    const hullPoints = adaptiveHull2D(flatPoints, Math.max(0.22, Math.min(0.55, cluster.radius * 0.22)))
    const shape = new THREE.Shape()
    shape.moveTo(hullPoints[0][0], hullPoints[0][1])
    for (let index = 1; index < hullPoints.length; index += 1) shape.lineTo(hullPoints[index][0], hullPoints[index][1])
    shape.closePath()
    const outlinePoints = [...hullPoints, hullPoints[0]].map(([x, y]) => new THREE.Vector3(x, y, 0))
    const labelAnchor = [...hullPoints].sort((a, b) => b[1] - a[1] || a[0] - b[0])[0]
    return {
      cluster,
      shape,
      outline: new THREE.BufferGeometry().setFromPoints(outlinePoints),
      labelPosition: [labelAnchor[0], 0.12, labelAnchor[1] + 0.12],
      y: Math.min(0, ...worldPositions.map(([, y]) => y)) - FLOOR_OFFSET,
      color: colorForClusterId(cluster.id),
    }
  }), [clusters, positions])

  if (hulls.length === 0) return null

  return <>{hulls.map((hull) => {
    const identity = identities?.get(hull.cluster.id)
    const selected = hull.cluster.id === selectedId
    const hovered = hull.cluster.id === hoveredId
    const prominent = selected || hovered
    return <group key={hull.cluster.id}>
      <mesh
        position={[0, hull.y, 0]}
        rotation={[Math.PI / 2, 0, 0]}
        onClick={(event) => { event.stopPropagation(); onSelect?.(hull.cluster) }}
        onDoubleClick={(event) => { event.stopPropagation(); onEnter?.(hull.cluster) }}
        onPointerEnter={() => setHoveredId(hull.cluster.id)}
        onPointerLeave={() => setHoveredId(null)}
      >
        <shapeGeometry args={[hull.shape]} />
        <meshBasicMaterial color={hull.color} transparent opacity={prominent ? 0.25 : 0.11} side={THREE.DoubleSide} depthWrite={false} />
      </mesh>
      <lineLoop geometry={hull.outline} position={[0, hull.y - 0.002, 0]} rotation={[Math.PI / 2, 0, 0]}>
        <lineBasicMaterial color={hull.color} transparent opacity={prominent ? 0.95 : 0.48} />
      </lineLoop>
      <Html position={hull.labelPosition} center zIndexRange={[20, 0]}>
        <button
          type="button"
          className="cluster-region-label"
          data-visual-role="cluster-label"
          data-selected={selected}
          aria-label={`${identity?.name ?? hull.cluster.id} cluster${identity?.partial ? ', partially rendered' : ''}`}
          title={identity?.responsibility ?? hull.cluster.id}
          onClick={(event) => { event.stopPropagation(); onSelect?.(hull.cluster) }}
          onDoubleClick={(event) => { event.stopPropagation(); onEnter?.(hull.cluster) }}
          onFocus={() => setHoveredId(hull.cluster.id)}
          onBlur={() => setHoveredId(null)}
        >
          <strong>{identity?.name ?? hull.cluster.id}</strong>
          <span>{identity ? `${identity.visibleMemberCount}/${identity.memberCount} nodes` : `${hull.cluster.members.length} nodes`}</span>
          {identity?.tensionCount ? <span data-severity={identity.highestSeverity?.toLowerCase()}>{identity.tensionCount} attention</span> : null}
        </button>
      </Html>
    </group>
  })}</>
}

function hashString(value: string): number {
  let hash = 5381
  for (let index = 0; index < value.length; index += 1) hash = (hash * 33) ^ value.charCodeAt(index)
  return hash >>> 0
}

function colorForClusterId(id: string): string {
  const hues = [188, 207, 226, 265, 302, 338, 30, 58, 104, 145]
  return `hsl(${hues[hashString(id) % hues.length]}, 58%, 57%)`
}
