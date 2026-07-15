import { callTool } from './rpc'

export interface TraceNode { id: string; label: string; name: string; file_path: string | null; in_degree: number; out_degree: number }
export interface TraceResult { root: TraceNode; visited: Array<{ node: TraceNode; hop: number }>; relations: Array<{ source: string; target: string; kind: string }> }
export function tracePath(query: string, depth = 2): Promise<TraceResult> { return callTool('trace_path', { query, depth, direction: 'both' }) }
export function impactAnalysis(query: string, depth = 2): Promise<TraceResult> { return callTool('impact_analysis', { query, depth }) }
