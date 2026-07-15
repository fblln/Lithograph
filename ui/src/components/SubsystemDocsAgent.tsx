import { useEffect, useState } from 'react'
import { generateSubsystemDocument, refineSubsystemDocument, type SubsystemDocument } from '../api/subsystemDocs'

export interface SubsystemAgentContext {
  scopeId: string
  nodeIds: string[]
  edgeCount: number
  evidenceCount: number
  tensionCount: number
  graphSnapshotId?: string
}

export function SubsystemDocsAgent({ context, onFocus }: { context: SubsystemAgentContext; onFocus: (id: string) => void }) {
  const [versions, setVersions] = useState<SubsystemDocument[]>([])
  const [activeVersion, setActiveVersion] = useState(-1)
  const [acceptedVersion, setAcceptedVersion] = useState<number | undefined>()
  const [instruction, setInstruction] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [comparing, setComparing] = useState(false)
  const storageKey = `lithograph:subsystem-docs-agent:v1:${context.graphSnapshotId ?? 'unknown'}:${context.scopeId}`
  const current = versions[activeVersion]
  const stale = current && context.graphSnapshotId !== undefined && current.graph_snapshot_id !== context.graphSnapshotId
  const estimatedTokens = context.nodeIds.length * 40 + context.edgeCount * 20

  useEffect(() => {
    try {
      const session = JSON.parse(localStorage.getItem(storageKey) ?? 'null') as { versions?: SubsystemDocument[]; activeVersion?: number; acceptedVersion?: number } | null
      setVersions(Array.isArray(session?.versions) ? session.versions : [])
      setActiveVersion(typeof session?.activeVersion === 'number' ? session.activeVersion : -1)
      setAcceptedVersion(typeof session?.acceptedVersion === 'number' ? session.acceptedVersion : undefined)
    } catch { setVersions([]); setActiveVersion(-1); setAcceptedVersion(undefined) }
  }, [storageKey])

  function persist(nextVersions: SubsystemDocument[], nextActive: number, nextAccepted = acceptedVersion) {
    try { localStorage.setItem(storageKey, JSON.stringify({ versions: nextVersions, activeVersion: nextActive, acceptedVersion: nextAccepted })) } catch { /* Private browsing may disable persistence; the active session still works. */ }
  }

  async function run(refine: boolean) {
    setBusy(true); setError(null)
    try {
      const document = refine
        ? await refineSubsystemDocument(context.scopeId, context.nodeIds, instruction)
        : await generateSubsystemDocument(context.scopeId, context.nodeIds, instruction || undefined)
      const nextVersions = [...versions, document]
      const nextActive = nextVersions.length - 1
      setVersions(nextVersions)
      setActiveVersion(nextActive)
      persist(nextVersions, nextActive)
      setInstruction('')
    } catch (cause) { setError(String(cause)) }
    finally { setBusy(false) }
  }

  return <section aria-label="Subsystem documentation agent" className="mt-5 border-t pt-4" style={{ borderColor: 'var(--atlas-border)' }}>
    <h2 className="text-[11px] font-bold uppercase" style={{ color: 'var(--atlas-text-dim)' }}>Draft subsystem docs</h2>
    <div aria-label="Agent context" className="mt-2 rounded border p-2 text-[10px]" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}>
      <p className="truncate" title={context.scopeId}>Scope: <strong>{context.scopeId}</strong></p>
      <p>{context.nodeIds.length} nodes · {context.edgeCount} edges · {context.evidenceCount} evidence refs · {context.tensionCount} tensions</p>
      <p>Snapshot {context.graphSnapshotId?.slice(0, 12) ?? 'loading'} · estimated {estimatedTokens.toLocaleString()} context tokens</p>
    </div>
    {!current && <button type="button" disabled={busy || context.nodeIds.length === 0} onClick={() => void run(false)} className="mt-2 w-full rounded px-2 py-1.5 text-[10.5px] font-semibold" style={{ background: 'var(--atlas-accent)', color: 'var(--atlas-canvas)' }}>{busy ? 'Generating…' : 'Generate with graph agent'}</button>}
    {error && <p role="alert" className="mt-2 text-[10px]" style={{ color: 'var(--atlas-danger)' }}>{error}</p>}
    {current && <div className="mt-3" aria-busy={busy}>
      {stale && <p role="alert" className="mb-2 text-[10px]" style={{ color: 'var(--atlas-warn)' }}>This version is stale for the current graph snapshot. Generate again before accepting it.</p>}
      <div className="flex items-center gap-2 text-[9.5px]"><strong>Version {activeVersion + 1}/{versions.length}</strong><span>{current.confidence} confidence</span><span>{acceptedVersion === activeVersion ? 'accepted' : 'draft'}</span></div>
      <pre className="mt-2 max-h-56 overflow-auto whitespace-pre-wrap rounded p-2 text-[10px]" style={{ background: 'var(--atlas-canvas)', color: 'var(--atlas-text-muted)' }}>{current.markdown}</pre>
      {comparing && activeVersion > 0 && <div aria-label="Previous version" className="mt-2 rounded border p-2 text-[10px]" style={{ borderColor: 'var(--atlas-border)' }}><strong>Previous version</strong><pre className="mt-1 whitespace-pre-wrap">{versions[activeVersion - 1].markdown}</pre></div>}
      <div className="mt-2 flex flex-wrap gap-1">{current.cited_nodes.slice(0, 10).map((id) => <button type="button" key={id} onClick={() => onFocus(id)} title={id} className="max-w-full truncate rounded px-1.5 py-0.5 text-[9px]" style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-accent)' }}>{id}</button>)}</div>
      <label className="mt-3 block text-[9.5px]">Refinement instruction<textarea aria-label="Refinement instruction" value={instruction} onChange={(event) => setInstruction(event.target.value)} className="mt-1 h-16 w-full rounded border p-2 text-[10.5px]" style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-canvas)' }} /></label>
      <div className="mt-2 flex flex-wrap gap-1"><button type="button" disabled={busy || !instruction.trim()} onClick={() => void run(true)}>{busy ? 'Refining…' : 'Refine'}</button><button type="button" disabled={activeVersion === 0} onClick={() => { const next = activeVersion - 1; setActiveVersion(next); persist(versions, next) }}>Revert</button><button type="button" disabled={activeVersion === 0} aria-pressed={comparing} onClick={() => setComparing((value) => !value)}>Compare</button><button type="button" disabled={Boolean(stale)} onClick={() => { setAcceptedVersion(activeVersion); persist(versions, activeVersion, activeVersion) }}>Accept</button></div>
    </div>}
  </section>
}
