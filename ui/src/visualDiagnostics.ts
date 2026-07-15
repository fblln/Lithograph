export type VisualRole = 'graph-viewport' | 'cluster-label' | 'node-label' | 'graph-overlay'

export interface VisualRect {
  left: number
  top: number
  right: number
  bottom: number
  width: number
  height: number
}

export interface VisualElementGeometry {
  role: VisualRole
  rect: VisualRect
  unscaledWidth: number
  unscaledHeight: number
  label?: string
}

export interface VisualDiagnostics {
  viewport: VisualRect
  clusterLabelCount: number
  nodeLabelCount: number
  clippedClusterLabels: string[]
  clippedNodeLabels: string[]
  clusterLabelScaleRatio: number
  nodeLabelScaleRatio: number
  overlappingNodeLabelRatio: number
  clusterSpreadRatio: number
  overlayOcclusionRatio: number
  issues: string[]
}

const OVERLAP_AREA_THRESHOLD = 0.18

/**
 * Browser-independent readability oracle. It intentionally measures
 * projected screen geometry rather than WebGL pixels, so failures explain
 * whether the problem is clipping, perspective scale, collisions, sparse
 * framing, or inspector occlusion.
 */
export function analyzeVisualGeometry(elements: VisualElementGeometry[]): VisualDiagnostics {
  const viewport = elements.find((item) => item.role === 'graph-viewport')?.rect ?? emptyRect()
  const clusters = elements.filter((item) => item.role === 'cluster-label')
  const nodes = elements.filter((item) => item.role === 'node-label')
  const overlays = elements.filter((item) => item.role === 'graph-overlay')
  const clippedClusterLabels = clippedLabels(clusters, viewport)
  const clippedNodeLabels = clippedLabels(nodes, viewport)
  const clusterLabelScaleRatio = scaleRatio(clusters)
  const nodeLabelScaleRatio = scaleRatio(nodes)
  const overlappingNodeLabelRatio = overlapRatio(nodes)
  const clusterSpreadRatio = spreadRatio(clusters, viewport)
  const overlayOcclusionRatio = Math.min(1, overlays.reduce((total, item) => total + intersectionArea(item.rect, viewport), 0) / Math.max(1, area(viewport)))
  const issues: string[] = []
  if (clippedClusterLabels.length) issues.push(`${clippedClusterLabels.length} cluster label(s) are clipped by the graph viewport`)
  if (clippedNodeLabels.length / Math.max(1, nodes.length) > 0.1) issues.push(`${clippedNodeLabels.length}/${nodes.length} node labels are clipped`)
  if (clusterLabelScaleRatio > 1.35) issues.push(`cluster label perspective scale ratio is ${clusterLabelScaleRatio.toFixed(2)}× (maximum 1.35×)`)
  if (nodeLabelScaleRatio > 1.35) issues.push(`node label perspective scale ratio is ${nodeLabelScaleRatio.toFixed(2)}× (maximum 1.35×)`)
  if (overlappingNodeLabelRatio > 0.25) issues.push(`${Math.round(overlappingNodeLabelRatio * 100)}% of node labels overlap materially`)
  if (clusters.length > 2 && clusterSpreadRatio < 0.16) issues.push(`cluster labels occupy only ${Math.round(clusterSpreadRatio * 100)}% of the viewport span`)
  if (overlayOcclusionRatio > 0.18) issues.push(`graph overlays cover ${Math.round(overlayOcclusionRatio * 100)}% of the viewport`)
  return { viewport, clusterLabelCount: clusters.length, nodeLabelCount: nodes.length, clippedClusterLabels, clippedNodeLabels, clusterLabelScaleRatio, nodeLabelScaleRatio, overlappingNodeLabelRatio, clusterSpreadRatio, overlayOcclusionRatio, issues }
}

export function collectVisualDiagnostics(root: ParentNode = document): VisualDiagnostics {
  const elements = [...root.querySelectorAll<HTMLElement>('[data-visual-role]')].map((element) => {
    const rect = toRect(element.getBoundingClientRect())
    return {
      role: element.dataset.visualRole as VisualRole,
      rect,
      unscaledWidth: element.offsetWidth || rect.width,
      unscaledHeight: element.offsetHeight || rect.height,
      label: element.getAttribute('aria-label') ?? element.textContent?.trim().slice(0, 80),
    }
  })
  return analyzeVisualGeometry(elements)
}

export function installVisualDiagnostics(): void {
  if (!new URLSearchParams(window.location.search).has('visualDiagnostics')) return
  window.__LITHOGRAPH_VISUAL_DIAGNOSTICS__ = { collect: () => collectVisualDiagnostics() }
}

declare global {
  interface Window {
    __LITHOGRAPH_VISUAL_DIAGNOSTICS__?: { collect: () => VisualDiagnostics }
  }
}

function clippedLabels(items: VisualElementGeometry[], viewport: VisualRect): string[] {
  return items.filter(({ rect }) => rect.left < viewport.left || rect.top < viewport.top || rect.right > viewport.right || rect.bottom > viewport.bottom).map((item) => item.label ?? 'unlabeled').sort()
}

function scaleRatio(items: VisualElementGeometry[]): number {
  const scales = items.map((item) => item.rect.height / Math.max(1, item.unscaledHeight)).filter((value) => Number.isFinite(value) && value > 0)
  return scales.length < 2 ? 1 : Math.max(...scales) / Math.max(0.001, Math.min(...scales))
}

function overlapRatio(items: VisualElementGeometry[]): number {
  const overlapping = new Set<number>()
  for (let left = 0; left < items.length; left += 1) for (let right = left + 1; right < items.length; right += 1) {
    const intersection = intersectionArea(items[left].rect, items[right].rect)
    if (intersection / Math.max(1, Math.min(area(items[left].rect), area(items[right].rect))) >= OVERLAP_AREA_THRESHOLD) {
      overlapping.add(left)
      overlapping.add(right)
    }
  }
  return overlapping.size / Math.max(1, items.length)
}

function spreadRatio(items: VisualElementGeometry[], viewport: VisualRect): number {
  if (!items.length) return 0
  const centers = items.map(({ rect }) => [(rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2] as const)
  const width = Math.max(...centers.map(([x]) => x)) - Math.min(...centers.map(([x]) => x))
  const height = Math.max(...centers.map(([, y]) => y)) - Math.min(...centers.map(([, y]) => y))
  return (width * height) / Math.max(1, area(viewport))
}

function intersectionArea(left: VisualRect, right: VisualRect): number {
  return Math.max(0, Math.min(left.right, right.right) - Math.max(left.left, right.left)) * Math.max(0, Math.min(left.bottom, right.bottom) - Math.max(left.top, right.top))
}

function area(rect: VisualRect): number { return Math.max(0, rect.width) * Math.max(0, rect.height) }
function emptyRect(): VisualRect { return { left: 0, top: 0, right: 0, bottom: 0, width: 0, height: 0 } }
function toRect(rect: DOMRect): VisualRect { return { left: rect.left, top: rect.top, right: rect.right, bottom: rect.bottom, width: rect.width, height: rect.height } }
