/** Extract a generated section body while removing machine-readable evidence comments. */
export function sectionBody(markdown: string, title: string): string {
  const marker = `## ${title}`
  const start = markdown.indexOf(marker)
  if (start < 0) return 'Generated content is not available for this section.'
  const bodyStart = start + marker.length
  const next = markdown.indexOf('\n## ', bodyStart)
  return markdown.slice(bodyStart, next < 0 ? undefined : next).replace(/<!--[^]*?-->/g, '').trim()
}
