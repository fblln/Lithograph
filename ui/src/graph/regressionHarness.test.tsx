import { afterEach, describe, expect, it, vi } from 'vitest'
import { act, cleanup, render, screen } from '@testing-library/react'
import type { ReactNode } from 'react'
import { GraphScene } from './GraphScene'
import { cameraFrameForPositions } from './cameraFrame'
import { computeMatrixPositions } from './matrixLayout'
import { edgeFadeOpacity } from './positions'
import { largeGraphFixture, mediumGraphFixture, smallGraphFixture } from '../testdata/graphFixtures'

// WebGL itself belongs to the browser driver, while this contract test proves
// that a populated layout reaches the canvas and its batched renderers. The
// lightweight stand-ins make it deterministic and suitable for normal CI.
vi.mock('@react-three/fiber', () => ({ Canvas: ({ children }: { children: ReactNode }) => <div data-testid="graph-canvas">{children}</div> }))
vi.mock('@react-three/drei', () => ({ OrbitControls: () => <div data-testid="orbit-controls" />, Html: ({ children }: { children: ReactNode }) => <>{children}</> }))
vi.mock('./NodeCloud', () => ({ NodeCloud: ({ nodes }: { nodes: unknown[] }) => <div data-testid="node-cloud">{nodes.length}</div> }))
vi.mock('./EdgeLines', () => ({ EdgeLines: ({ edges }: { edges: unknown[] }) => <div data-testid="edge-lines">{edges.length}</div> }))
vi.mock('./ClusterHulls', () => ({ ClusterHulls: () => <div data-testid="cluster-hulls" /> }))

describe('graph explorer regression harness', () => {
  afterEach(() => {
    cleanup()
    vi.unstubAllGlobals()
  })
  it.each([
    ['small', smallGraphFixture],
    ['medium', mediumGraphFixture],
    ['large', largeGraphFixture],
  ])('keeps the %s fixture complete and deterministic', (_name, fixture) => {
    const layout = fixture()
    const first = computeMatrixPositions(layout.nodes)
    expect(layout.budget.nodes_returned).toBe(layout.nodes.length)
    expect(layout.budget.edges_returned).toBe(layout.edges.length)
    expect(first).toEqual(computeMatrixPositions(layout.nodes))
    expect(first.size).toBe(layout.nodes.length)
    expect(edgeFadeOpacity(layout.edges.length)).toBeGreaterThan(0)
  })

  it('routes a non-empty large graph to one canvas with batched nodes and edges', () => {
    const layout = largeGraphFixture()
    render(<GraphScene layout={layout} viewMode="radial" clusters={[]} selectedId={null} onSelect={() => {}} dragPositions={{ getOverride: () => undefined, setOverride: () => {}, clearOverride: () => {}, clearAll: () => {}, hasOverride: () => false }} />)
    expect(screen.getByTestId('graph-canvas')).not.toBeEmptyDOMElement()
    expect(screen.getByTestId('node-cloud')).toHaveTextContent('1000')
    expect(screen.getByTestId('edge-lines')).toHaveTextContent('999')
  })

  it('applies relationship-kind filters before rendering edges', () => {
    const layout = smallGraphFixture()
    render(<GraphScene layout={layout} viewMode="radial" clusters={[]} selectedId={null} onSelect={() => {}} edgeKinds={new Set(['Calls'])} dragPositions={{ getOverride: () => undefined, setOverride: () => {}, clearOverride: () => {}, clearAll: () => {}, hasOverride: () => false }} />)
    const expected = layout.edges.filter((edge) => edge.kind === 'Calls').length
    expect(screen.getByTestId('edge-lines')).toHaveTextContent(String(expected))
  })

  it('frames repository-scale layouts to fit each axis without the old empty-canvas margin', () => {
    const frame = cameraFrameForPositions(new Map([
      ['left', [-40, 0, -20]],
      ['right', [40, 0, 20]],
    ]))
    expect(frame.position[0]).toBe(0)
    // Far enough that both the 80-wide and 40-deep spans fit the frustum with
    // margin, but closer than the old flat `max-span * 1.6` (128 here), which
    // left wide layouts as a thin band in an empty canvas.
    expect(frame.position[1]).toBeGreaterThan(50)
    expect(frame.position[1]).toBeLessThan(128)
    expect(Math.abs(frame.position[2] - frame.target[2])).toBeLessThan(frame.position[1] * 0.1)
    expect(frame.far).toBeGreaterThan(1000)
    expect(frame.position.every(Number.isFinite)).toBe(true)
  })

  it('backs the camera out for a deep layout so the depth axis also fits', () => {
    // Depth (Z) is limited by the vertical frustum, which is narrower than
    // the horizontal one at any aspect > 1, so a deep layout needs more
    // distance than an equally wide one.
    const wide = cameraFrameForPositions(new Map([
      ['left', [-40, 0, 0]],
      ['right', [40, 0, 0]],
    ]))
    const deep = cameraFrameForPositions(new Map([
      ['near', [0, 0, -40]],
      ['far', [0, 0, 40]],
    ]))
    expect(deep.position[1]).toBeGreaterThan(wide.position[1])
  })

  it('uses the current graph viewport aspect and reacts when an inspector narrows it', () => {
    let resize: ResizeObserverCallback | undefined
    vi.stubGlobal('ResizeObserver', class {
      constructor(callback: ResizeObserverCallback) { resize = callback }
      observe() {}
      unobserve() {}
      disconnect() {}
    })
    render(<GraphScene layout={smallGraphFixture()} viewMode="radial" clusters={[]} selectedId={null} onSelect={() => {}} dragPositions={{ getOverride: () => undefined, setOverride: () => {}, clearOverride: () => {}, clearAll: () => {}, hasOverride: () => false }} />)
    const viewport = screen.getByTestId('graph-scene-viewport')

    act(() => resize?.([{ contentRect: { width: 800, height: 600 } } as ResizeObserverEntry], {} as ResizeObserver))
    expect(viewport).toHaveAttribute('data-camera-aspect', String(800 / 600))

    act(() => resize?.([{ contentRect: { width: 480, height: 600 } } as ResizeObserverEntry], {} as ResizeObserver))
    expect(viewport).toHaveAttribute('data-camera-aspect', String(480 / 600))
  })

  it('backs the camera out when the measured viewport becomes narrower', () => {
    const positions = new Map([
      ['left', [-40, 0, 0] as [number, number, number]],
      ['right', [40, 0, 0] as [number, number, number]],
    ])
    const wide = cameraFrameForPositions(positions, 16 / 9)
    const narrow = cameraFrameForPositions(positions, 4 / 5)
    expect(narrow.position[1]).toBeGreaterThan(wide.position[1])
  })
})
