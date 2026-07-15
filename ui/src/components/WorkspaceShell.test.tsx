import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { WorkspaceShell } from './WorkspaceShell'
import { WorkspacePanel } from './WorkspacePanel'

describe('WorkspaceShell', () => {
  afterEach(cleanup)

  it('keeps optional rails outside the bounded graph canvas', () => {
    render(<WorkspaceShell topBar={<header>Toolbar</header>} sidebar={<aside>Filters</aside>} inspector={<WorkspacePanel title="Inspector" side="right">Details</WorkspacePanel>}><div>Canvas</div></WorkspaceShell>)
    expect(screen.getByRole('main')).toHaveClass('relative', 'min-w-0', 'flex-1')
    expect(screen.getByText('Inspector').closest('aside')).toHaveClass('max-lg:absolute', 'max-lg:right-0')
    expect(screen.getByText('Filters').closest('main')).toBeNull()
    expect(screen.getByText('Inspector').closest('main')).toBeNull()
    expect(screen.getByText('Canvas')).toBeInTheDocument()
  })
})
