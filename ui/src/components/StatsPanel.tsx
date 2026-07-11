import type { LayoutResult } from '../graph/types'

export interface StatsPanelProps {
  layout: LayoutResult
}

/**
 * Read-only rollups over the currently rendered layout slice: edge-kind
 * breakdown and the most-connected nodes. Complements the node-kind
 * legend (a filter control) with information that has no control attached
 * to it -- the "stats" half of the prototype's Filters/Stats tab split.
 */
export function StatsPanel({ layout }: StatsPanelProps) {
  const edgeKindCounts = new Map<string, number>()
  for (const edge of layout.edges) {
    edgeKindCounts.set(edge.kind, (edgeKindCounts.get(edge.kind) ?? 0) + 1)
  }
  const edgeKinds = [...edgeKindCounts.entries()].sort((a, b) => b[1] - a[1])

  const topByDegree = [...layout.nodes]
    .sort((a, b) => b.in_degree + b.out_degree - (a.in_degree + a.out_degree))
    .slice(0, 5)

  return (
    <div className="flex flex-col gap-4 p-3">
      <section>
        <h2
          className="mb-2 text-[9.5px] font-bold tracking-wide uppercase"
          style={{ color: 'var(--atlas-text-dim)' }}
        >
          Edge kinds
        </h2>
        {edgeKinds.length === 0 ? (
          <p className="text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>
            No edges in the current view.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {edgeKinds.map(([kind, count]) => (
              <li key={kind} className="flex items-center gap-2 px-0.5 py-1">
                <span
                  className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px]"
                  style={{ color: 'var(--atlas-text-bright)' }}
                >
                  {kind}
                </span>
                <span className="text-[10px]" style={{ color: 'var(--atlas-text-dim)' }}>
                  {count}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>

      <section>
        <h2
          className="mb-2 text-[9.5px] font-bold tracking-wide uppercase"
          style={{ color: 'var(--atlas-text-dim)' }}
        >
          Most connected
        </h2>
        {topByDegree.length === 0 ? (
          <p className="text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>
            No nodes in the current view.
          </p>
        ) : (
          <ul className="flex flex-col gap-0.5">
            {topByDegree.map((node) => (
              <li key={node.id} className="flex items-center gap-2 px-0.5 py-1">
                <span
                  className="flex-1 overflow-hidden text-ellipsis whitespace-nowrap text-[11px]"
                  style={{ color: 'var(--atlas-text-bright)' }}
                >
                  {node.name}
                </span>
                <span className="text-[10px]" style={{ color: 'var(--atlas-text-dim)' }}>
                  {node.in_degree + node.out_degree}
                </span>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  )
}
