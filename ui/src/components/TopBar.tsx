export interface TopBarProps {
  centerLabel: string
  status: 'loading' | 'ready' | 'error'
}

export function TopBar({ centerLabel, status }: TopBarProps) {
  return (
    <header
      className="flex flex-none items-center gap-3 border-b px-3.5 py-2"
      style={{ borderColor: 'var(--atlas-border)', background: 'var(--atlas-panel-header)' }}
    >
      <span className="text-[13px] font-semibold" style={{ color: 'var(--atlas-text)' }}>
        Lithograph <span style={{ color: 'var(--atlas-text-muted)', fontWeight: 500 }}>Explorer</span>
      </span>
      <span
        className="rounded px-2.5 py-1 text-[11px]"
        style={{ background: 'var(--atlas-chip)', color: 'var(--atlas-text-dim)' }}
      >
        {centerLabel}
      </span>
      <span className="flex-1" />
      <StatusDot status={status} />
    </header>
  )
}

function StatusDot({ status }: { status: TopBarProps['status'] }) {
  const color =
    status === 'ready' ? 'var(--atlas-ready)' : status === 'error' ? '#e5484d' : 'var(--atlas-text-dim)'
  const label = status === 'ready' ? 'Ready' : status === 'error' ? 'Error' : 'Loading…'
  return (
    <span className="flex items-center gap-1.5 text-[11px]" style={{ color }}>
      <span className="h-[7px] w-[7px] rounded-full" style={{ background: color }} />
      {label}
    </span>
  )
}
