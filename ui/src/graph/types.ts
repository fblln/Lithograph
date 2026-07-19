/**
 * Mirrors the JSON shape of `get_graph_layout`'s response
 * (src/graph/layout.rs::LayoutResult and friends). Field names match the
 * server's serde output verbatim (snake_case, since the Rust structs
 * already use snake_case field names and carry no `#[serde(rename_all)]`).
 */

export interface PositionedNode {
  id: string
  label: string
  name: string
  file_path: string | null
  in_degree: number
  out_degree: number
  x: number
  y: number
  hop: number
}

export interface LayoutEdge {
  id?: string
  source: string
  target: string
  kind: string
  resolution?: 'HybridResolved' | 'SyntaxOnly' | 'Fallback'
  confidence?: 'Low' | 'High'
  resolver_strategy?: string | null
  count?: number
  kinds?: Array<{ kind: string; count: number }>
}

export interface LayoutBudget {
  node_budget: number
  edge_budget: number
  nodes_available: number
  edges_available: number
  nodes_returned: number
  edges_returned: number
  nodes_truncated: boolean
  edges_truncated: boolean
}

export interface LayoutResult {
  graph_snapshot_id: string
  algorithm_version: number
  center_node: string | null
  nodes: PositionedNode[]
  edges: LayoutEdge[]
  budget: LayoutBudget
}

export interface LayoutRequest {
  center_node?: string
  radius?: number
  max_nodes?: number
  max_edges?: number
  node_labels?: string[]
  node_ids?: string[]
  edge_types?: string[]
  // LIT-84: exclude Unresolved nodes and their edges from the slice. Off by default.
  hide_unresolved?: boolean
}
