import { afterEach, describe, expect, it, vi } from 'vitest'
import { callServerTool, callTool, RpcError, setActiveProjectId } from './rpc'

function mockFetch(response: unknown, ok = true, status = 200) {
  vi.stubGlobal(
    'fetch',
    vi.fn().mockResolvedValue({
      ok,
      status,
      json: () => Promise.resolve(response),
    }),
  )
}

describe('callTool', () => {
  afterEach(() => {
    setActiveProjectId(undefined)
    vi.unstubAllGlobals()
  })

  it('attaches the selected project id without exposing a repository path', async () => {
    mockFetch({
      jsonrpc: '2.0', id: 1,
      result: { content: [{ type: 'text', text: '{}' }] },
    })
    setActiveProjectId('web')

    await callTool('get_graph_schema')

    const body = JSON.parse((vi.mocked(fetch).mock.calls[0][1] as RequestInit).body as string)
    expect(body.params.project_id).toBe('web')
    expect(JSON.stringify(body)).not.toContain('/repos/')
  })

  it('keeps server-wide discovery project agnostic', async () => {
    mockFetch({
      jsonrpc: '2.0', id: 1,
      result: { content: [{ type: 'text', text: '[]' }] },
    })
    setActiveProjectId('web')

    await callServerTool('list_projects')

    const body = JSON.parse((vi.mocked(fetch).mock.calls[0][1] as RequestInit).body as string)
    expect(body.params.project_id).toBeUndefined()
  })

  it('sends the tools/call JSON-RPC envelope and unwraps result.content[0].text', async () => {
    mockFetch({
      jsonrpc: '2.0',
      id: 1,
      result: { content: [{ type: 'text', text: JSON.stringify({ ok: true, value: 42 }) }] },
    })

    const result = await callTool<{ ok: boolean; value: number }>('get_graph_schema', { a: 1 })

    expect(result).toEqual({ ok: true, value: 42 })
    const [url, init] = vi.mocked(fetch).mock.calls[0]
    expect(url).toBe('/rpc')
    const body = JSON.parse((init as RequestInit).body as string)
    expect(body).toMatchObject({
      jsonrpc: '2.0',
      method: 'tools/call',
      params: { name: 'get_graph_schema', arguments: { a: 1 } },
    })
  })

  it('throws RpcError with the server-reported code on a JSON-RPC error', async () => {
    mockFetch({
      jsonrpc: '2.0',
      id: 1,
      error: { code: -32601, message: 'unknown tool' },
    })

    await expect(callTool('nonexistent_tool')).rejects.toMatchObject(
      new RpcError('unknown tool', -32601),
    )
  })

  it('throws RpcError on a non-2xx HTTP response', async () => {
    mockFetch({}, false, 504)

    await expect(callTool('get_graph_schema')).rejects.toMatchObject({ code: 504 })
  })

  it('throws RpcError when the response has no content[0].text', async () => {
    mockFetch({ jsonrpc: '2.0', id: 1, result: {} })

    await expect(callTool('get_graph_schema')).rejects.toBeInstanceOf(RpcError)
  })
})
