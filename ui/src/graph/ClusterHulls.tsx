import { useMemo } from 'react'
import * as THREE from 'three'
import type { ArchitectureCluster } from '../api/architecture'
import { convexHull2D } from './convexHull'

export interface ClusterHullsProps {
  clusters: ArchitectureCluster[]
  /** Resolved node positions (radial or matrix, with drag overrides
   * applied) -- the same map GraphScene hands to NodeCloud/EdgeLines, so
   * hulls always track wherever their members actually are, in whichever
   * view mode is active. */
  positions: Map<string, [number, number, number]>
}

const MIN_MEMBERS = 3
const FLOOR_OFFSET = 0.05
const OPACITY_MIN = 0.12
const OPACITY_MAX = 0.18

interface HullEntry {
  id: string
  shape: THREE.Shape
  y: number
  color: string
  opacity: number
}

/**
 * Deterministic string hash (djb2) so the same cluster id always maps to
 * the same hue across renders/reloads, without needing to persist a
 * color assignment anywhere.
 */
function hashString(value: string): number {
  let hash = 5381
  for (let i = 0; i < value.length; i++) {
    hash = (hash * 33) ^ value.charCodeAt(i)
  }
  return hash >>> 0
}

function colorForClusterId(id: string): string {
  const hue = hashString(id) % 360
  return `hsl(${hue}, 65%, 55%)`
}

/**
 * Translucent flat polygons under each architecture cluster's members,
 * so users can see functional-community boundaries at a glance without
 * the wash ever competing with node/edge rendering for depth. One
 * `<mesh>` per cluster (cluster counts are small relative to node/edge
 * counts, so this doesn't need instancing the way NodeCloud/EdgeLines do).
 */
export function ClusterHulls({ clusters, positions }: ClusterHullsProps) {
  const hulls = useMemo<HullEntry[]>(() => {
    const entries: HullEntry[] = []

    for (const cluster of clusters) {
      if (cluster.members.length < MIN_MEMBERS) continue

      const worldPositions = cluster.members
        .map((memberId) => positions.get(memberId))
        .filter((position): position is [number, number, number] => position !== undefined)

      if (worldPositions.length < MIN_MEMBERS) continue

      // Project onto the XZ ground plane -- x and z are the two axes
      // nodes actually spread across (per positions.ts); y encodes hop
      // depth, not lateral position.
      const flatPoints: [number, number][] = worldPositions.map(([x, , z]) => [x, z])
      const hullPoints = convexHull2D(flatPoints)
      if (hullPoints.length < MIN_MEMBERS) continue

      const shape = new THREE.Shape()
      shape.moveTo(hullPoints[0][0], hullPoints[0][1])
      for (let i = 1; i < hullPoints.length; i++) {
        shape.lineTo(hullPoints[i][0], hullPoints[i][1])
      }
      shape.closePath()

      const minY = Math.min(...worldPositions.map(([, y]) => y))

      entries.push({
        id: cluster.id,
        shape,
        y: minY - FLOOR_OFFSET,
        color: colorForClusterId(cluster.id),
        opacity: OPACITY_MIN + cluster.cohesion * (OPACITY_MAX - OPACITY_MIN),
      })
    }

    return entries
  }, [clusters, positions])

  if (hulls.length === 0) return null

  return (
    <>
      {hulls.map((hull) => (
        // ShapeGeometry builds its triangles in the shape's local XY
        // plane; rotating +90deg around X maps local (x, y) to world
        // (x, 0, y), i.e. flat onto the XZ ground plane nodes sit on
        // with no mirroring (verified against Three's actual rotation
        // matrix: Rx(+90deg) sends local +Y to world +Z directly, whereas
        // -90deg would flip it to world -Z and mirror every hull).
        <mesh key={hull.id} position={[0, hull.y, 0]} rotation={[Math.PI / 2, 0, 0]}>
          <shapeGeometry args={[hull.shape]} />
          <meshBasicMaterial
            color={hull.color}
            transparent
            opacity={hull.opacity}
            side={THREE.DoubleSide}
            depthWrite={false}
          />
        </mesh>
      ))}
    </>
  )
}
