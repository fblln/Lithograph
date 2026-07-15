import { useEffect, useState } from 'react'
import { getGraphSchema, queryGraph, type GraphSchema, type QueryRow } from '../api/query'
import { TraceExplorer } from './TraceExplorer'
import type { InvestigationQueryState } from '../investigations'

const EXAMPLE = 'MATCH (a:Artifact)-[:Contains]->(b:Symbol) RETURN a, b'

const EMPTY_QUERY_STATE: InvestigationQueryState = { query: EXAMPLE, rows: [] }
const EMPTY_SCOPE: string[] = []

export function QueryWorkbench({ onFocusNode, state = EMPTY_QUERY_STATE, onStateChange = () => {}, scopeNodeIds = EMPTY_SCOPE }: { onFocusNode: (id: string) => void; state?: InvestigationQueryState; onStateChange?: (state: InvestigationQueryState) => void; scopeNodeIds?: string[] }) {
  const [query, setQuery] = useState(state.query)
  const [rows, setRows] = useState<QueryRow[]>(state.rows)
  const [saved, setSaved] = useState<string[]>([])
  const [schema, setSchema] = useState<GraphSchema | null>(null)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => { getGraphSchema().then(setSchema, (cause: unknown) => setError(String(cause))) }, [])
  useEffect(() => { setQuery(state.query); setRows(state.rows) }, [state])
  function updateQuery(next: string) { setQuery(next); onStateChange({ query: next, rows }) }
  async function execute() {
    setError(null)
    try {
      const result = await queryGraph(query)
      setRows(result)
      onStateChange({ query, rows: result })
    } catch (cause) { setError(cause instanceof Error ? cause.message : String(cause)) }
  }
  function save() { if (query.trim() && !saved.includes(query)) setSaved((items) => [...items, query]) }
  const visibleRows = scopeNodeIds.length > 0 ? rows.filter((row) => scopeNodeIds.includes(row.id)) : rows
  return <section className="p-3 text-[11px]">
    <h2 className="mb-2 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Query workbench</h2>
    {scopeNodeIds.length > 0 && <p aria-label="Query scope" className="mb-2" style={{ color: 'var(--atlas-accent)' }}>Scoped to {scopeNodeIds.length} cluster/tag nodes</p>}
    <textarea aria-label="Graph query" value={query} onChange={(event) => updateQuery(event.target.value)} className="mb-2 min-h-20 w-full rounded border p-2 font-mono text-[10px]" style={{ background: 'var(--atlas-canvas)', borderColor: 'var(--atlas-border)', color: 'var(--atlas-text-bright)' }} />
    <div className="mb-3 flex gap-2"><button type="button" onClick={execute}>Run query</button><button type="button" onClick={save}>Save query</button></div>
    {error && <p role="alert" style={{ color: 'var(--atlas-danger)' }}>{error}</p>}
    {saved.length > 0 && <div className="mb-3"><h3>Saved queries</h3>{saved.map((item) => <button key={item} type="button" onClick={() => setQuery(item)} className="block max-w-full truncate text-left">{item}</button>)}</div>}
    {rows.length > 0 && <div className="mb-3"><h3>Results</h3>{visibleRows.length === 0 ? <p role="status">No query results are inside the current scope.</p> : <table><tbody>{visibleRows.map((row) => <tr key={`${row.alias}:${row.id}`}><td>{row.label}</td><td>{row.name}</td><td><button type="button" onClick={() => onFocusNode(row.id)}>Focus</button></td></tr>)}</tbody></table>}</div>}
    <Schema schema={schema} />
    <TraceExplorer onFocusNode={onFocusNode} scopeNodeIds={scopeNodeIds} />
  </section>
}

function Schema({ schema }: { schema: GraphSchema | null }) {
  if (!schema) return <p>Loading schema…</p>
  return <div><h3>Schema</h3><p>Node labels: {schema.node_labels.map((item) => `${item.label} (${item.count})`).join(', ')}</p><p>Edge types: {schema.edge_types.map((item) => `${item.edge_type} (${item.count})`).join(', ')}</p><p>Properties: name, path</p><p>Metrics: in-degree, out-degree, hop from focus</p><p>Communities: cluster overlays and cohesion</p><p>Examples: {schema.relationship_patterns.slice(0, 3).join('; ')}</p></div>
}
