export type TagMode = 'include' | 'exclude'

export function parseExpression(expression: string): Map<string, TagMode> {
  const selected = new Map<string, TagMode>()
  for (const token of expression.split(',').map((item) => item.trim()).filter(Boolean)) {
    selected.set(token.startsWith('!') ? token.slice(1) : token, token.startsWith('!') ? 'exclude' : 'include')
  }
  return selected
}

export function serializeExpression(selected: Map<string, TagMode>): string {
  return [...selected].sort(([a], [b]) => a.localeCompare(b)).map(([tag, mode]) => mode === 'exclude' ? `!${tag}` : tag).join(',')
}
