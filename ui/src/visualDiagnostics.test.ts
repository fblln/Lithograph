import { describe, expect, it } from 'vitest'
import { analyzeVisualGeometry, type VisualElementGeometry, type VisualRect, type VisualRole } from './visualDiagnostics'

const rect = (left: number, top: number, width: number, height: number): VisualRect => ({ left, top, right: left + width, bottom: top + height, width, height })
const item = (role: VisualRole, geometry: VisualRect, unscaledHeight = geometry.height, label?: string): VisualElementGeometry => ({ role, rect: geometry, unscaledWidth: geometry.width, unscaledHeight, label })

describe('visual readability diagnostics', () => {
  it('reports the scale, clipping, collision, sparse framing, and overlay signature visible in the realistic screenshot', () => {
    const result = analyzeVisualGeometry([
      item('graph-viewport', rect(0, 0, 1440, 900)),
      item('cluster-label', rect(-40, 180, 150, 45), 45, 'clipped cluster'),
      item('cluster-label', rect(650, 190, 105, 32), 32, 'distant cluster'),
      item('cluster-label', rect(180, 580, 260, 120), 40, 'near cluster'),
      item('node-label', rect(700, 560, 100, 48), 48, 'node one'),
      item('node-label', rect(720, 575, 100, 48), 48, 'node two'),
      item('graph-overlay', rect(0, 610, 430, 290)),
    ])

    expect(result.clippedClusterLabels).toEqual(['clipped cluster'])
    expect(result.clusterLabelScaleRatio).toBe(3)
    expect(result.overlappingNodeLabelRatio).toBe(1)
    expect(result.issues).toEqual(expect.arrayContaining([
      expect.stringContaining('cluster label(s) are clipped'),
      expect.stringContaining('perspective scale ratio'),
      expect.stringContaining('node labels overlap'),
    ]))
  })

  it('accepts a balanced, screen-sized layout', () => {
    const result = analyzeVisualGeometry([
      item('graph-viewport', rect(0, 0, 1200, 800)),
      item('cluster-label', rect(100, 100, 160, 52), 52),
      item('cluster-label', rect(520, 320, 170, 52), 52),
      item('cluster-label', rect(900, 620, 160, 52), 52),
      item('node-label', rect(300, 260, 90, 40), 40),
      item('node-label', rect(720, 430, 90, 40), 40),
      item('graph-overlay', rect(12, 620, 240, 150)),
    ])

    expect(result.issues).toEqual([])
  })
})
