import { describe, expect, it } from 'vitest'
import { deriveDisplayRootPrefix, stripDisplayRoot } from './displayRoot'

const HASH = '0123456789abcdef0123456789abcdef'

describe('display root', () => {
  it('derives and strips only a shared hash-anchored root', () => {
    const root = deriveDisplayRootPrefix([
      { id: 'a', file_path: `.cache/${HASH}/src/api.ts` },
      { id: 'b', file_path: `.cache/${HASH}/tests/api.test.ts` },
    ])
    expect(root).toBe(`.cache/${HASH}/`)
    expect(stripDisplayRoot(`.cache/${HASH}/src/api.ts`, root)).toBe('src/api.ts')
    expect(stripDisplayRoot(`symbol:.cache/${HASH}/src/api.ts#run`, root)).toBe('symbol:src/api.ts#run')
  })

  it('does not strip an ordinary common directory or a non-boundary match', () => {
    expect(deriveDisplayRootPrefix([{ id: 'a', file_path: 'src/a.ts' }, { id: 'b', file_path: 'src/b.ts' }])).toBeUndefined()
    expect(stripDisplayRoot(`x/.cache/${HASH}/src/a.ts`, `.cache/${HASH}/`)).toBe(`x/.cache/${HASH}/src/a.ts`)
  })
})
