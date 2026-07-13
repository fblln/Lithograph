import { useCallback, useEffect, useState } from 'react'

/**
 * Public API returned by `useDragPositions`. A plain object (not the
 * underlying `Map`) so consumers get stable method identities and can't
 * reach in and mutate storage state directly.
 */
export interface DragPositions {
  getOverride(nodeId: string): [number, number, number] | undefined
  setOverride(nodeId: string, position: [number, number, number]): void
  clearOverride(nodeId: string): void
  clearAll(): void
  hasOverride(nodeId: string): boolean
}

type OverrideRecord = Record<string, [number, number, number]>

function storageKeyFor(persistKey: string): string {
  return `lithograph-drag-positions:${persistKey}`
}

/**
 * Reads and parses overrides for a given persist key. Any failure --
 * localStorage unavailable (SSR/older private-browsing), missing entry, or
 * corrupt JSON left over from a previous schema -- degrades to an empty
 * map rather than throwing, since a lost drag override is much cheaper
 * than a crashed graph view.
 */
function loadOverrides(persistKey: string): Map<string, [number, number, number]> {
  try {
    if (typeof window === 'undefined' || !window.localStorage) return new Map()
    const raw = window.localStorage.getItem(storageKeyFor(persistKey))
    if (!raw) return new Map()
    const parsed: unknown = JSON.parse(raw)
    if (typeof parsed !== 'object' || parsed === null) return new Map()
    const entries = Object.entries(parsed as OverrideRecord).filter(
      (entry): entry is [string, [number, number, number]] =>
        Array.isArray(entry[1]) &&
        entry[1].length === 3 &&
        entry[1].every((n) => typeof n === 'number'),
    )
    return new Map(entries)
  } catch {
    return new Map()
  }
}

function saveOverrides(persistKey: string, overrides: Map<string, [number, number, number]>): void {
  try {
    if (typeof window === 'undefined' || !window.localStorage) return
    const record: OverrideRecord = Object.fromEntries(overrides)
    window.localStorage.setItem(storageKeyFor(persistKey), JSON.stringify(record))
  } catch {
    // Write can fail (private browsing quota, disabled storage); an
    // override that fails to persist just falls back to in-memory-only
    // for this session, which is an acceptable degradation.
  }
}

export function useDragPositions(persistKey: string): DragPositions {
  const [overrides, setOverrides] = useState<Map<string, [number, number, number]>>(
    () => new Map(),
  )

  // Reload from storage whenever the scope (graph snapshot) changes, so
  // overrides from a previous graph never leak into a new one.
  useEffect(() => {
    setOverrides(loadOverrides(persistKey))
  }, [persistKey])

  const getOverride = useCallback(
    (nodeId: string) => overrides.get(nodeId),
    [overrides],
  )

  const hasOverride = useCallback((nodeId: string) => overrides.has(nodeId), [overrides])

  const setOverride = useCallback(
    (nodeId: string, position: [number, number, number]) => {
      setOverrides((prev) => {
        const next = new Map(prev)
        next.set(nodeId, position)
        saveOverrides(persistKey, next)
        return next
      })
    },
    [persistKey],
  )

  const clearOverride = useCallback(
    (nodeId: string) => {
      setOverrides((prev) => {
        if (!prev.has(nodeId)) return prev
        const next = new Map(prev)
        next.delete(nodeId)
        saveOverrides(persistKey, next)
        return next
      })
    },
    [persistKey],
  )

  const clearAll = useCallback(() => {
    setOverrides(() => {
      const next = new Map<string, [number, number, number]>()
      saveOverrides(persistKey, next)
      return next
    })
  }, [persistKey])

  return { getOverride, setOverride, clearOverride, clearAll, hasOverride }
}
