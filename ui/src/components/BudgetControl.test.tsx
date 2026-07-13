import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { BudgetControl } from './BudgetControl'

describe('BudgetControl', () => {
  afterEach(() => {
    cleanup()
  })

  it('applies a small value immediately on blur', async () => {
    const onApply = vi.fn()
    const user = userEvent.setup()
    render(<BudgetControl value={undefined} onApply={onApply} />)

    const input = screen.getByPlaceholderText('150 (default)')
    await user.type(input, '200')
    await user.tab()

    expect(onApply).toHaveBeenCalledWith(200)
  })

  it('applies immediately on Enter as well as blur', async () => {
    const onApply = vi.fn()
    const user = userEvent.setup()
    render(<BudgetControl value={undefined} onApply={onApply} />)

    await user.type(screen.getByPlaceholderText('150 (default)'), '200{Enter}')

    expect(onApply).toHaveBeenCalledWith(200)
  })

  it('clearing the field applies undefined (server default)', async () => {
    const onApply = vi.fn()
    const user = userEvent.setup()
    render(<BudgetControl value={200} onApply={onApply} />)

    const input = screen.getByPlaceholderText('150 (default)')
    await user.clear(input)
    await user.tab()

    expect(onApply).toHaveBeenCalledWith(undefined)
  })

  it('holds a large value behind a confirmation instead of applying it immediately', async () => {
    const onApply = vi.fn()
    const user = userEvent.setup()
    render(<BudgetControl value={undefined} onApply={onApply} />)

    await user.type(screen.getByPlaceholderText('150 (default)'), '2000')
    await user.tab()

    expect(onApply).not.toHaveBeenCalled()
    expect(screen.getByText(/2000 nodes may render slowly/)).toBeInTheDocument()

    await user.click(screen.getByText('Load anyway'))
    expect(onApply).toHaveBeenCalledWith(2000)
  })

  it('Cancel on a large-value confirmation never applies it', async () => {
    const onApply = vi.fn()
    const user = userEvent.setup()
    render(<BudgetControl value={undefined} onApply={onApply} />)

    await user.type(screen.getByPlaceholderText('150 (default)'), '2000')
    await user.tab()
    await user.click(screen.getByText('Cancel'))

    expect(onApply).not.toHaveBeenCalled()
    expect(screen.queryByText(/may render slowly/)).not.toBeInTheDocument()
  })

  it('ignores invalid (non-numeric or non-positive) input rather than applying it', async () => {
    const onApply = vi.fn()
    const user = userEvent.setup()
    render(<BudgetControl value={undefined} onApply={onApply} />)

    await user.type(screen.getByPlaceholderText('150 (default)'), '-5')
    await user.tab()

    expect(onApply).not.toHaveBeenCalled()
  })
})
