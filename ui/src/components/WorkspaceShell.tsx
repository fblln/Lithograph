import type { ReactNode } from 'react'

/** Shared dense-workspace frame: one toolbar, two optional rails, and a bounded canvas. */
export function WorkspaceShell({ topBar, sidebar, inspector, children }: { topBar: ReactNode; sidebar?: ReactNode; inspector?: ReactNode; children: ReactNode }) {
  return <div className="flex h-full min-w-0 flex-col">
    {topBar}
    <div className="relative flex min-h-0 min-w-0 flex-1">
      {sidebar}
      <main data-visual-role="graph-viewport" className="relative min-w-0 flex-1 overflow-hidden" style={{ background: 'var(--atlas-canvas)' }}>{children}</main>
      {inspector}
    </div>
  </div>
}
