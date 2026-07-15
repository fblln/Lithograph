import { describe, expect, it } from 'vitest'
import { deriveClusterIdentities, humanClusterNameFromEvidence } from './clusterIdentity'
import type { VisualCluster } from './graph/clusterLayout'
import type { PositionedNode } from './graph/types'

const node = (id: string, path: string, label = 'Artifact'): PositionedNode => ({ id, label, name: path, file_path: path, in_degree: 1, out_degree: 2, x: 0, y: 0, hop: 0 })

describe('cluster identity', () => {
  it('derives deterministic human names from paths rather than exposing raw IDs', () => {
    const cluster: VisualCluster = { id: 'cluster:artifact:web/src/App.tsx', members: ['app'], totalMembers: 1, center: [0, 0, 0], radius: 1 }
    const identities = deriveClusterIdentities([cluster], [node('app', 'web/src/App.tsx')], [])

    expect(identities.get(cluster.id)?.name).toBe('Web frontend')
    expect(identities.get(cluster.id)?.responsibility).toContain('Web frontend groups')
  })

  it('uses representative evidence for a stable fallback name', () => {
    expect(humanClusterNameFromEvidence({
      id: 'cluster:artifact:src/python_app/service.py',
      members: ['service'],
      top_nodes: [{ id: 'service', label: 'Artifact', name: 'service.py', file_path: 'src/python_app/service.py', in_degree: 2, out_degree: 8 }],
    })).toBe('Python App subsystem')
    expect(humanClusterNameFromEvidence({ id: 'cluster:opaque', members: [] })).toBe('Opaque subsystem')
  })

  it('reports partial rendering, dependencies, tensions, and plain-language pressure', () => {
    const cluster: VisualCluster = {
      id: 'api', members: ['api'], totalMembers: 4, center: [0, 0, 0], radius: 1,
      analyticalCluster: { id: 'api', members: ['api', 'hidden-1', 'hidden-2', 'hidden-3'], top_nodes: [], packages: [], edge_types: ['Calls'], cohesion: 0.2, incoming_pressure: 5, outgoing_pressure: 7 },
    }
    const other: VisualCluster = { id: 'web', members: ['web'], totalMembers: 1, center: [2, 0, 0], radius: 1, fallbackKey: 'path:web' }
    const link = { source: 'web', target: 'api', count: 3, kinds: [{ kind: 'Calls', count: 3 }], underlying: [{ source: 'web', target: 'api', kind: 'Calls' }] }
    const result = deriveClusterIdentities([cluster, other], [node('api', 'src/api/main.py'), node('web', 'web/App.tsx')], [link], [], [{ id: 't1', category: 'CouplingHotspot', severity: 'High', confidence: 'High', affected_nodes: ['api'], metric_inputs: {}, evidence_references: [], follow_up_queries: [], explanation: 'signal' }])
    const identity = result.get('api')!

    expect(identity).toMatchObject({ visibleMemberCount: 1, memberCount: 4, partial: true, tensionCount: 1, highestSeverity: 'High' })
    expect(identity.incoming).toHaveLength(1)
    expect(identity.boundaryInterpretation).toContain('Loosely connected')
  })

  it('disambiguates repeated evidence-based names without raw IDs', () => {
    const clusters: VisualCluster[] = [
      { id: 'web-package', members: ['package'], totalMembers: 1, center: [0, 0, 0], radius: 1 },
      { id: 'web-app', members: ['app'], totalMembers: 1, center: [0, 0, 0], radius: 1 },
    ]
    const result = deriveClusterIdentities(clusters, [node('package', 'web/package.json'), node('app', 'web/src/App.tsx')], [])
    const names = [...result.values()].map((identity) => identity.name)
    expect(new Set(names).size).toBe(2)
    expect(names.every((name) => name.startsWith('Web frontend · '))).toBe(true)
  })
})
