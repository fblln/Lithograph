import { useMemo } from 'react'
import * as THREE from 'three'
import type { LayoutEdge } from './types'
import { edgeFadeOpacity } from './positions'

export interface EdgeLinesProps {
  edges: LayoutEdge[]
  positions: Map<string, [number, number, number]>
}

const EDGE_COLOR = new THREE.Color('#3a3f4d')

/**
 * All edges as one `LineSegments` over a single `BufferGeometry`: one
 * draw call regardless of edge count. Opacity fades as edge count grows
 * (count-aware fading, per decision-1) since edges -- not nodes -- are
 * what first turns a graph into a solid unreadable mass as it gets denser.
 * Takes a resolved `positions` map (radial or matrix, with any drag
 * overrides already applied) rather than computing positions itself, so
 * edges always track wherever their endpoints actually are.
 */
export function EdgeLines({ edges, positions }: EdgeLinesProps) {
  const geometry = useMemo(() => {
    const flatPositions = new Float32Array(edges.length * 6)
    let written = 0
    for (const edge of edges) {
      const source = positions.get(edge.source)
      const target = positions.get(edge.target)
      if (!source || !target) continue
      flatPositions.set([...source, ...target], written * 6)
      written += 1
    }
    const buffer = new THREE.BufferGeometry()
    buffer.setAttribute(
      'position',
      new THREE.BufferAttribute(flatPositions.subarray(0, written * 6), 3),
    )
    return buffer
  }, [edges, positions])

  const opacity = edgeFadeOpacity(edges.length)

  if (edges.length === 0) return null

  return (
    <lineSegments geometry={geometry}>
      <lineBasicMaterial color={EDGE_COLOR} transparent opacity={opacity} />
    </lineSegments>
  )
}
