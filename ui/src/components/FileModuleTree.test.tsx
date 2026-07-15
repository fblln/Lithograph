import { cleanup, render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { FileModuleTree } from './FileModuleTree'

describe('FileModuleTree', () => {
  afterEach(cleanup)
  const nodes = [
    { id: 'artifact:src/api.rs', label: 'Artifact', name: 'api.rs', file_path: 'src/api.rs', in_degree: 0, out_degree: 0, x: 0, y: 0, hop: 0 },
    { id: 'symbol:src/api.rs:run', label: 'Symbol', name: 'run', file_path: 'src/api.rs', in_degree: 0, out_degree: 0, x: 0, y: 0, hop: 0 },
    { id: 'module:src/graph', label: 'Module', name: 'graph', file_path: 'src/graph/mod.rs', in_degree: 0, out_degree: 0, x: 0, y: 0, hop: 0 },
    { id: 'unresolved:map', label: 'Unresolved', name: 'map', file_path: null, in_degree: 0, out_degree: 0, x: 0, y: 0, hop: 0 },
  ]

  it('nests the bounded layout slice, filters it, and focuses a selected item', async () => {
    const focus = vi.fn()
    const user = userEvent.setup()
    render(<FileModuleTree nodes={nodes} onFocusNode={focus} />)
    expect(screen.getByText('src')).toBeInTheDocument()
    expect(screen.getAllByRole('button', { name: /src\/api.rs/ })).toHaveLength(1)
    expect(screen.queryByRole('button', { name: /map.*Unresolved/ })).not.toBeInTheDocument()
    await user.click(screen.getByRole('button', { name: /src\/api.rs.*Artifact.*\+1/ }))
    expect(focus).toHaveBeenCalledWith('artifact:src/api.rs')
    await user.type(screen.getByLabelText('Filter files and modules'), 'graph')
    expect(screen.queryByRole('button', { name: /src\/api.rs/ })).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /src\/graph\/mod.rs.*Module/ })).toBeInTheDocument()
  })
})
