import { callTool } from './rpc'

export interface GraphDocumentSection {
  id: string
  kind: string
  title: string
  source_query_ids: string[]
  evidence_references: string[]
  affected_nodes: string[]
  affected_edges: string[]
  confidence: string
  graph_snapshot_id: string
  deep_link_target: string
  tags: Array<{ id: string; namespace: string; value: string; source: string; confidence: string }>
}

export interface GraphDocumentResult {
  document: { id: string; graph_snapshot_id: string; schema_version: number; sections: GraphDocumentSection[] }
  markdown: string
  freshness: 'current' | 'stale'
  section_freshness?: Array<{ section_id: string; status: 'current' | 'partially_stale' | 'stale'; source_query_hash: string; evidence_hash: string; prompt_context_version: number; drift_findings: string[] }>
  diff?: Array<{ section_id: string; title: string; before?: string; after: string }>
  regenerated: boolean
}

export function getGraphDocument(): Promise<GraphDocumentResult> {
  return callTool('get_graph_document')
}

export function regenerateGraphDocument(sectionIds?: string[]): Promise<GraphDocumentResult> {
  return callTool('regenerate_graph_document', sectionIds ? { section_ids: sectionIds } : {})
}
