// Atlas-level data for the Lithograph explorer: clusters, tensions, tags,
// docs sections, and saved investigations. Built on top of graph-data.js and
// grounded in the fixtures/polyglot fixture (snapshot 5bcfa51).

import { nodes, relations, nodeById } from './graph-data.js';

export const snapshot = {
  id: '5bcfa51',
  project: 'fixtures/polyglot',
  scannedFiles: 22,
  age: '2 min ago',
  layoutCache: 'hit (layout v3)',
};

// ---------------------------------------------------------------------------
// Clusters (Leiden-style communities, persisted summaries per LIT-24.23)
// ---------------------------------------------------------------------------
export const clusters = [
  {
    id: 'cl:python', label: 'Python service', short: 'service',
    color: 'oklch(0.78 0.16 55)',
    members: ['pkg:polyglot-fixture', 'pkg:pydantic', 'm:python_app', 'a:__init__.py', 'a:service.py', 's:RouteService', 's:RouteService.__init__', 's:RouteService.load_settings', 's:RouteService.bake_route', 's:run_worker', 'u:pathlib.Path', 'u:target/debug/worker', 'cmd:worker-invoke'],
    cohesion: 0.78, conductance: 0.21,
    bridgeNodes: ['s:run_worker'],
    dominantTags: ['lang:python', 'layer:service'],
    summary: 'RouteService loads route settings and shells out to the Rust worker binary through run_worker.',
  },
  {
    id: 'cl:rust', label: 'Rust worker', short: 'worker',
    color: 'oklch(0.72 0.16 25)',
    members: ['pkg:fixture-worker', 'pkg:anyhow', 'm:fixture_worker', 'm:worker_bin', 'a:Cargo.toml', 'a:lib.rs', 'a:worker.rs', 's:RouteBake', 's:RouteBaker', 's:RouteBaker.from_env', 's:RouteBaker.bake', 's:bake_route_fn', 's:main', 'e:RIDGELINE_CACHE_DIR'],
    cohesion: 0.84, conductance: 0.12,
    bridgeNodes: ['s:main'],
    dominantTags: ['lang:rust', 'layer:worker'],
    summary: 'fixture-worker crate: RouteBake trait, RouteBaker, and the worker binary invoked by the Python service.',
  },
  {
    id: 'cl:infra', label: 'Container infra', short: 'infra',
    color: 'oklch(0.75 0.13 200)',
    members: ['a:Dockerfile', 'a:docker-compose.yml', 'c:api', 'c:web', 'c:8080', 'img:rust', 'img:python', 'img:node', 'img:route-api-version', 'e:RIDGELINE_WORKER', 'e:VERSION', 'cmd:cargo-build', 'cmd:pip-install', 'cmd:python-cmd', 'cmd:npm-dev'],
    cohesion: 0.62, conductance: 0.34,
    bridgeNodes: ['e:RIDGELINE_WORKER'],
    dominantTags: ['runtime:docker', 'layer:infra'],
    summary: 'Multi-stage Dockerfile plus compose wiring: api service builds the image, web runs node:24-alpine.',
  },
  {
    id: 'cl:ci', label: 'CI pipeline', short: 'ci',
    color: 'oklch(0.72 0.14 150)',
    members: ['a:ci.yml', 'c:checks', 'cmd:pytest', 'cmd:cargo-test', 'cmd:npm-lint', 'cmd:docker-build', 'img:route-api-sha'],
    cohesion: 0.88, conductance: 0.1,
    bridgeNodes: ['c:checks'],
    dominantTags: ['layer:ci'],
    summary: 'One GitHub Actions job (checks) runs Python, Rust, and web checks, then builds the release image.',
  },
  {
    id: 'cl:config', label: 'Configuration', short: 'config',
    color: 'oklch(0.72 0.14 300)',
    members: ['a:settings.yaml', 'a:schema.json', 'img:worker-schema', 'a:pyproject.toml', 'a:requirements.txt'],
    cohesion: 0.31, conductance: 0.55,
    bridgeNodes: ['a:settings.yaml'],
    dominantTags: ['layer:config'],
    summary: 'YAML/JSON settings and package manifests. Mostly references other clusters; weak internal structure.',
  },
  {
    id: 'cl:docs', label: 'Docs & meta', short: 'docs',
    color: 'oklch(0.78 0.1 120)',
    members: ['a:README.md', 'd:readme.title', 'd:readme.categories', 'd:readme.offline', 'a:architecture.md', 'd:arch.title', 'a:LICENSE', 'a:Makefile'],
    cohesion: 0.7, conductance: 0.08,
    bridgeNodes: [],
    dominantTags: ['layer:docs'],
    summary: 'README and architecture notes, including the existing Mermaid diagram of service/worker/web flow.',
  },
  {
    id: 'cl:web', label: 'Web frontend', short: 'web',
    color: 'oklch(0.72 0.13 250)',
    members: ['a:App.tsx', 'a:index.html'],
    cohesion: 0.5, conductance: 0.05,
    bridgeNodes: [],
    dominantTags: ['lang:typescript', 'layer:web'],
    summary: 'TSX shell handled through generic-text fallback \u2014 no deep symbols until TypeScript support exists.',
  },
  {
    id: 'cl:unanalyzed', label: 'Unanalyzed', short: 'other',
    color: 'oklch(0.55 0.01 260)',
    members: ['a:vendor_lib.rs', 'a:client.py', 'a:sample.bin', 'a:logo.svg'],
    cohesion: 0.0, conductance: 0.0,
    bridgeNodes: [],
    dominantTags: ['status:excluded'],
    summary: 'Vendored, generated, binary, and asset files detected but excluded from deep analysis.',
  },
];

export const clusterOfNode = {};
for (const cl of clusters) for (const id of cl.members) clusterOfNode[id] = cl.id;
export const clusterById = Object.fromEntries(clusters.map((c) => [c.id, c]));

// Boundary edges per cluster (relations crossing the cluster boundary)
export function boundaryEdges(clusterId) {
  return relations.filter((r) => {
    const a = clusterOfNode[r.source], b = clusterOfNode[r.target];
    return (a === clusterId) !== (b === clusterId);
  });
}
export function crossClusterEdges() {
  return relations.filter((r) => clusterOfNode[r.source] !== clusterOfNode[r.target]);
}

// ---------------------------------------------------------------------------
// Tensions (LIT-24.30 contract)
// ---------------------------------------------------------------------------
export const SEVERITY = {
  high: { label: 'High', color: 'oklch(0.66 0.19 25)' },
  medium: { label: 'Medium', color: 'oklch(0.78 0.15 75)' },
  low: { label: 'Low', color: 'oklch(0.78 0.11 120)' },
};

export const tensions = [
  {
    id: 'tn1', label: 'Bridge bottleneck: Python \u2192 Rust hand-off', category: 'bridge-bottleneck',
    severity: 'high', confidence: 'High',
    clusterIds: ['cl:python', 'cl:rust'],
    affected: ['s:run_worker', 'cmd:worker-invoke', 'e:RIDGELINE_WORKER', 'u:target/debug/worker', 's:main', 's:RouteService.bake_route'],
    why: 'Every Python \u2192 Rust call funnels through one subprocess invocation in run_worker. The contract \u2014 CLI args, env resolution, binary path \u2014 is implicit: no typed interface, no test seam, and the binary path comes from an env var with a debug-build fallback.',
    metrics: ['betweenness(run_worker) \u2248 0.42', '1 of 1 cross-cluster call paths', 'fan-in 4 / fan-out 2'],
    evidence: [{ file: 'service.py', line: 26 }, { file: 'worker.rs', line: 3 }],
    suggested: ['Trace call path RouteService.bake_route \u2192 worker::main', 'List every reader of RIDGELINE_WORKER'],
  },
  {
    id: 'tn2', label: 'Drift risk: RIDGELINE_WORKER defined in 4 places', category: 'drift-risk',
    severity: 'medium', confidence: 'High',
    clusterIds: ['cl:python', 'cl:infra', 'cl:config'],
    affected: ['e:RIDGELINE_WORKER', 's:RouteService.__init__', 'a:Dockerfile', 'a:docker-compose.yml', 'a:settings.yaml'],
    why: 'The worker binary path has four independent sources of truth: the code default target/debug/worker, Dockerfile ENV, compose environment, and settings.yaml. They already disagree \u2014 the code falls back to a debug path while every deployment surface says /usr/local/bin/worker.',
    metrics: ['4 declaration sites', '2 distinct values'],
    evidence: [{ file: 'service.py', line: 14 }, { file: 'Dockerfile', line: 11 }, { file: 'docker-compose.yml', line: 9 }, { file: 'settings.yaml', line: 6 }],
    suggested: ['Show all References \u2192 RIDGELINE_WORKER', 'Diff declared values across artifacts'],
  },
  {
    id: 'tn3', label: 'Change concentration: image tags split across 3 files', category: 'change-concentration',
    severity: 'medium', confidence: 'Medium',
    clusterIds: ['cl:infra', 'cl:ci', 'cl:config'],
    affected: ['img:route-api-version', 'img:route-api-sha', 'img:worker-schema', 'a:docker-compose.yml', 'a:ci.yml', 'a:schema.json'],
    why: 'Three files each hold their own image reference: compose tags with ${VERSION}, CI tags with the commit SHA, and schema.json defaults to a third image entirely (worker:1.0). A release change touches all three.',
    metrics: ['3 image references', '2 dynamic template expressions'],
    evidence: [{ file: 'docker-compose.yml', line: 6 }, { file: 'ci.yml', line: 15 }, { file: 'schema.json', line: 9 }],
    suggested: ['List all Container nodes', 'Trace BuildsImage / UsesImage edges'],
  },
  {
    id: 'tn4', label: 'Orphaned sources: 4 files outside the graph', category: 'dead-code',
    severity: 'low', confidence: 'High',
    clusterIds: ['cl:unanalyzed'],
    affected: ['a:vendor_lib.rs', 'a:client.py', 'a:App.tsx', 'a:sample.bin'],
    why: 'Vendored, generated, unsupported, and binary files carry zero graph relations. They are invisible to impact analysis: a change there will never show up in a blast-radius query.',
    metrics: ['4 nodes with degree 0'],
    evidence: [{ file: 'vendor/example/lib.rs', line: 1 }, { file: 'generated/client.py', line: 1 }],
    suggested: ['Filter graph to degree = 0 nodes'],
  },
  {
    id: 'tn5', label: 'Low cohesion: configuration cluster', category: 'low-cohesion',
    severity: 'low', confidence: 'Medium',
    clusterIds: ['cl:config'],
    affected: ['a:settings.yaml', 'a:schema.json', 'a:pyproject.toml', 'a:requirements.txt', 'img:worker-schema'],
    why: 'Config artifacts reference three other clusters but barely reference each other (cohesion 0.31, conductance 0.55). Settings changes fan out unpredictably.',
    metrics: ['cohesion 0.31', 'conductance 0.55', '7 boundary edges'],
    evidence: [{ file: 'settings.yaml', line: 1 }],
    suggested: ['Isolate boundary edges of Configuration'],
  },
];
export const tensionById = Object.fromEntries(tensions.map((t) => [t.id, t]));

// Max severity per node (for the tension overlay)
export const nodeTensionSeverity = {};
const sevRank = { low: 1, medium: 2, high: 3 };
for (const t of tensions) {
  for (const id of t.affected) {
    const cur = nodeTensionSeverity[id];
    if (!cur || sevRank[t.severity] > sevRank[cur]) nodeTensionSeverity[id] = t.severity;
  }
}

// ---------------------------------------------------------------------------
// Tags (LIT-24.41): computed from cluster + node facts, plus manual extras
// ---------------------------------------------------------------------------
function langOf(nd) {
  const p = nd.path || '';
  if (nd.kind === 'Module') return nd.sub === 'Python' ? 'python' : nd.sub === 'Rust' ? 'rust' : null;
  if (p.endsWith('.py')) return 'python';
  if (p.endsWith('.rs')) return 'rust';
  if (p.endsWith('.tsx')) return 'typescript';
  return null;
}
const MANUAL_TAGS = {
  'a:vendor_lib.rs': ['status:vendored'],
  'a:client.py': ['status:generated'],
  'a:App.tsx': ['status:unsupported'],
  'a:sample.bin': ['status:binary'],
  'img:route-api-version': ['status:dynamic'],
  'img:route-api-sha': ['status:dynamic'],
  'u:target/debug/worker': ['status:dynamic'],
};
export const nodeTags = {};
for (const nd of nodes) {
  const tags = [];
  const cl = clusterById[clusterOfNode[nd.id]];
  if (cl) tags.push('layer:' + cl.short);
  const lang = langOf(nd);
  if (lang) tags.push('lang:' + lang);
  if (nd.kind === 'Container' || nd.id === 'a:Dockerfile' || nd.id === 'a:docker-compose.yml' || (nd.kind === 'Config' && nd.sub === 'Service')) tags.push('runtime:docker');
  if (MANUAL_TAGS[nd.id]) tags.push(...MANUAL_TAGS[nd.id]);
  nodeTags[nd.id] = tags;
}
export function tagFacets() {
  const counts = {};
  for (const tags of Object.values(nodeTags)) for (const t of tags) counts[t] = (counts[t] || 0) + 1;
  const namespaces = {};
  for (const [tag, count] of Object.entries(counts)) {
    const ns = tag.split(':')[0];
    (namespaces[ns] ||= []).push({ tag, value: tag.split(':')[1], count });
  }
  for (const list of Object.values(namespaces)) list.sort((a, b) => b.count - a.count);
  return Object.entries(namespaces).map(([ns, values]) => ({ ns, values }));
}

// ---------------------------------------------------------------------------
// Docs workspace (LIT-24.37): graph-grounded sections with freshness
// ---------------------------------------------------------------------------
export const docsSections = [
  {
    id: 'overview', title: 'Overview', freshness: 'fresh',
    paragraphs: [
      'polyglot-fixture is a deliberately small mixed-language repository: a Python service (python_app) that loads route settings and delegates route baking to a Rust worker binary (fixture-worker), wired for deployment through a multi-stage Dockerfile and docker-compose, with a thin TSX web shell.',
      'The graph holds 68 nodes and 61 relations across 8 clusters. Two clusters carry almost all behavior: Python service and Rust worker.',
    ],
    evidence: [{ file: 'README.md', line: 1 }, { file: 'architecture.md', line: 1 }],
    relatedNodes: ['a:README.md', 'a:architecture.md'],
    relatedClusters: ['cl:python', 'cl:rust'],
    query: 'MATCH (n) RETURN count(n) BY label',
  },
  {
    id: 'architecture', title: 'Architecture', freshness: 'fresh',
    paragraphs: [
      'RouteService (service.py) is the entry class: __init__ resolves the worker path from RIDGELINE_WORKER, load_settings reads config/schema.json, and bake_route delegates to the module-level run_worker function.',
      'run_worker crosses the language boundary with subprocess.run. On the Rust side, worker::main parses --route and calls fixture_worker::bake_route, which builds a RouteBaker from RIDGELINE_CACHE_DIR and calls its RouteBake::bake implementation.',
    ],
    evidence: [{ file: 'service.py', line: 10 }, { file: 'lib.rs', line: 6 }, { file: 'worker.rs', line: 3 }],
    relatedNodes: ['s:RouteService', 's:run_worker', 's:main', 's:RouteBaker'],
    relatedClusters: ['cl:python', 'cl:rust'],
    query: 'PATH (s:RouteService)-[:Calls*]->(m:main)',
  },
  {
    id: 'workflow', title: 'Workflow', freshness: 'fresh',
    paragraphs: [
      'CI runs one job (checks): pytest, cargo test, web lint, then docker build tagged with the commit SHA. Local development mirrors the same commands via the Makefile.',
    ],
    evidence: [{ file: 'ci.yml', line: 7 }],
    relatedNodes: ['c:checks', 'cmd:pytest', 'cmd:cargo-test', 'cmd:docker-build'],
    relatedClusters: ['cl:ci'],
    query: 'MATCH (j:Job)-[:RunsCommand]->(c) RETURN c',
  },
  {
    id: 'boundaries', title: 'Boundaries', freshness: 'stale',
    staleReason: 'Tension tn2 (drift risk) touched 3 evidence hashes since generation.',
    paragraphs: [
      'The Python \u2192 Rust boundary is a subprocess contract, not a typed one: binary path via RIDGELINE_WORKER, arguments via CLI, results via stdout. It is the highest-tension edge in the repository.',
      'The web cluster is fully disconnected from the service graph today \u2014 architecture.md declares a Web \u2192 PythonService dependency that the graph cannot yet confirm.',
    ],
    evidence: [{ file: 'service.py', line: 26 }, { file: 'architecture.md', line: 9 }],
    relatedNodes: ['s:run_worker', 'e:RIDGELINE_WORKER'],
    relatedClusters: ['cl:python', 'cl:rust', 'cl:web'],
    query: 'MATCH boundary(cl:python, cl:rust)',
  },
  {
    id: 'configuration', title: 'Configuration', freshness: 'partial',
    staleReason: 'schema.json evidence hash changed; 1 of 3 subsections regenerated.',
    paragraphs: [
      'Runtime configuration flows from three surfaces: settings.yaml (service name, image, port, env, paths), config/schema.json (RouteSettings schema with route_file required), and environment variables RIDGELINE_WORKER, RIDGELINE_CACHE_DIR, VERSION.',
    ],
    evidence: [{ file: 'settings.yaml', line: 1 }, { file: 'schema.json', line: 1 }],
    relatedNodes: ['a:settings.yaml', 'a:schema.json', 'e:RIDGELINE_WORKER', 'e:RIDGELINE_CACHE_DIR', 'e:VERSION'],
    relatedClusters: ['cl:config'],
    query: 'MATCH (n:EnvVar)<-[:ReadsEnv|References]-(x) RETURN x',
  },
  {
    id: 'operations', title: 'Operations', freshness: 'fresh',
    paragraphs: [
      'Deployment is compose-first: the api service builds from the Dockerfile (rust:1.96 builder, python:3.13-slim runtime), publishes port 8080, and depends on the web service running node:24-alpine.',
    ],
    evidence: [{ file: 'docker-compose.yml', line: 2 }, { file: 'Dockerfile', line: 1 }],
    relatedNodes: ['c:api', 'c:web', 'c:8080', 'img:python', 'img:rust'],
    relatedClusters: ['cl:infra'],
    query: 'MATCH (s:Service)-[:UsesImage|BuildsImage]->(i) RETURN s, i',
  },
  {
    id: 'risks', title: 'Risks & Tensions', freshness: 'fresh',
    paragraphs: [
      '5 open tensions: 1 high (bridge bottleneck at run_worker), 2 medium (RIDGELINE_WORKER drift, image-tag change concentration), 2 low (orphaned sources, low-cohesion configuration).',
    ],
    evidence: [{ file: 'service.py', line: 26 }],
    relatedNodes: ['s:run_worker', 'e:RIDGELINE_WORKER'],
    relatedClusters: ['cl:python', 'cl:config'],
    query: 'MATCH (t:Tension) RETURN t ORDER BY severity',
  },
  {
    id: 'drift', title: 'Drift', freshness: 'stale',
    staleReason: 'Graph snapshot advanced 2 commits since last drift scan.',
    paragraphs: [
      'Known drift: RIDGELINE_WORKER default in code (target/debug/worker) disagrees with all deployment surfaces (/usr/local/bin/worker). architecture.md claims a Web \u2192 PythonService edge the graph cannot verify.',
    ],
    evidence: [{ file: 'service.py', line: 14 }, { file: 'architecture.md', line: 12 }],
    relatedNodes: ['e:RIDGELINE_WORKER', 'a:architecture.md'],
    relatedClusters: ['cl:config'],
    query: 'DRIFT SCAN docs/ vs graph@5bcfa51',
  },
  {
    id: 'questions', title: 'Open Questions', freshness: 'fresh',
    paragraphs: [
      'Should the web shell call the Python service directly (as architecture.md implies), or is it decorative until TypeScript support lands? Is ghcr.io/example/worker:1.0 in schema.json a live default or a leftover?',
    ],
    evidence: [{ file: 'schema.json', line: 9 }],
    relatedNodes: ['a:App.tsx', 'img:worker-schema'],
    relatedClusters: ['cl:web', 'cl:config'],
    query: 'MATCH unresolved OR unsupported RETURN n',
  },
];

// ---------------------------------------------------------------------------
// Saved investigations (LIT-24.27)
// ---------------------------------------------------------------------------
export const savedInvestigations = [
  {
    id: 'inv1', title: 'RIDGELINE_WORKER drift', when: 'yesterday', snapshot: '5bcfa51',
    note: 'All four declaration sites; code default is the odd one out.',
    state: { selectedType: 'tension', selectedId: 'tn2', overlay: 'tension' },
    stale: false,
  },
  {
    id: 'inv2', title: 'Image tag provenance', when: '3 days ago', snapshot: '41c9af7',
    note: 'Who builds/uses each route-api image reference.',
    state: { selectedType: 'tension', selectedId: 'tn3', overlay: 'tension' },
    stale: true,
  },
];

// Pre-written subsystem doc used by the agent-generation simulation.
export const AGENT_DOC = {
  'cl:python': 'The Python service cluster centers on RouteService (service.py:10). Construction resolves the worker binary path from RIDGELINE_WORKER with a debug-build fallback (service.py:14) \u2014 flagged by tension tn2. load_settings reads config/schema.json through the injected config_path. bake_route delegates to run_worker (service.py:24), the cluster\u2019s single bridge node, which crosses into the Rust worker via subprocess.run. Public surface is re-exported through __init__.py (RouteService, run_worker). External dependency: pydantic (declared, not yet imported in analyzed sources).',
  'cl:rust': 'The Rust worker cluster is the fixture-worker crate: RouteBake trait (lib.rs:6), RouteBaker struct with cache_dir from RIDGELINE_CACHE_DIR (lib.rs:17), and the free function bake_route that composes them. The worker binary (worker.rs) parses --route and prints bake_route output to stdout \u2014 the implicit contract consumed by the Python service. External dependency: anyhow.',
  'cl:infra': 'Container infra: a two-stage Dockerfile (rust:1.96 builder \u2192 python:3.13-slim runtime) copies the compiled worker to /usr/local/bin/worker and sets RIDGELINE_WORKER accordingly. Compose wires api (builds the image, port 8080, depends on web) and web (node:24-alpine, npm run dev).',
};
export const AGENT_DOC_DEFAULT = 'This cluster has a small, self-contained surface. Members, boundary edges, and evidence references are listed in the cluster summary; no additional architecture narrative is required at this snapshot.';
