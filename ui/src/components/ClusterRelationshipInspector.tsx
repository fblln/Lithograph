import { useMemo, useState } from 'react'
import type { ClusterIdentity } from '../clusterIdentity'
import type { ClusterLink, ClusterLayoutResult } from '../graph/clusterLayout'
import type { LayoutEdge } from '../graph/types'

export function ClusterRelationshipInspector({ clusterLayout, identities, selectedClusterId, onInspectRelationship }: { clusterLayout: ClusterLayoutResult; identities: Map<string, ClusterIdentity>; selectedClusterId: string | null; onInspectRelationship: (edge: LayoutEdge) => void }) {
  const [expanded, setExpanded] = useState<string | null>(null)
  const links = useMemo(() => clusterLayout.links
    .filter((link) => link.source === selectedClusterId || link.target === selectedClusterId)
    .sort((a, b) => b.count - a.count || linkKey(a).localeCompare(linkKey(b))), [clusterLayout.links, selectedClusterId])
  if (!selectedClusterId) return null
  const selected = identities.get(selectedClusterId)
  return <aside aria-label="Selected cluster relationships" className="absolute right-3 top-20 z-10 max-h-[55%] w-72 overflow-auto rounded border p-2 text-[10px]" style={{ borderColor: 'var(--atlas-border-strong)', background: 'color-mix(in srgb, var(--atlas-surface) 94%, transparent)' }}>
    <h2 className="font-semibold" style={{ color: 'var(--atlas-text-bright)' }}>{selected?.name ?? selectedClusterId}</h2>
    <p style={{ color: 'var(--atlas-text-muted)' }}>{selected?.responsibility ?? 'Selected architectural region.'}</p>
    <p className="mt-1">{selected?.visibleMemberCount ?? 0}/{selected?.memberCount ?? 0} nodes · {selected?.fileCount ?? 0} files{selected?.partial ? ' · partial render' : ''}</p>
    <h3 className="mt-2 font-semibold">Cluster dependencies</h3>
    {links.length === 0 ? <p role="status" style={{ color: 'var(--atlas-text-muted)' }}>No cross-cluster relationships are visible under the current relationship filters.</p> : <ul className="mt-1 space-y-1">{links.map((link) => {
      const key = linkKey(link)
      const outgoing = link.source === selectedClusterId
      const otherId = outgoing ? link.target : link.source
      const other = identities.get(otherId)
      const isExpanded = expanded === key
      return <li key={key} className="rounded border p-1.5" style={{ borderColor: 'var(--atlas-border)' }}>
        <button type="button" aria-expanded={isExpanded} onClick={() => setExpanded(isExpanded ? null : key)} className="w-full text-left">
          <strong>{outgoing ? '→' : '←'} {other?.name ?? otherId}</strong>
          <span className="block" style={{ color: 'var(--atlas-text-dim)' }}>{link.count} relationship{link.count === 1 ? '' : 's'} · {link.kinds.slice(0, 3).map(({ kind, count }) => `${humanKind(kind)} ${count}`).join(', ')}</span>
        </button>
        {isExpanded && <ul className="mt-1 border-t pt-1" style={{ borderColor: 'var(--atlas-border)' }}>{link.underlying.map((edge) => <li key={`${edge.source}\0${edge.target}\0${edge.kind}`}><button type="button" onClick={() => onInspectRelationship(edge)} className="w-full truncate py-0.5 text-left" title={`${edge.source} → ${edge.target}`}><span style={{ color: 'var(--atlas-accent)' }}>{humanKind(edge.kind)}</span> · {shortName(edge.source)} → {shortName(edge.target)}</button></li>)}</ul>}
      </li>
    })}</ul>}
    <p className="mt-2" style={{ color: 'var(--atlas-text-faint)' }}>Select once to inspect this region; double-click its graph label to enter all members.</p>
  </aside>
}

function linkKey(link: ClusterLink): string { return `${link.source}\0${link.target}` }
function shortName(id: string): string { return id.split(/[/:#]/).filter(Boolean).at(-1) ?? id }
function humanKind(value: string): string { return value.replace(/([a-z])([A-Z])/g, '$1 $2').replace(/[-_]+/g, ' ') }
