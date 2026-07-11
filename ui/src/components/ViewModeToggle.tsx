export type ViewMode = 'radial' | 'matrix'

export interface ViewModeToggleProps {
  mode: ViewMode
  onChange: (mode: ViewMode) => void
}

const OPTIONS: Array<{ mode: ViewMode; label: string }> = [
  { mode: 'radial', label: 'Radial' },
  { mode: 'matrix', label: 'Matrix' },
]

/**
 * Segmented pill button group for switching between layout view modes.
 * Mirrors the view-switcher chrome in lithograph-atlas-prototype/Lithograph
 * Atlas.dc.html (the `viewOptions` button row) so this reads as the same UI
 * language as the rest of the app's chrome.
 */
export function ViewModeToggle({ mode, onChange }: ViewModeToggleProps) {
  return (
    <div
      className="flex gap-0.5 rounded-[7px] p-0.5"
      style={{ background: 'oklch(0.19 0.006 260 / 0.94)', border: '1px solid var(--atlas-border-strong)' }}
    >
      {OPTIONS.map((option) => {
        const active = option.mode === mode
        return (
          <button
            key={option.mode}
            type="button"
            data-active={active}
            onClick={() => onChange(option.mode)}
            className="rounded-[5px] border-0 px-3 py-1.5 text-[10.5px] font-semibold"
            style={{
              background: active ? 'var(--atlas-accent)' : 'transparent',
              color: active ? 'var(--atlas-bg)' : 'var(--atlas-text-muted)',
              cursor: 'pointer',
            }}
          >
            {option.label}
          </button>
        )
      })}
    </div>
  )
}
