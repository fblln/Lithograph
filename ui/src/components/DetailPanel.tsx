import type { PositionedNode } from '../graph/types'
import { colorForLabel } from '../graph/palette'

export interface DetailPanelProps {
  node: PositionedNode | null
  onFocus: (node: PositionedNode) => void
  onClear: () => void
}

export function DetailPanel({ node, onFocus, onClear }: DetailPanelProps) {
  return (
    <aside
      className="flex w-72 flex-none flex-col overflow-y-auto border-l p-3"
      style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-surface)' }}
    >
      <h2
        className="mb-2 text-[9.5px] font-bold tracking-wide uppercase"
        style={{ color: 'var(--atlas-text-dim)' }}
      >
        Selected node
      </h2>
      {node === null ? (
        <p className="text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>
          Click a node in the graph to inspect it.
        </p>
      ) : (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <span
              className="h-2.5 w-2.5 flex-none rounded-full"
              style={{ background: colorForLabel(node.label) }}
            />
            <span className="text-[12px] font-semibold" style={{ color: 'var(--atlas-text-bright)' }}>
              {node.name}
            </span>
          </div>
          <dl className="flex flex-col gap-1 text-[11px]">
            <Row label="Kind" value={node.label} />
            <Row label="Id" value={node.id} mono />
            {node.file_path && <Row label="Path" value={node.file_path} mono />}
            <Row label="In / out degree" value={`${node.in_degree} / ${node.out_degree}`} />
            <Row label="Hop from focus" value={String(node.hop)} />
          </dl>
          <div className="mt-2 flex gap-2">
            <button
              type="button"
              onClick={() => onFocus(node)}
              className="cursor-pointer rounded px-2.5 py-1 text-[10.5px] font-semibold"
              style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-text-bright)' }}
            >
              Focus here
            </button>
            <button
              type="button"
              onClick={onClear}
              className="cursor-pointer rounded px-2.5 py-1 text-[10.5px] font-semibold"
              style={{ background: 'transparent', color: 'var(--atlas-text-muted)' }}
            >
              Clear
            </button>
          </div>
        </div>
      )}
    </aside>
  )
}

function Row({ label, value, mono = false }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-[9.5px] uppercase" style={{ color: 'var(--atlas-text-faint)' }}>
        {label}
      </dt>
      <dd
        className="overflow-hidden text-ellipsis"
        style={{
          color: 'var(--atlas-text-muted)',
          fontFamily: mono ? 'ui-monospace, Menlo, monospace' : undefined,
        }}
      >
        {value}
      </dd>
    </div>
  )
}
