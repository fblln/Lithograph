import type { PositionedNode } from '../graph/types'
import type { NodeDetail, RelatedNode, RelatedRelation } from '../api/nodeDetail'
import type { ReactNode } from 'react'
import { colorForLabel } from '../graph/palette'
import { WorkspacePanel } from './WorkspacePanel'
import { ProvenanceTags } from './ProvenanceTags'
import { stripDisplayRoot } from '../displayRoot'

export interface DetailPanelProps {
  node: PositionedNode | null
  detail: NodeDetail | null
  detailError: string | null
  onFocus: (node: PositionedNode) => void
  onClear: () => void
  displayRootPrefix?: string
}

export function DetailPanel({ node, detail, detailError, onFocus, onClear, displayRootPrefix }: DetailPanelProps) {
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
            <span className="text-[12px] font-semibold" title={node.name} style={{ color: 'var(--atlas-text-bright)' }}>
              {stripDisplayRoot(node.name, displayRootPrefix)}
            </span>
          </div>
          <dl className="flex flex-col gap-1 text-[11px]">
            <Row label="Kind" value={node.label} />
            <Row label="Id" value={node.id} mono />
            {node.file_path && <Row label="Path" value={stripDisplayRoot(node.file_path, displayRootPrefix)} title={node.file_path} mono />}
            <Row label="In / out degree" value={`${node.in_degree} / ${node.out_degree}`} />
            <Row label="Hop from focus" value={String(node.hop)} />
          </dl>
          {detail === null && detailError === null && <PanelNote>Loading evidence…</PanelNote>}
          {detailError !== null && <PanelNote>Evidence unavailable: {detailError}</PanelNote>}
          {detail !== null && <EvidenceDetail detail={detail} displayRootPrefix={displayRootPrefix} />}
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

function EvidenceDetail({ detail, displayRootPrefix }: { detail: NodeDetail; displayRootPrefix?: string }) {
  const evidence = detail.evidence?.[0]
  return (
    <div className="flex flex-col gap-3 text-[11px]">
      {evidence && <Row label="Evidence" value={`${stripDisplayRoot(evidence.path, displayRootPrefix)}${evidence.start_line ? `:${evidence.start_line}-${evidence.end_line ?? evidence.start_line}` : ''}`} title={evidence.path} mono />}
      <Section title="Source excerpt">
      {detail.source?.status === 'available' && detail.source.text ? <pre className="max-h-56 overflow-auto rounded p-2 text-[10px]" style={{ background: 'var(--atlas-canvas)', color: 'var(--atlas-text-muted)' }}>{detail.source.text}</pre> : <PanelNote>{detail.source?.message ?? 'No source excerpt is available.'}</PanelNote>}
      </Section>
      <Related title="Definitions" nodes={detail.definitions ?? []} displayRootPrefix={displayRootPrefix} />
      <References relations={detail.references ?? []} displayRootPrefix={displayRootPrefix} />
      <Related title="Related documentation" nodes={detail.related_docs ?? []} displayRootPrefix={displayRootPrefix} />
      <ProvenanceTags tags={detail.tags ?? []} label="Node provenance tags" />
    </div>
  )
}

function Section({ title, children }: { title: string; children: ReactNode }) {
  return <section><h3 className="mb-1 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>{title}</h3>{children}</section>
}

function PanelNote({ children }: { children: ReactNode }) {
  return <p className="text-[11px]" style={{ color: 'var(--atlas-text-muted)' }}>{children}</p>
}

function Related({ title, nodes, displayRootPrefix }: { title: string; nodes: RelatedNode[]; displayRootPrefix?: string }) {
  if (nodes.length === 0) return null
  return <Section title={title}><ul className="flex flex-col gap-1">{nodes.map((node) => <li key={node.id}><span title={node.name} style={{ color: 'var(--atlas-text-bright)' }}>{stripDisplayRoot(node.name, displayRootPrefix)}</span> <span style={{ color: 'var(--atlas-text-faint)' }}>({node.label})</span></li>)}</ul></Section>
}

function References({ relations, displayRootPrefix }: { relations: RelatedRelation[]; displayRootPrefix?: string }) {
  if (relations.length === 0) return null
  return <Section title="References"><ul className="flex flex-col gap-1">{relations.map((relation) => <li key={relation.id}><span style={{ color: 'var(--atlas-text-bright)' }}>{relation.direction} {relation.kind}</span> <span title={relation.counterpart.name}>{stripDisplayRoot(relation.counterpart.name, displayRootPrefix)}</span>{relation.resolver_strategy && <span style={{ color: 'var(--atlas-text-faint)' }}> · {relation.resolver_strategy} · {relation.confidence}</span>}<ProvenanceTags tags={relation.tags ?? []} label={`Relation provenance tags for ${relation.id}`} /></li>)}</ul></Section>
}

function Row({ label, value, title, mono = false }: { label: string; value: string; title?: string; mono?: boolean }) {
  return (
    <div className="flex flex-col gap-0.5">
      <dt className="text-[9.5px] uppercase" style={{ color: 'var(--atlas-text-faint)' }}>
        {label}
      </dt>
      <dd
        title={title}
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
