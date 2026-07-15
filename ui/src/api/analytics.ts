import { callTool } from './rpc'
export interface AnalyticsNode { id: string; fan_in: number; fan_out: number; page_rank: number; betweenness: number }
export interface HealthFinding { id: string; rule: string; severity: string; affected_nodes: string[]; evidence: string[]; investigation_query: string }
export interface Analytics { nodes: AnalyticsNode[]; findings: HealthFinding[] }
export function getGraphAnalytics(): Promise<Analytics> { return callTool('get_graph_analytics') }
