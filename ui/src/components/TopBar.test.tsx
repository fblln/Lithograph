import { cleanup, fireEvent, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it, vi } from 'vitest'
import userEvent from '@testing-library/user-event'
import { TopBar } from './TopBar'

describe('TopBar', () => {
  afterEach(() => { cleanup(); vi.unstubAllGlobals() })

  it('keeps long context and snapshot metadata constrained in narrow layouts', () => {
    render(<TopBar centerLabel="a/very/long/path/that/must/not/push/controls/off/screen" snapshotId="blake3:an-extremely-long-snapshot-id" status="ready" onFocus={() => {}} />)
    expect(screen.getByText(/a\/very\/long/)).toHaveClass('max-w-32', 'truncate')
    expect(screen.getByText(/snapshot blake3:an-ex/)).toHaveClass('hidden', 'lg:inline')
    expect(screen.getByLabelText('Search graph').parentElement).toHaveClass('max-sm:basis-full')
  })

  it('exposes a keyboard-accessible reversible exploration path', async () => {
    const user = userEvent.setup()
    const onBack = vi.fn()
    const onNavigate = vi.fn()
    render(<TopBar centerLabel="API" breadcrumbs={['Overview', 'Web', 'API']} onBack={onBack} onNavigateBreadcrumb={onNavigate} status="ready" onFocus={() => {}} />)
    expect(screen.getByRole('navigation', { name: 'Exploration path' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Back to previous context' }))
    await user.click(screen.getByRole('button', { name: 'Web' }))
    expect(onBack).toHaveBeenCalledOnce()
    expect(onNavigate).toHaveBeenCalledWith(1)
    expect(screen.getByRole('button', { name: 'API' })).toHaveAttribute('aria-current', 'page')
  })

  it('focuses graph search with the command shortcut', () => {
    render(<TopBar centerLabel="overview" status="ready" onFocus={() => {}} />)
    const search = screen.getByRole('combobox', { name: 'Search graph' })
    fireEvent.keyDown(window, { key: 'k', ctrlKey: true })
    expect(search).toHaveFocus()
    expect(search).toHaveAttribute('aria-keyshortcuts', 'Control+K Meta+K')
  })

  it('uses semantic tokens for loading and error state', () => {
    const { rerender } = render(<TopBar centerLabel="overview" status="loading" onFocus={() => {}} />)
    expect(screen.getByText('Loading…')).toHaveStyle({ color: 'var(--atlas-loading)' })
    rerender(<TopBar centerLabel="overview" status="error" onFocus={() => {}} />)
    expect(screen.getByText('Error')).toHaveStyle({ color: 'var(--atlas-danger)' })
  })

  it('shortens hash-root breadcrumbs but preserves the full tooltip', () => {
    const hash = '0123456789abcdef0123456789abcdef'
    const crumb = `.cache/${hash}/src/api.ts`
    render(<TopBar centerLabel={crumb} breadcrumbs={['Overview', crumb]} displayRootPrefix={`.cache/${hash}/`} status="ready" onFocus={() => {}} />)
    const button = screen.getByRole('button', { name: 'src/api.ts' })
    expect(button).toHaveAttribute('title', crumb)
    expect(screen.queryByText(new RegExp(hash))).not.toBeInTheDocument()
  })
})
