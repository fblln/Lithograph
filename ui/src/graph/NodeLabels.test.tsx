import { describe, expect, it } from 'vitest'
import { chooseImportantNodes } from './nodeLabelModel'
import type { PositionedNode } from './types'

const nodes: PositionedNode[] = Array.from({ length: 30 }, (_, index) => ({ id: `n${index}`, label: index === 29 ? 'Command' : 'Symbol', name: `n${index}`, file_path: `src/n${index}.ts`, in_degree: index, out_degree: 0, x: 0, y: 0, hop: 0 }))

describe('chooseImportantNodes', () => {
  it('prioritizes selected nodes, entry points, semantic roles, and central nodes with a bounded default', () => {
    const result = chooseImportantNodes(nodes, 'n0', new Set(['n1']))
    expect(result).toHaveLength(12)
    expect(result.slice(0, 3).map((node) => node.id)).toEqual(['n0', 'n1', 'n29'])
  })

  it('shows more labels within a selected cluster while keeping them bounded', () => {
    expect(chooseImportantNodes(nodes, null, new Set(), new Set(nodes.map((node) => node.id)))).toHaveLength(28)
  })
})
