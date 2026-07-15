import { afterEach, describe, expect, it, vi } from 'vitest'
import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { ViewModeToggle } from './ViewModeToggle'

describe('ViewModeToggle', () => {
  afterEach(() => {
    cleanup()
  })

  it('calls onChange with "cluster" when the Cluster button is clicked', async () => {
    const onChange = vi.fn()
    const user = userEvent.setup()
    render(<ViewModeToggle mode="radial" onChange={onChange} />)

    await user.click(screen.getByText('Cluster'))

    expect(onChange).toHaveBeenCalledWith('cluster')
  })

  it('calls onChange with "radial" when the Radial button is clicked', async () => {
    const onChange = vi.fn()
    const user = userEvent.setup()
    render(<ViewModeToggle mode="cluster" onChange={onChange} />)

    await user.click(screen.getByText('Radial'))

    expect(onChange).toHaveBeenCalledWith('radial')
  })

  it('marks the current mode as the active button', () => {
    render(<ViewModeToggle mode="radial" onChange={vi.fn()} />)

    expect(screen.getByText('Radial')).toHaveAttribute('data-active', 'true')
    expect(screen.getByText('Cluster')).toHaveAttribute('data-active', 'false')
  })

  it('flips which button is active when the mode prop changes', () => {
    const { rerender } = render(<ViewModeToggle mode="radial" onChange={vi.fn()} />)
    expect(screen.getByText('Cluster')).toHaveAttribute('data-active', 'false')

    rerender(<ViewModeToggle mode="cluster" onChange={vi.fn()} />)
    expect(screen.getByText('Cluster')).toHaveAttribute('data-active', 'true')
    expect(screen.getByText('Radial')).toHaveAttribute('data-active', 'false')
  })
})
