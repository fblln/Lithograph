import { useEffect, useMemo, useState } from 'react'
import { getGraphDocument, regenerateGraphDocument, type GraphDocumentResult, type GraphDocumentSection } from '../api/docs'
import { sectionBody } from '../docMarkdown'
import { SubsystemDocsAgent, type SubsystemAgentContext } from './SubsystemDocsAgent'

export function DocsWorkspace({ currentSnapshotId, relatedEntityId, selectedSectionId, agentContext, onSelectSection, onFocus, onTagScope = () => {} }: { currentSnapshotId?: string; relatedEntityId?: string; selectedSectionId?: string; agentContext?: SubsystemAgentContext; onSelectSection: (id: string) => void; onFocus: (id: string) => void; onTagScope?: (tag: string, nodeIds: string[]) => void }) {
  const [result, setResult] = useState<GraphDocumentResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [regenerating, setRegenerating] = useState(false)
  const [tagFilter, setTagFilter] = useState<string>()
  useEffect(() => { getGraphDocument().then(setResult, (cause: unknown) => setError(String(cause))) }, [])
  const related = useMemo(() => result?.document.sections.filter((section) => !relatedEntityId || section.affected_nodes.includes(relatedEntityId) || section.affected_edges.includes(relatedEntityId) || section.source_query_ids.includes(relatedEntityId)) ?? [], [relatedEntityId, result])
  const baseSections = relatedEntityId && related.length > 0 ? related : result?.document.sections ?? []
  const sections = tagFilter ? baseSections.filter((section) => section.tags.some((tag) => `${tag.namespace}:${tag.value}` === tagFilter)) : baseSections
  const selected = sections.find((section) => section.id === selectedSectionId) ?? sections[0]
  const availableTags = [...new Set((result?.document.sections ?? []).flatMap((section) => section.tags.map((tag) => `${tag.namespace}:${tag.value}`)))].sort()

  async function regenerate(sectionIds?: string[]) {
    setRegenerating(true)
    try { const next = await regenerateGraphDocument(sectionIds); setResult(next); setError(null) }
    catch (cause) { setError(String(cause)) }
    finally { setRegenerating(false) }
  }

  if (error) return <aside className="w-[32rem] flex-none border-l p-4 text-[11px]" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-surface)', color: 'var(--atlas-danger)' }}>Documentation unavailable: {error}</aside>
  if (!result) return <aside className="w-[32rem] flex-none border-l p-4 text-[11px]" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-surface)' }}>Loading documentation…</aside>
  const stale = result.freshness === 'stale' || (currentSnapshotId !== undefined && result.document.graph_snapshot_id !== currentSnapshotId)
  const selectedFreshness = result.section_freshness?.find((item) => item.section_id === selected?.id)

  return <aside aria-label="Docs workspace" className="flex w-[32rem] flex-none border-l" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-surface)' }}>
    <nav aria-label="Document sections" className="w-40 flex-none overflow-y-auto border-r p-2" style={{ borderColor: 'var(--atlas-border)' }}>
      <h2 className="mb-2 text-[9.5px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Sections</h2>
      {availableTags.length > 0 && <div aria-label="Document tag filters" className="mb-3"><button type="button" data-active={!tagFilter} onClick={() => setTagFilter(undefined)} className="mb-1 block text-[9px]">All tags</button>{availableTags.map((tag) => <button key={tag} type="button" data-active={tagFilter === tag} onClick={() => setTagFilter(tag)} className="mb-1 block w-full truncate text-left text-[9px]" title={tag}>#{tag}</button>)}</div>}
      {relatedEntityId && <p className="mb-2 truncate text-[9px]" title={relatedEntityId} style={{ color: 'var(--atlas-accent)' }}>Related to {relatedEntityId}</p>}
      {sections.map((section) => <button key={section.id} type="button" data-active={section.id === selected?.id} onClick={() => onSelectSection(section.id)} className="mb-1 block w-full rounded px-1.5 py-1 text-left text-[10.5px]" style={{ background: section.id === selected?.id ? 'var(--atlas-hover)' : 'transparent', color: 'var(--atlas-text-bright)' }}>{section.title}</button>)}
    </nav>
    <div className="min-w-0 flex-1 overflow-y-auto p-4">
      {stale && <div role="alert" className="mb-3 rounded border p-2 text-[10.5px]" style={{ borderColor: 'var(--atlas-warn)', color: 'var(--atlas-warn)' }}><p>This section is {selectedFreshness?.status.replace('_', ' ') ?? 'stale'} for snapshot {currentSnapshotId?.slice(0, 12) ?? result.document.graph_snapshot_id.slice(0, 12)}.</p>{selectedFreshness && <p>{selectedFreshness.drift_findings.join(' · ')}</p>}<div className="mt-1 flex gap-2"><button type="button" disabled={regenerating || !selected} onClick={() => selected && void regenerate([selected.id])}>{regenerating ? 'Regenerating…' : 'Regenerate section'}</button><button type="button" disabled={regenerating} onClick={() => void regenerate()}>Regenerate all</button></div></div>}
      {result.regenerated && <div role="status" className="mb-3 rounded border p-2 text-[10px]" style={{ borderColor: 'var(--atlas-ready)' }}>{result.diff?.length ? <><strong>Regeneration preview</strong>{result.diff.map((item) => <div key={item.section_id} className="mt-1"><span className="font-semibold">{item.title}</span><del className="block">{item.before ?? 'New section'}</del><ins className="block">{item.after}</ins></div>)}</> : 'No documentation changes were required.'}</div>}
      {selected ? <DocumentSectionView section={selected} markdown={result.markdown} stale={selectedFreshness ? selectedFreshness.status !== 'current' : stale} onFocus={onFocus} onTagScope={onTagScope} /> : <p role="status">No documentation section is available.</p>}
      {agentContext && <SubsystemDocsAgent context={agentContext} onFocus={onFocus} />}
    </div>
  </aside>
}

function DocumentSectionView({ section, markdown, stale, onFocus, onTagScope }: { section: GraphDocumentSection; markdown: string; stale: boolean; onFocus: (id: string) => void; onTagScope: (tag: string, nodeIds: string[]) => void }) {
  return <article>
    <div className="flex items-center gap-2"><h2 className="flex-1 text-[15px] font-semibold">{section.title}</h2><span className="rounded px-2 py-0.5 text-[9px] uppercase" style={{ background: 'var(--atlas-chip)', color: stale ? 'var(--atlas-warn)' : 'var(--atlas-ready)' }}>{stale ? 'stale' : 'fresh'}</span></div>
    <p className="mt-1 text-[9px]" style={{ color: 'var(--atlas-text-faint)' }}>{section.confidence} confidence · {section.source_query_ids.join(', ')}</p>
    <div className="mt-3 whitespace-pre-wrap text-[11.5px] leading-relaxed" style={{ color: 'var(--atlas-text-muted)' }}>{sectionBody(markdown, section.title)}</div>
    <h3 className="mb-1 mt-4 text-[9.5px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Evidence</h3>
    {section.evidence_references.length ? <ul>{section.evidence_references.map((item) => <li key={item} className="font-mono text-[10px]">{item}</li>)}</ul> : <p className="text-[10px]">No direct evidence references for this section.</p>}
    {section.tags.length > 0 && <div aria-label="Section tags" className="mt-2 flex flex-wrap gap-1">{section.tags.map((tag) => <button type="button" key={tag.id} onClick={() => onTagScope(`${tag.namespace}:${tag.value}`, section.affected_nodes)} className="rounded px-1.5 py-0.5 text-[9px]" title={`Scope graph · derived from ${tag.source} (${tag.confidence})`} style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-text-dim)' }}>{tag.namespace}:{tag.value}</button>)}</div>}
    <h3 className="mb-1 mt-4 text-[9.5px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>In the graph</h3>
    <div className="flex flex-wrap gap-1">{section.affected_nodes.map((id) => <button key={id} type="button" onClick={() => onFocus(id)} className="max-w-full truncate rounded-full px-2 py-1 text-[10px]" title={id} style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-text-bright)' }}>{id}</button>)}</div>
    {section.affected_edges.length > 0 && <div aria-label="Related edges" className="mt-2 flex flex-wrap gap-1">{section.affected_edges.map((id) => <span key={id} className="max-w-full truncate rounded-full px-2 py-1 text-[10px]" title={id} style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-accent)' }}>{id}</span>)}</div>}
  </article>
}
