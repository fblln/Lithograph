import { callTool, RpcError } from './rpc'

export interface NodeEvidence {
  path: string
  start_line: number | null
  end_line: number | null
}

export interface RelatedNode {
  id: string
  label: string
  name: string
}

export interface RelatedRelation {
  id: string
  direction: 'inbound' | 'outbound'
  kind: string
  counterpart: RelatedNode
  evidence: NodeEvidence[]
  resolver_strategy: string | null
  confidence: 'Low' | 'High'
}

export interface NodeDetail {
  id: string
  label: string
  name: string
  evidence: NodeEvidence[]
  source: { status: 'available' | 'missing' | 'opaque'; text: string | null; message: string | null }
  definitions: RelatedNode[]
  references: RelatedRelation[]
  related_docs: RelatedNode[]
  tags: Array<{ id: string; namespace: string; value: string; source: string; confidence: string; evidence: string[]; inherited_from: string | null }>
}

export async function getNodeDetail(nodeId: string): Promise<NodeDetail> {
  const result = await callTool<unknown>('get_node_detail', { node_id: nodeId })
  if (!isNodeDetail(result)) {
    // Older/smaller server builds do not expose this optional evidence tool.
    // Treat their tool-list response as an unavailable detail panel rather
    // than trusting it as NodeDetail and crashing the entire React tree.
    throw new RpcError('Node evidence is not available from this server.', -32601)
  }
  return result
}

function isNodeDetail(value: unknown): value is NodeDetail {
  if (typeof value !== 'object' || value === null) return false
  const detail = value as Partial<NodeDetail>
  return typeof detail.id === 'string' &&
    typeof detail.label === 'string' &&
    typeof detail.name === 'string' &&
    Array.isArray(detail.evidence) &&
    Array.isArray(detail.definitions) &&
    Array.isArray(detail.references) &&
    Array.isArray(detail.related_docs) &&
    Array.isArray(detail.tags) &&
    typeof detail.source === 'object' && detail.source !== null
}
