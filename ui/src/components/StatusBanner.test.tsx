import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { StatusBanner } from './StatusBanner'

describe('StatusBanner', () => {
  afterEach(cleanup)

  it('uses the error surface and text semantic tokens', () => {
    render(<StatusBanner>Unable to load the graph</StatusBanner>)
    expect(screen.getByRole('status')).toHaveStyle({ background: 'var(--atlas-error-surface)', color: 'var(--atlas-error-text)' })
  })
})
