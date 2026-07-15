import { useEffect, useState, type RefObject } from 'react'
import { searchGraph, type GraphSearchResult } from '../api/search'
import { getRepositoryTensions, type RepositoryTension } from '../api/tensions'
import type { ArchitectureCluster } from '../api/architecture'
import { humanClusterNameFromEvidence } from '../clusterIdentity'

type SearchResult = { kind: 'graph'; value: GraphSearchResult } | { kind: 'tension'; value: RepositoryTension }
const EMPTY_SCOPE: string[] = []

export function ExplorerSearch({ onFocus, onSelectTension = () => {}, onFocusCluster = () => {}, clusters = [], inputRef, scopeNodeIds = EMPTY_SCOPE }: { onFocus: (id: string) => void; onSelectTension?: (tension: RepositoryTension) => void; onFocusCluster?: (cluster: ArchitectureCluster) => void; clusters?: ArchitectureCluster[]; inputRef?: RefObject<HTMLInputElement | null>; scopeNodeIds?: string[] }) {
  const [query, setQuery] = useState('')
  const [results, setResults] = useState<SearchResult[]>([])
  const [activeIndex, setActiveIndex] = useState(-1)

  useEffect(() => {
    if (!query.trim()) { setResults([]); setActiveIndex(-1); return }
    let current = true
    Promise.all([searchGraph(query), getRepositoryTensions().catch(() => [])]).then(
      ([graphResults, tensions]) => {
        if (!current) return
        const needle = query.toLocaleLowerCase()
        const scope = new Set(scopeNodeIds)
        const inScope = (id: string) => scope.size === 0 || scope.has(id)
        const tensionResults = tensions.filter((tension) => (scope.size === 0 || tension.affected_nodes.some(inScope)) && `${tension.category ?? ''} ${tension.severity ?? ''} ${tension.confidence ?? ''} ${tension.explanation ?? ''} ${(tension.evidence_references ?? []).join(' ')} ${(tension.affected_nodes ?? []).join(' ')}`.toLocaleLowerCase().includes(needle))
        const merged: SearchResult[] = [...graphResults.filter((value) => inScope(value.id)).map((value) => ({ kind: 'graph' as const, value })), ...tensionResults.map((value) => ({ kind: 'tension' as const, value }))]
        setResults(merged)
        setActiveIndex(merged.length > 0 ? 0 : -1)
      },
      () => current && (setResults([]), setActiveIndex(-1)),
    )
    return () => { current = false }
  }, [query, scopeNodeIds])

  function choose(result: SearchResult) {
    if (result.kind === 'graph') onFocus(result.value.id)
    else {
      onSelectTension(result.value)
      if (result.value.affected_nodes[0]) onFocus(result.value.affected_nodes[0])
    }
    setQuery('')
    setResults([])
  }

  function handleKeyDown(event: React.KeyboardEvent<HTMLInputElement>) {
    if (event.key === 'Escape') { setQuery(''); setResults([]); return }
    if (results.length === 0) return
    if (event.key === 'ArrowDown') { event.preventDefault(); setActiveIndex((index) => (index + 1) % results.length) }
    if (event.key === 'ArrowUp') { event.preventDefault(); setActiveIndex((index) => (index + results.length - 1) % results.length) }
    if (event.key === 'Enter') { event.preventDefault(); choose(results[activeIndex < 0 ? 0 : activeIndex]) }
  }

  return <div className="relative min-w-52 flex-1 max-sm:basis-full"><input ref={inputRef} aria-label="Search graph" aria-keyshortcuts="Control+K Meta+K" aria-autocomplete="list" aria-controls="graph-search-results" aria-activedescendant={activeIndex >= 0 ? `graph-search-result-${activeIndex}` : undefined} role="combobox" aria-expanded={results.length > 0} value={query} onChange={(event) => setQuery(event.target.value)} onKeyDown={handleKeyDown} placeholder="Search files, modules, symbols, tensions…" className="w-full rounded border px-2 py-1 text-[11px]" style={{ background: 'var(--atlas-canvas)', borderColor: 'var(--atlas-border)', color: 'var(--atlas-text-bright)' }} />{results.length > 0 && <ul id="graph-search-results" role="listbox" className="absolute z-20 mt-1 w-full rounded border p-1" style={{ background: 'var(--atlas-surface)', borderColor: 'var(--atlas-border)' }}>{results.map((result, index) => { const cluster = result.kind === 'graph' ? clusters.find((candidate) => candidate.members.includes(result.value.id)) : undefined; return <li key={result.value.id} id={`graph-search-result-${index}`} role="option" aria-selected={activeIndex === index} className="flex items-center"><button type="button" className="min-w-0 flex-1 rounded px-2 py-1 text-left text-[11px]" style={{ background: activeIndex === index ? 'var(--atlas-hover)' : 'transparent' }} onClick={() => choose(result)}><span style={{ color: 'var(--atlas-text-bright)' }}>{result.kind === 'graph' ? result.value.name : result.value.explanation}</span> <span style={{ color: 'var(--atlas-text-dim)' }}>· {result.kind === 'graph' ? result.value.label : `Tension: ${result.value.category}`}{cluster ? ` · in ${humanClusterNameFromEvidence(cluster)}` : ''}</span></button>{cluster && <button type="button" aria-label={`Focus containing cluster ${humanClusterNameFromEvidence(cluster)}`} onClick={() => { onFocusCluster(cluster); setQuery(''); setResults([]) }} className="shrink-0 rounded px-1.5 py-1 text-[9px]" style={{ color: 'var(--atlas-accent)' }}>Cluster</button>}</li> })}</ul>}</div>
}
