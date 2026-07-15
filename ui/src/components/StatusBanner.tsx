import type { ReactNode } from 'react'

export function StatusBanner({ tone = 'error', children }: { tone?: 'error' | 'warning' | 'info'; children: ReactNode }) {
  const background = tone === 'error' ? 'var(--atlas-error-surface)' : tone === 'warning' ? 'var(--atlas-warn)' : 'var(--atlas-chip)'
  const color = tone === 'error' ? 'var(--atlas-error-text)' : 'var(--atlas-text)'
  return <p role="status" className="rounded px-3 py-1.5 text-[12px]" style={{ background, color }}>{children}</p>
}
