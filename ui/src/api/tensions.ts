import { callTool } from './rpc'
import type { GraphTag } from './tags'

export interface RepositoryTension { id: string; category: string; severity: string; confidence: string; affected_nodes: string[]; metric_inputs: Record<string, number>; evidence_references: string[]; follow_up_queries: string[]; explanation: string; tags?: GraphTag[] }
export async function getRepositoryTensions(): Promise<RepositoryTension[]> {
  const result = await callTool<unknown>('get_repository_tensions')
  return Array.isArray(result) ? result as RepositoryTension[] : []
}
