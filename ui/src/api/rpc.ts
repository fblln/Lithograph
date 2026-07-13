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

export async function callTool<T>(name: string, args: unknown = {}): Promise<T> {
  const id = nextId++
  const response = await fetch('/rpc', {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      jsonrpc: '2.0',
      id,
      method: 'tools/call',
      params: { name, arguments: args },
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
