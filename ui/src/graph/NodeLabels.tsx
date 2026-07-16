import { Html } from '@react-three/drei'
import type { PositionedNode } from './types'
import { chooseImportantNodes } from './nodeLabelModel'

export function NodeLabels({ nodes, positions, selectedId, entryPointIds = new Set(), clusterMemberIds, onSelect, onFocus }: { nodes: PositionedNode[]; positions: Map<string, [number, number, number]>; selectedId: string | null; entryPointIds?: Set<string>; clusterMemberIds?: Set<string>; onSelect: (node: PositionedNode) => void; onFocus?: (node: PositionedNode) => void }) {
  const important = chooseImportantNodes(nodes, selectedId, entryPointIds, clusterMemberIds, positions)
  return <>{important.map((node) => {
    const position = positions.get(node.id)
    if (!position) return null
    return <Html key={node.id} position={[position[0], position[1] + 0.19, position[2]]} center zIndexRange={[12, 0]}>
      <button type="button" className="node-graph-label" data-visual-role="node-label" data-selected={node.id === selectedId} title={`${node.label} · ${node.file_path ?? node.id}`} onClick={(event) => { event.stopPropagation(); onSelect(node) }} onDoubleClick={(event) => { event.stopPropagation(); onFocus?.(node) }}>
        <strong>{displayName(node)}</strong><span>{humanKind(node.label)}</span>
      </button>
    </Html>
  })}</>
}

function displayName(node: PositionedNode): string {
  const value = node.name.split('::').at(-1) ?? node.name
  return value.split('/').at(-1) ?? value
}

function humanKind(value: string): string { return value.replace(/([a-z])([A-Z])/g, '$1 $2') }
