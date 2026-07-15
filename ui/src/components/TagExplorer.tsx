import { useEffect, useMemo, useState } from 'react'
import { getTagFacets, resolveTagExpression } from '../api/tags'
import { parseExpression, serializeExpression, type TagMode } from '../tagExpression'

interface SavedTagFilter { name: string; expression: string }
const STORAGE_KEY = 'lithograph:tag-filters:v1'

export function TagExplorer({ expression, onChange }: { expression: string; onChange: (expression: string, nodeIds: string[]) => void }) {
  const [facets, setFacets] = useState<Record<string, number> | null>(null)
  const [query, setQuery] = useState('')
  const [error, setError] = useState<string | null>(null)
  const [saved, setSaved] = useState<SavedTagFilter[]>(readSaved)
  const selected = useMemo(() => parseExpression(expression), [expression])

  useEffect(() => { getTagFacets().then(setFacets, (cause: unknown) => setError(String(cause))) }, [])

  async function apply(next: Map<string, TagMode>) {
    const nextExpression = serializeExpression(next)
    if (!nextExpression) {
      setError(null)
      onChange('', [])
      return
    }
    try {
      const nodeIds = await resolveTagExpression(nextExpression)
      setError(null)
      onChange(nextExpression, nodeIds)
    } catch (cause) {
      setError(String(cause))
    }
  }

  function toggle(tag: string, mode: TagMode) {
    const next = new Map(selected)
    if (next.get(tag) === mode) next.delete(tag)
    else next.set(tag, mode)
    void apply(next)
  }

  function saveCurrent() {
    if (!expression || saved.some((item) => item.expression === expression)) return
    const next = [...saved, { name: expression, expression }]
    setSaved(next)
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next))
  }

  if (facets === null && error === null) return <section className="p-3 text-[11px]">Loading tags…</section>
  const needle = query.trim().toLocaleLowerCase()
  const entries = Object.entries(facets ?? {}).filter(([tag]) => tag.toLocaleLowerCase().includes(needle))
  const grouped = groupFacets(entries)

  return <section className="p-3 text-[11px]">
    <h2 className="mb-2 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Tags</h2>
    <input aria-label="Search tags" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search tags…" className="mb-2 w-full rounded border px-2 py-1" style={{ background: 'var(--atlas-canvas)', borderColor: 'var(--atlas-border)', color: 'var(--atlas-text-bright)' }} />
    {selected.size > 0 && <div aria-label="Selected tags" className="mb-2 flex flex-wrap gap-1">{[...selected].map(([tag, mode]) => <button key={tag} type="button" onClick={() => toggle(tag, mode)} className="max-w-full truncate rounded px-1.5 py-0.5" title={tag} style={{ color: mode === 'include' ? 'var(--atlas-ready)' : 'var(--atlas-danger)', background: 'var(--atlas-chip)' }}>{mode === 'include' ? '+' : '−'} {tag} ×</button>)}</div>}
    {expression && <button type="button" onClick={saveCurrent} className="mb-2 rounded px-2 py-1" style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-text-bright)' }}>Save filter</button>}
    {saved.length > 0 && <div className="mb-3"><h3 className="mb-1 text-[9px] font-bold uppercase" style={{ color: 'var(--atlas-text-faint)' }}>Saved filters</h3>{saved.map((item) => <button key={item.expression} type="button" onClick={() => void apply(parseExpression(item.expression))} className="mb-1 block w-full truncate text-left" title={item.expression}>{item.name}</button>)}</div>}
    {entries.length === 0 ? <p role="status" style={{ color: 'var(--atlas-text-muted)' }}>No tags match this search.</p> : [...grouped].sort(([a], [b]) => a.localeCompare(b)).map(([namespace, tags]) => <div key={namespace} className="mb-3"><h3 className="mb-1 text-[9.5px] font-bold uppercase" style={{ color: 'var(--atlas-text-faint)' }}>{namespace}</h3><ul>{tags.sort(([a], [b]) => a.localeCompare(b)).map(([tag, count]) => <li key={tag} className="flex min-w-0 items-center gap-1 py-0.5"><span className="min-w-0 flex-1 truncate" title={tag}>{tag.slice(namespace.length + 1)}</span><span style={{ color: 'var(--atlas-text-faint)' }}>{count}</span><button aria-label={`Include ${tag}`} data-active={selected.get(tag) === 'include'} type="button" onClick={() => toggle(tag, 'include')} className="h-5 w-5 rounded" style={{ color: selected.get(tag) === 'include' ? 'var(--atlas-ready)' : 'var(--atlas-text-muted)', background: 'var(--atlas-chip)' }}>+</button><button aria-label={`Exclude ${tag}`} data-active={selected.get(tag) === 'exclude'} type="button" onClick={() => toggle(tag, 'exclude')} className="h-5 w-5 rounded" style={{ color: selected.get(tag) === 'exclude' ? 'var(--atlas-danger)' : 'var(--atlas-text-muted)', background: 'var(--atlas-chip)' }}>−</button></li>)}</ul></div>)}
    {error && <p role="alert" style={{ color: 'var(--atlas-danger)' }}>{error}</p>}
  </section>
}

function readSaved(): SavedTagFilter[] {
  try {
    const value = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]')
    return Array.isArray(value) ? value.filter((item): item is SavedTagFilter => typeof item?.name === 'string' && typeof item?.expression === 'string') : []
  } catch {
    return []
  }
}

function groupFacets(entries: Array<[string, number]>): Map<string, Array<[string, number]>> {
  const grouped = new Map<string, Array<[string, number]>>()
  for (const entry of entries) {
    const namespace = entry[0].split(':', 1)[0]
    const values = grouped.get(namespace) ?? []
    values.push(entry)
    grouped.set(namespace, values)
  }
  return grouped
}
