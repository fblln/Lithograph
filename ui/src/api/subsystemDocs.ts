import { callTool } from './rpc'

export interface SubsystemDocument {
  subsystem_id: string
  graph_snapshot_id: string
  prompt_version: string
  confidence: string
  cited_nodes: string[]
  cited_edges: string[]
  source_spans: string[]
  unresolved_assumptions: string[]
  markdown: string
  resolved_tags: Array<{ id: string; namespace: string; value: string; source: string }>
  tag_expression?: string
}

export function generateSubsystemDocument(subsystem: string, nodeIds: string[], instruction?: string): Promise<SubsystemDocument> {
  return callTool('generate_subsystem_document', { subsystem, node_ids: nodeIds, ...(instruction ? { instruction } : {}) })
}

export function refineSubsystemDocument(subsystem: string, nodeIds: string[], instruction: string): Promise<SubsystemDocument> {
  return callTool('refine_subsystem_document', { subsystem, node_ids: nodeIds, instruction })
}
