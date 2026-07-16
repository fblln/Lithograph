import type { GraphTag } from '../api/tags'

export function ProvenanceTags({ tags, label = 'Provenance tags' }: { tags: GraphTag[]; label?: string }) {
  if (tags.length === 0) return null
  return <div aria-label={label} className="mt-1 flex flex-wrap gap-1">{tags.map((tag) => {
    const evidence = tag.evidence
    const provenance = [tag.source, tag.confidence, ...evidence.map((item) => `evidence: ${item}`), tag.inherited_from ? `inherited from ${tag.inherited_from}` : undefined].filter(Boolean).join(' · ')
    return <span key={tag.id} className="rounded px-1.5 py-0.5 text-[9px]" title={provenance} style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-accent)' }}>
      #{tag.namespace}/{tag.value}<span style={{ color: 'var(--atlas-text-faint)' }}> · {tag.source} · {tag.confidence}{evidence.length ? ` · evidence ${evidence.length}` : ''}{tag.inherited_from ? ' · inherited' : ''}</span>
    </span>
  })}</div>
}
