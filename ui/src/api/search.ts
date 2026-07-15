import { callTool } from './rpc'

export interface GraphSearchResult {
  id: string
  label: string
  name: string
  file_path: string | null
  in_degree: number
  out_degree: number
}

export function searchGraph(query: string): Promise<GraphSearchResult[]> {
  return callTool<GraphSearchResult[]>('search_graph', { query, limit: 12 })
}
