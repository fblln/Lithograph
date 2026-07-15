import type { PositionedNode } from '../graph/types'
import type { NodeDetail, RelatedNode, RelatedRelation } from '../api/nodeDetail'
import type { ReactNode } from 'react'
import { colorForLabel } from '../graph/palette'
import { WorkspacePanel } from './WorkspacePanel'

export interface DetailPanelProps {
  node: PositionedNode | null
  detail: NodeDetail | null
  detailError: string | null
  onFocus: (node: PositionedNode) => void
  onClear: () => void
}

export function DetailPanel({ node, detail, detailError, onFocus, onClear }: DetailPanelProps) {
  return (
    <WorkspacePanel title="Selected node" side="right">
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
          {detail === null && detailError === null && <PanelNote>Loading evidence…</PanelNote>}
          {detailError !== null && <PanelNote>Evidence unavailable: {detailError}</PanelNote>}
          {detail !== null && <EvidenceDetail detail={detail} />}
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
    </WorkspacePanel>
  )
}

function EvidenceDetail({ detail }: { detail: NodeDetail }) {
  const evidence = detail.evidence?.[0]
  return (
    <div className="flex flex-col gap-3 text-[11px]">
      {evidence && <Row label="Evidence" value={`${evidence.path}${evidence.start_line ? `:${evidence.start_line}-${evidence.end_line ?? evidence.start_line}` : ''}`} mono />}
      <Section title="Source excerpt">
      {detail.source?.status === 'available' && detail.source.text ? <pre className="max-h-56 overflow-auto rounded p-2 text-[10px]" style={{ background: 'var(--atlas-canvas)', color: 'var(--atlas-text-muted)' }}>{detail.source.text}</pre> : <PanelNote>{detail.source?.message ?? 'No source excerpt is available.'}</PanelNote>}
      </Section>
      <Related title="Definitions" nodes={detail.definitions ?? []} />
      <References relations={detail.references ?? []} />
      <Related title="Related documentation" nodes={detail.related_docs ?? []} />
      {(detail.tags ?? []).length > 0 && <Section title="Tags"><ul className="flex flex-col gap-1">{detail.tags.map((tag) => <li key={tag.id}><span style={{ color: 'var(--atlas-accent)' }}>#{tag.namespace}/{tag.value}</span> <span style={{ color: 'var(--atlas-text-faint)' }}>· {tag.source} · {tag.confidence}{tag.inherited_from ? ' · inherited' : ''}</span></li>)}</ul></Section>}
    </div>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section><h3 className="mb-1 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>{title}</h3>{children}</section>
}

function PanelNote({ children }: { children: ReactNode }) {
  return <p className="text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>{children}</p>
}

function Related({ title, nodes }: { title: string; nodes: RelatedNode[] }) {
  if (nodes.length === 0) return null
  return <Section title={title}><ul className="flex flex-col gap-1">{nodes.map((node) => <li key={node.id}><span style={{ color: 'var(--atlas-text-bright)' }}>{node.name}</span> <span style={{ color: 'var(--atlas-text-faint)' }}>({node.label})</span></li>)}</ul></Section>
}

function References({ relations }: { relations: RelatedRelation[] }) {
  if (relations.length === 0) return null
  return <Section title="References"><ul className="flex flex-col gap-1">{relations.map((relation) => <li key={relation.id}><span style={{ color: 'var(--atlas-text-bright)' }}>{relation.direction} {relation.kind}</span> {relation.counterpart.name}{relation.resolver_strategy && <span style={{ color: 'var(--atlas-text-faint)' }}> · {relation.resolver_strategy} · {relation.confidence}</span>}</li>)}</ul></Section>
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
