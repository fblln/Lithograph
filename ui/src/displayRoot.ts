export interface DisplayRootEntity {
  id: string
  name?: string
  file_path?: string | null
}

/** Finds a shared path prefix ending in a hash-like directory. Ordinary
 * repository roots such as `src/` are intentionally never stripped. */
export function deriveDisplayRootPrefix(entities: DisplayRootEntity[]): string | undefined {
  const paths = entities.map((entity) => entity.file_path).filter((path): path is string => Boolean(path)).map(pathSegments)
  if (paths.length === 0) return undefined
  const common: string[] = []
  for (let index = 0; index < Math.min(...paths.map((parts) => parts.length)); index += 1) {
    const segment = paths[0][index]
    if (!paths.every((parts) => parts[index] === segment)) break
    common.push(segment)
  }
  const hashIndex = common.findLastIndex(isHashRoot)
  return hashIndex < 0 ? undefined : `${common.slice(0, hashIndex + 1).join('/')}/`
}

/** Removes a derived root only at a path boundary or immediately after an
 * entity-kind prefix. The underlying value is never mutated. */
export function stripDisplayRoot(value: string, rootPrefix?: string): string {
  if (!rootPrefix) return value
  const normalized = value.startsWith('/') ? value.slice(1) : value
  if (normalized.startsWith(rootPrefix)) return normalized.slice(rootPrefix.length)
  const colon = normalized.indexOf(':')
  if (colon >= 0 && normalized.slice(colon + 1).startsWith(rootPrefix)) {
    return `${normalized.slice(0, colon + 1)}${normalized.slice(colon + 1 + rootPrefix.length)}`
  }
  return value
}

function pathSegments(path: string): string[] {
  return path.replaceAll('\\', '/').split('/').filter(Boolean)
}

function isHashRoot(segment: string): boolean {
  return /^(?:[a-f\d]{12,128}|(?:sha256|blake3)-[a-f\d]{12,128})$/i.test(segment)
}
