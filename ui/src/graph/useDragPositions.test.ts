import { act } from 'react'
import { renderHook } from '@testing-library/react'
import { afterEach, beforeAll, beforeEach, describe, expect, it } from 'vitest'
import type { DragPositions } from './useDragPositions'
import { useDragPositions } from './useDragPositions'

// Mirrors the private `storageKeyFor` template in useDragPositions.ts, so
// tests can seed/inspect localStorage directly without exporting an
// internal helper from the module.
function storageKeyFor(persistKey: string): string {
  return `lithograph-drag-positions:${persistKey}`
}

// Newer Node runtimes ship their own experimental global `localStorage`
// (behind an unset `--localstorage-file` flag, where it resolves to
// `undefined` rather than throwing). Vitest's jsdom environment sees that
// global already exists and skips copying jsdom's real, working
// `localStorage` polyfill over it, so `window.localStorage` can end up
// `undefined` under jsdom depending on the Node version running the
// suite. Vitest still exposes the live JSDOM instance as `globalThis.jsdom`
// regardless, so fall back to its real storage implementation when that
// shadowing happens -- this only patches the test's global, never touches
// the hook module itself.
beforeAll(() => {
  if (typeof window.localStorage !== 'undefined') return
  const jsdomGlobal = (globalThis as unknown as { jsdom?: { window: { localStorage: Storage } } })
    .jsdom
  if (!jsdomGlobal) return
  Object.defineProperty(window, 'localStorage', {
    value: jsdomGlobal.window.localStorage,
    configurable: true,
  })
})

describe('useDragPositions', () => {
  beforeEach(() => {
    window.localStorage.clear()
  })

  afterEach(() => {
    window.localStorage.clear()
  })

  it('sets and gets an override, and reflects it via hasOverride', () => {
    const { result } = renderHook(() => useDragPositions('snapshot-a'))

    expect(result.current.hasOverride('node-1')).toBe(false)
    expect(result.current.getOverride('node-1')).toBeUndefined()

    act(() => {
      result.current.setOverride('node-1', [1, 2, 3])
    })

    expect(result.current.getOverride('node-1')).toEqual([1, 2, 3])
    expect(result.current.hasOverride('node-1')).toBe(true)
  })

  it('clearOverride removes just one node, leaving others intact', () => {
    const { result } = renderHook(() => useDragPositions('snapshot-a'))

    act(() => {
      result.current.setOverride('node-1', [1, 2, 3])
      result.current.setOverride('node-2', [4, 5, 6])
    })

    act(() => {
      result.current.clearOverride('node-1')
    })

    expect(result.current.hasOverride('node-1')).toBe(false)
    expect(result.current.getOverride('node-1')).toBeUndefined()
    expect(result.current.getOverride('node-2')).toEqual([4, 5, 6])
  })

  it('clearAll removes every override', () => {
    const { result } = renderHook(() => useDragPositions('snapshot-a'))

    act(() => {
      result.current.setOverride('node-1', [1, 2, 3])
      result.current.setOverride('node-2', [4, 5, 6])
    })

    act(() => {
      result.current.clearAll()
    })

    expect(result.current.hasOverride('node-1')).toBe(false)
    expect(result.current.hasOverride('node-2')).toBe(false)
  })

  it('persists overrides across an independent remount for the same key', () => {
    const first = renderHook(() => useDragPositions('snapshot-persist'))

    act(() => {
      first.result.current.setOverride('node-1', [7, 8, 9])
    })

    first.unmount()

    const second = renderHook(() => useDragPositions('snapshot-persist'))

    expect(second.result.current.getOverride('node-1')).toEqual([7, 8, 9])
  })

  it('isolates overrides between different persistKeys', () => {
    const a = renderHook(() => useDragPositions('snapshot-x'))

    act(() => {
      a.result.current.setOverride('shared-node-id', [1, 1, 1])
    })

    const b = renderHook(() => useDragPositions('snapshot-y'))

    expect(b.result.current.hasOverride('shared-node-id')).toBe(false)
    expect(b.result.current.getOverride('shared-node-id')).toBeUndefined()
    // The original key's data is untouched by rendering a hook under a
    // different key.
    expect(a.result.current.getOverride('shared-node-id')).toEqual([1, 1, 1])
  })

  it('starts empty and does not throw when localStorage holds corrupt JSON', () => {
    window.localStorage.setItem(storageKeyFor('snapshot-corrupt'), 'not valid json')

    let result: { current: DragPositions } | undefined
    expect(() => {
      ;({ result } = renderHook(() => useDragPositions('snapshot-corrupt')))
    }).not.toThrow()

    expect(result!.current.hasOverride('anything')).toBe(false)
    expect(result!.current.getOverride('anything')).toBeUndefined()
  })
})
