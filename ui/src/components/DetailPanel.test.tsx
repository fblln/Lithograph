import { cleanup, render, screen } from '@testing-library/react'
import { afterEach, describe, expect, it } from 'vitest'
import { DetailPanel } from './DetailPanel'

describe('DetailPanel display root', () => {
  afterEach(cleanup)

  it('shortens display labels while retaining the full id and tooltips', () => {
    const hash = '0123456789abcdef0123456789abcdef'
    const root = `.cache/${hash}/`
    const fullPath = `${root}src/api.ts`
    const id = `artifact:${fullPath}`
    render(<DetailPanel node={{ id, label: 'Artifact', name: fullPath, file_path: fullPath, in_degree: 0, out_degree: 1, x: 0, y: 0, hop: 0 }} detail={null} detailError={null} displayRootPrefix={root} onFocus={() => {}} onClear={() => {}} />)

    expect(screen.getAllByText('src/api.ts').some((item) => item.getAttribute('title') === fullPath)).toBe(true)
    expect(screen.getByText(id)).toBeInTheDocument()
    expect(screen.queryByText(fullPath)).not.toBeInTheDocument()
  })
})
