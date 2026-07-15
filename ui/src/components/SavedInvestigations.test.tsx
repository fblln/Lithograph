import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { saveInvestigation } from '../investigations'
import { SavedInvestigations } from './SavedInvestigations'

describe('SavedInvestigations', () => {
  let values = new Map<string, string>()
  beforeEach(() => { values = new Map(); vi.stubGlobal('localStorage', { getItem: (key: string) => values.get(key) ?? null, setItem: (key: string, value: string) => values.set(key, value) }) })
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })
  it('marks a saved view from a different graph snapshot as stale', () => {
    saveInvestigation({ version: 1, id: 'old', name: 'Old risk', graphSnapshotId: 'old-graph', urlState: { viewMode: 'radial' }, activeLabels: [], notes: '' })
    render(<SavedInvestigations current={{ version: 1, graphSnapshotId: 'new-graph', urlState: { viewMode: 'radial' }, activeLabels: [] }} onRestore={() => {}} />)
    expect(screen.getByRole('status')).toHaveTextContent('stale snapshot')
  })

  it('downloads a portable JSON report for a saved investigation', async () => {
    saveInvestigation({ version: 1, id: 'risk', name: 'Risk report', graphSnapshotId: 'graph', urlState: { viewMode: 'radial' }, activeLabels: [], notes: 'keep this' })
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
    const user = userEvent.setup()
    render(<SavedInvestigations current={{ version: 1, graphSnapshotId: 'graph', urlState: { viewMode: 'radial' }, activeLabels: [] }} onRestore={() => {}} />)

    await user.click(screen.getByRole('button', { name: 'Export Risk report as JSON' }))
    expect(click).toHaveBeenCalledOnce()
    const anchor = click.mock.instances[0] as HTMLAnchorElement
    expect(anchor.download).toBe('Risk-report.json')
    expect(anchor.href).toContain('data:application/json')
  })
})
