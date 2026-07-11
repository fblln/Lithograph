import { callTool } from './rpc'
import type { LayoutRequest, LayoutResult } from '../graph/types'

export function getGraphLayout(request: LayoutRequest = {}): Promise<LayoutResult> {
  return callTool<LayoutResult>('get_graph_layout', request)
}
