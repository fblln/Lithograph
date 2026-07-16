import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { ProvenanceTags } from './ProvenanceTags'

describe('ProvenanceTags', () => {
  afterEach(cleanup)

  it('shows source, confidence, evidence, and inheritance provenance', () => {
    render(<ProvenanceTags tags={[{
      id: 'tag:owner', entity_id: 'symbol:a', namespace: 'owner', value: 'payments',
      source: 'User', confidence: 'High', evidence: ['ADR-0001'],
      inherited_from: 'tag:cluster-owner', graph_snapshot_id: 'blake3:graph',
    }]} />)

    const tag = screen.getByText(/#owner\/payments/)
    expect(tag).toHaveTextContent('User · High · evidence 1 · inherited')
    expect(tag).toHaveAttribute('title', expect.stringContaining('evidence: ADR-0001'))
    expect(tag).toHaveAttribute('title', expect.stringContaining('inherited from tag:cluster-owner'))
  })
})
