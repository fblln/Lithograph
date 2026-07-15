import { callTool } from './rpc'

export interface QueryRow { alias: string; id: string; label: string; name: string; file_path: string | null }
export interface GraphSchema { node_labels: Array<{ label: string; count: number }>; edge_types: Array<{ edge_type: string; count: number }>; relationship_patterns: string[] }

export function queryGraph(query: string): Promise<QueryRow[]> { return callTool<QueryRow[]>('query_graph', { query }) }
export function getGraphSchema(): Promise<GraphSchema> { return callTool<GraphSchema>('get_graph_schema') }
