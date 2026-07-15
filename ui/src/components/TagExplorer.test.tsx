import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { useState } from 'react'
import { TagExplorer } from './TagExplorer'
import { parseExpression, serializeExpression } from '../tagExpression'

const resolveTagExpression = vi.fn(async (expression: string) => expression.includes('kind:artifact') ? ['artifact:a'] : [])
vi.mock('../api/tags', () => ({
  getTagFacets: async () => ({ 'kind:artifact': 12, 'kind:symbol': 7, 'role:this-is-a-deliberately-long-tag-value-for-overflow-testing': 1 }),
  resolveTagExpression: (expression: string) => resolveTagExpression(expression),
}))

function Harness() {
  const [expression, setExpression] = useState('')
  return <TagExplorer expression={expression} onChange={(next) => setExpression(next)} />
}

describe('TagExplorer', () => {
  beforeEach(() => {
    const values = new Map<string, string>()
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      clear: () => values.clear(),
    })
  })
  afterEach(() => { cleanup(); vi.clearAllMocks(); vi.unstubAllGlobals() })

  it('searches namespaced facets and composes include/exclude filters', async () => {
    const user = userEvent.setup()
    render(<Harness />)
    expect(await screen.findByText('artifact')).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Include kind:artifact' }))
    expect(resolveTagExpression).toHaveBeenLastCalledWith('kind:artifact')
    expect(screen.getByLabelText('Selected tags')).toHaveTextContent('+ kind:artifact')
    await user.click(screen.getByRole('button', { name: 'Exclude kind:symbol' }))
    expect(resolveTagExpression).toHaveBeenLastCalledWith('kind:artifact,!kind:symbol')
    await user.type(screen.getByLabelText('Search tags'), 'missing')
    expect(screen.getByRole('status')).toHaveTextContent('No tags match')
  })

  it('persists filters and keeps long tag labels accessible', async () => {
    const user = userEvent.setup()
    render(<Harness />)
    expect(await screen.findByRole('button', { name: 'Include role:this-is-a-deliberately-long-tag-value-for-overflow-testing' })).toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: 'Include kind:artifact' }))
    await user.click(screen.getByRole('button', { name: 'Save filter' }))
    expect(localStorage.getItem('lithograph:tag-filters:v1')).toContain('kind:artifact')
  })

  it('round-trips deterministic expressions', () => {
    expect(serializeExpression(parseExpression('kind:symbol,!risk:high'))).toBe('kind:symbol,!risk:high')
  })
})
