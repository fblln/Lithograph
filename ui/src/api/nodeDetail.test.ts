import { describe, expect, it, vi } from 'vitest'
import { getNodeDetail } from './nodeDetail'

describe('getNodeDetail', () => {
  it('rejects an unsupported-tool response instead of treating it as node detail', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        result: {
          content: [{ text: JSON.stringify({ available_tools: ['get_graph_layout'], message: 'unknown tool `get_node_detail`' }) }],
        },
      }),
    }))

    await expect(getNodeDetail('symbol:missing')).rejects.toThrow('Node evidence is not available')
  })
})
