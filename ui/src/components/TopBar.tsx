import { useEffect, useRef } from 'react'
import { ExplorerSearch } from './ExplorerSearch'
import type { ArchitectureCluster } from '../api/architecture'
import type { RepositoryTension } from '../api/tensions'

export interface TopBarProps {
  centerLabel: string
  status: 'loading' | 'ready' | 'error'
  onFocus: (id: string) => void
  clusters?: ArchitectureCluster[]
  onFocusCluster?: (cluster: ArchitectureCluster) => void
  onSelectTension?: (tension: RepositoryTension) => void
  snapshotId?: string
  renderedNodes?: number
  availableNodes?: number
  scopeNodeIds?: string[]
  workspaceMode?: 'explore' | 'docs'
  onWorkspaceMode?: (mode: 'explore' | 'docs') => void
  breadcrumbs?: string[]
  onNavigateBreadcrumb?: (index: number) => void
  onBack?: () => void
}

export function TopBar({ centerLabel, status, onFocus, clusters = [], onFocusCluster = () => {}, onSelectTension, snapshotId, renderedNodes, availableNodes, scopeNodeIds, workspaceMode = 'explore', onWorkspaceMode = () => {}, breadcrumbs = [centerLabel], onNavigateBreadcrumb = () => {}, onBack }: TopBarProps) {
  const searchRef = useRef<HTMLInputElement>(null)

  useEffect(() => {
    function focusSearch(event: KeyboardEvent) {
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        searchRef.current?.focus()
      }
    }
    window.addEventListener('keydown', focusSearch)
    return () => window.removeEventListener('keydown', focusSearch)
  }, [])

  return (
    <header
      className="flex min-w-0 flex-none flex-wrap items-center gap-2 border-b px-3.5 py-2"
      style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}
    >
      <div className="flex items-center gap-2 whitespace-nowrap text-[13.5px] font-semibold" style={{ color: 'var(--atlas-text)' }}><AtlasMark />Lithograph <span style={{ color: 'var(--atlas-text-muted)', fontWeight: 500 }}>Atlas</span></div>
      <nav aria-label="Exploration path" className="flex min-w-0 max-w-[38vw] items-center gap-1 rounded px-1.5 py-1 text-[10.5px]" style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-text-dim)' }}>
        {breadcrumbs.length > 1 && <button type="button" aria-label="Back to previous context" onClick={onBack} className="rounded px-1.5 py-0.5" style={{ color: 'var(--atlas-accent)' }}>← Back</button>}
        <div className="flex min-w-0 items-center overflow-hidden">{breadcrumbs.map((crumb, index) => <span key={`${crumb}:${index}`} className="flex min-w-0 items-center"><span aria-hidden="true" className={index === 0 ? 'hidden' : 'mx-1'}>/</span><button type="button" aria-current={index === breadcrumbs.length - 1 ? 'page' : undefined} onClick={() => onNavigateBreadcrumb(index)} className="max-w-32 truncate rounded px-1 py-0.5 font-semibold" title={crumb} style={{ color: index === breadcrumbs.length - 1 ? 'var(--atlas-text-bright)' : 'var(--atlas-text-muted)' }}>{crumb}</button></span>)}</div>
      </nav>
      {snapshotId && <span className="hidden whitespace-nowrap text-[9.5px] lg:inline" style={{ color: 'var(--atlas-text-faint)' }}>snapshot {snapshotId.slice(0, 12)}{renderedNodes !== undefined && availableNodes !== undefined ? ` · ${renderedNodes}/${availableNodes} nodes` : ''}</span>}
      <ExplorerSearch onFocus={onFocus} onFocusCluster={onFocusCluster} clusters={clusters} onSelectTension={onSelectTension} inputRef={searchRef} scopeNodeIds={scopeNodeIds} />
      <div className="flex rounded p-0.5" style={{ background: 'var(--atlas-chip)' }}><button type="button" data-active={workspaceMode === 'explore'} onClick={() => onWorkspaceMode('explore')} className="rounded px-2.5 py-1 text-[10.5px]" style={{ background: workspaceMode === 'explore' ? 'var(--atlas-accent)' : 'transparent' }}>Explore</button><button type="button" data-active={workspaceMode === 'docs'} onClick={() => onWorkspaceMode('docs')} className="rounded px-2.5 py-1 text-[10.5px]" style={{ background: workspaceMode === 'docs' ? 'var(--atlas-accent)' : 'transparent' }}>Docs</button></div>
      <StatusDot status={status} />
    </header>
  )
}

function AtlasMark() {
  return <svg aria-hidden="true" width="16" height="16" viewBox="0 0 18 18"><circle cx="4" cy="4" r="2.6" fill="var(--atlas-accent)" /><circle cx="14" cy="4" r="2.6" fill="var(--node-symbol)" /><circle cx="9" cy="13.5" r="2.6" fill="var(--node-config)" /><path d="M4 4 9 13.5 14 4" fill="none" stroke="var(--atlas-border-strong)" strokeWidth="1.1" /></svg>
}

function StatusDot({ status }: { status: TopBarProps['status'] }) {
  const color =
    status === 'ready' ? 'var(--atlas-ready)' : status === 'error' ? 'var(--atlas-danger)' : 'var(--atlas-loading)'
  const label = status === 'ready' ? 'Ready' : status === 'error' ? 'Error' : 'Loading…'
  return (
    <span className="flex items-center gap-1.5 text-[11px]" style={{ color }}>
      <span className="h-[7px] w-[7px] rounded-full" style={{ background: color }} />
      {label}
    </span>
  )
}
