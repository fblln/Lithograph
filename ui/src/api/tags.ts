import { callTool } from './rpc'

export interface GraphTag {
  id: string
  entity_id: string
  namespace: string
  value: string
  source: string
  confidence: string
  evidence: string[]
  inherited_from: string | null
  graph_snapshot_id: string
}

export function getTagFacets(): Promise<Record<string, number>> {
  return callTool('get_tag_facets')
}

export function resolveTagExpression(expression: string): Promise<string[]> {
  return callTool('resolve_tag_expression', { expression })
}

export function listTags(): Promise<GraphTag[]> {
  return callTool('list_tags')
}
