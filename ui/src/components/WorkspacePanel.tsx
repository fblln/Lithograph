import type { ReactNode } from 'react'

export function WorkspacePanel({ title, children, side = 'left' }: { title: string; children: ReactNode; side?: 'left' | 'right' }) {
  return <aside className={`flex w-72 flex-none flex-col overflow-y-auto p-3 ${side === 'left' ? 'border-r' : 'border-l max-lg:absolute max-lg:inset-y-0 max-lg:right-0 max-lg:z-20 max-lg:shadow-2xl'}`} style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-surface)' }}><h2 className="mb-2 text-[9.5px] font-bold tracking-wide uppercase" style={{ color: 'var(--atlas-text-dim)' }}>{title}</h2>{children}</aside>
}
