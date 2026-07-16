/**
 * Client for the JSON-RPC 2.0 `tools/call` envelope implemented by the
 * embedded Lithograph server (Rust src/serve.rs). One call = one tool
 * name plus its arguments; the result is JSON-encoded text at
 * `result.content[0].text`, matching the shape the server's
 * `rpc_handler` produces.
 */

export class RpcError extends Error {
  readonly code: number

  constructor(message: string, code: number) {
    super(message)
    this.name = 'RpcError'
    this.code = code
  }
}

let nextId = 1
let activeProjectId: string | undefined

/** Selects the allowlisted project attached to subsequent tool calls. */
export function setActiveProjectId(projectId: string | undefined) {
  activeProjectId = projectId && projectId !== 'primary' ? projectId : undefined
}

async function callToolForProject<T>(name: string, args: unknown, signal: AbortSignal | undefined, projectId: string | undefined): Promise<T> {
  const id = nextId++
  const response = await fetch('/rpc', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    signal,
    body: JSON.stringify({
      jsonrpc: '2.0',
      id,
      method: 'tools/call',
      params: { name, arguments: args, ...(projectId ? { project_id: projectId } : {}) },
    }),
  })
  if (!response.ok) {
    throw new RpcError(`HTTP ${response.status} calling ${name}`, response.status)
  }
  const body = await response.json()
  if (body.error) {
    throw new RpcError(body.error.message ?? `${name} failed`, body.error.code ?? -32000)
  }
  const text = body.result?.content?.[0]?.text
  if (typeof text !== 'string') {
    throw new RpcError(`malformed response from ${name}`, -32603)
  }
  return JSON.parse(text) as T
}

export async function callTool<T>(name: string, args: unknown = {}, signal?: AbortSignal): Promise<T> {
  return callToolForProject<T>(name, args, signal, activeProjectId)
}

/** Calls a server-wide tool such as project discovery without project routing. */
export async function callServerTool<T>(name: string, args: unknown = {}, signal?: AbortSignal): Promise<T> {
  return callToolForProject<T>(name, args, signal, undefined)
}
