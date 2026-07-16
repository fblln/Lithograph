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

  it('skips labels crowding an already-labelled position, keeping the selected node', () => {
    // The live polyglot failure signature: the highest-priority nodes all sit
    // in one dense region, so priority-only selection stacked their labels.
    const positions = new Map<string, [number, number, number]>(
      nodes.map((node, index) => {
        const crowdedTopPriority = index >= 20
        return [node.id, crowdedTopPriority ? [0, 0, index * 0.01] : [index * 10, 0, index * 10]]
      }),
    )
    const result = chooseImportantNodes(nodes, 'n28', new Set(), undefined, positions)

    const crowded = result.filter((node) => Number(node.id.slice(1)) >= 20)
    expect(crowded.map((node) => node.id)).toContain('n28')
    expect(crowded.length).toBeLessThanOrEqual(2)
    expect(result.length).toBeGreaterThan(2)
    // Deterministic: same inputs, same picks.
    expect(chooseImportantNodes(nodes, 'n28', new Set(), undefined, positions)).toEqual(result)
  })
})
