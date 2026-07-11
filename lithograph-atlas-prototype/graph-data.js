// Sample Lithograph semantic-graph data, grounded in the fixtures/polyglot
// fixture repository (README.md, service.py, lib.rs, worker.rs,
// docker-compose.yml, config/settings.yaml, config/schema.json, ci.yml,
// docs/architecture.md). Shapes mirror src/graph/model.rs + GraphSchema.

// ---- Node kind metadata (top-level GraphNode::node_type tags) ----
export const NODE_KINDS = [
  { key: 'Artifact', label: 'Artifact', color: 'oklch(0.72 0.14 250)' },
  { key: 'Symbol', label: 'Symbol', color: 'oklch(0.78 0.16 55)' },
  { key: 'Module', label: 'Module', color: 'oklch(0.74 0.15 300)' },
  { key: 'Package', label: 'Package', color: 'oklch(0.72 0.16 20)' },
  { key: 'Config', label: 'Config', color: 'oklch(0.75 0.14 165)' },
  { key: 'Container', label: 'Container', color: 'oklch(0.78 0.13 200)' },
  { key: 'Command', label: 'Command', color: 'oklch(0.76 0.15 330)' },
  { key: 'EnvVar', label: 'Env Var', color: 'oklch(0.82 0.15 95)' },
  { key: 'Documentation', label: 'Documentation', color: 'oklch(0.8 0.1 120)' },
  { key: 'Unresolved', label: 'Unresolved', color: 'oklch(0.55 0.01 260)' },
];

// ---- Relation kind metadata (RelationKind), grouped for the filter panel ----
export const RELATION_GROUPS = [
  { key: 'structural', label: 'Structural', kinds: ['Contains', 'BelongsToModule', 'BelongsToPackage'] },
  { key: 'dependency', label: 'Dependencies', kinds: ['DependsOnPackage', 'Imports'] },
  { key: 'behavior', label: 'Code behavior', kinds: ['Calls', 'Implements', 'Inherits', 'TypeRefs', 'Usages', 'Ffi', 'DataFlows', 'SimilarTo'] },
  { key: 'infra', label: 'Infra', kinds: ['ReadsEnv', 'RunsCommand', 'UsesImage', 'BuildsImage', 'PublishesImage'] },
  { key: 'messaging', label: 'Messaging', kinds: ['Emits', 'ListensOn'] },
  { key: 'generic', label: 'Generic', kinds: ['References'] },
];

const REL_COLOR = {
  Contains: 'oklch(0.62 0.05 250)',
  BelongsToModule: 'oklch(0.62 0.05 240)',
  BelongsToPackage: 'oklch(0.62 0.05 260)',
  DependsOnPackage: 'oklch(0.68 0.14 225)',
  Imports: 'oklch(0.68 0.14 235)',
  Calls: 'oklch(0.72 0.15 45)',
  Implements: 'oklch(0.72 0.15 35)',
  Inherits: 'oklch(0.72 0.15 55)',
  TypeRefs: 'oklch(0.72 0.15 65)',
  Usages: 'oklch(0.72 0.15 25)',
  Ffi: 'oklch(0.72 0.15 75)',
  DataFlows: 'oklch(0.72 0.15 15)',
  SimilarTo: 'oklch(0.72 0.15 85)',
  ReadsEnv: 'oklch(0.7 0.13 165)',
  RunsCommand: 'oklch(0.7 0.13 150)',
  UsesImage: 'oklch(0.7 0.13 180)',
  BuildsImage: 'oklch(0.7 0.13 195)',
  PublishesImage: 'oklch(0.7 0.13 135)',
  Emits: 'oklch(0.7 0.15 340)',
  ListensOn: 'oklch(0.7 0.15 320)',
  References: 'oklch(0.55 0.02 260)',
};

export const RELATION_KINDS = RELATION_GROUPS.flatMap((g) =>
  g.kinds.map((k) => ({ key: k, label: humanize(k), color: REL_COLOR[k], group: g.key }))
);

function humanize(key) {
  return key.replace(/([a-z])([A-Z])/g, '$1 $2');
}

export function nodeKindMeta(kind) {
  return NODE_KINDS.find((n) => n.key === kind);
}
export function relKindMeta(kind) {
  return RELATION_KINDS.find((r) => r.key === kind);
}

// ---- Nodes ----
// n() keeps literals compact: id, kind, label, sub(-kind/category), path, detail, evidence[]
function n(id, kind, label, sub, path, detail, evidence) {
  return { id, kind, label, sub: sub || null, path: path || null, detail: detail || null, evidence: evidence || [] };
}

export const nodes = [
  // Artifacts (category drawn straight from the fixture's README table)
  n('a:Dockerfile', 'Artifact', 'Dockerfile', 'ContainerDefinition', 'Dockerfile', 'Multi-stage container build: Rust builder stage, Python runtime stage.'),
  n('a:LICENSE', 'Artifact', 'LICENSE', 'Documentation', 'LICENSE', 'Extensionless legal text.'),
  n('a:Makefile', 'Artifact', 'Makefile', 'BuildDefinition', 'Makefile', 'Extensionless build command file.'),
  n('a:README.md', 'Artifact', 'README.md', 'Documentation', 'README.md', 'Repository-level Markdown with links, commands, and code fences.'),
  n('a:docker-compose.yml', 'Artifact', 'docker-compose.yml', 'ContainerDefinition', 'docker-compose.yml', 'Service wiring, ports, env, and build context.'),
  n('a:pyproject.toml', 'Artifact', 'pyproject.toml', 'PackageManifest', 'pyproject.toml', 'Python package metadata.'),
  n('a:requirements.txt', 'Artifact', 'requirements.txt', 'PackageManifest', 'requirements.txt', 'Python dependency list.'),
  n('a:ci.yml', 'Artifact', 'ci.yml', 'ContinuousIntegration', '.github/workflows/ci.yml', 'CI commands across Python, Rust, and web assets.'),
  n('a:logo.svg', 'Artifact', 'logo.svg', 'StaticAsset', 'assets/logo.svg', 'Text static asset with image semantics.'),
  n('a:schema.json', 'Artifact', 'schema.json', 'Configuration', 'config/schema.json', 'JSON schema-like structured config.'),
  n('a:settings.yaml', 'Artifact', 'settings.yaml', 'Configuration', 'config/settings.yaml', 'YAML config with service, image, env, and path references.'),
  n('a:architecture.md', 'Artifact', 'architecture.md', 'Documentation', 'docs/architecture.md', 'Existing architecture evidence and a Mermaid diagram.'),
  n('a:client.py', 'Artifact', 'client.py', 'GeneratedSource', 'generated/client.py', 'Generated-file header detected.'),
  n('a:Cargo.toml', 'Artifact', 'Cargo.toml', 'PackageManifest', 'rust/Cargo.toml', 'Cargo manifest, binary target, and dependencies.'),
  n('a:lib.rs', 'Artifact', 'lib.rs', 'SourceCode', 'rust/src/lib.rs', 'Rust public API: trait, struct, function, and env read.'),
  n('a:worker.rs', 'Artifact', 'worker.rs', 'SourceCode', 'rust/src/bin/worker.rs', 'Rust binary target.'),
  n('a:__init__.py', 'Artifact', '__init__.py', 'SourceCode', 'src/python_app/__init__.py', 'Python package marker and export surface.'),
  n('a:service.py', 'Artifact', 'service.py', 'SourceCode', 'src/python_app/service.py', 'Python imports, classes, functions, env reads, and a subprocess call.'),
  n('a:vendor_lib.rs', 'Artifact', 'lib.rs', 'SourceCode', 'vendor/example/lib.rs', 'Vendored source path detected \u2014 excluded from deep analysis.'),
  n('a:index.html', 'Artifact', 'index.html', 'Template', 'web/index.html', 'HTML entrypoint/template.'),
  n('a:App.tsx', 'Artifact', 'App.tsx', 'SourceCode', 'web/src/App.tsx', 'Unsupported TSX source \u2014 generic-text fallback, no deep symbols.'),
  n('a:sample.bin', 'Artifact', 'sample.bin', 'BinaryAsset', 'data/sample.bin', 'Opaque NUL-byte binary data.'),

  // Modules
  n('m:python_app', 'Module', 'python_app', 'Python', 'python_app', 'Python package module.', [{ file: 'src/python_app/__init__.py', line: 1 }]),
  n('m:fixture_worker', 'Module', 'fixture_worker', 'Rust', 'fixture_worker', 'Rust library crate root module.', [{ file: 'rust/src/lib.rs', line: 1 }]),
  n('m:worker_bin', 'Module', 'worker', 'Rust', 'worker', 'Rust binary crate root module.', [{ file: 'rust/src/bin/worker.rs', line: 1 }]),

  // Packages
  n('pkg:polyglot-fixture', 'Package', 'polyglot-fixture', 'local', null, 'Local Python package (pyproject.toml).'),
  n('pkg:pydantic', 'Package', 'pydantic', 'external', null, 'External PyPI dependency.'),
  n('pkg:fixture-worker', 'Package', 'fixture-worker', 'local', null, 'Local Rust crate (Cargo.toml), hosts one lib + one bin target.'),
  n('pkg:anyhow', 'Package', 'anyhow', 'external', null, 'External crates.io dependency.'),

  // Symbols \u2014 Python (service.py)
  n('s:RouteService', 'Symbol', 'RouteService', 'Class', 'src/python_app/service.py', 'Loads route metadata and delegates expensive work to the Rust worker.', [{ file: 'service.py', line: 10 }]),
  n('s:RouteService.__init__', 'Symbol', '__init__', 'Method', 'src/python_app/service.py', 'Reads RIDGELINE_WORKER, defaults to target/debug/worker.', [{ file: 'service.py', line: 13 }]),
  n('s:RouteService.load_settings', 'Symbol', 'load_settings', 'Method', 'src/python_app/service.py', 'Reads config_path / schema.json.', [{ file: 'service.py', line: 17 }]),
  n('s:RouteService.bake_route', 'Symbol', 'bake_route', 'Method', 'src/python_app/service.py', 'Delegates to run_worker(self.worker_path, route_file).', [{ file: 'service.py', line: 21 }]),
  n('s:run_worker', 'Symbol', 'run_worker', 'Function', 'src/python_app/service.py', 'Runs the Rust worker binary via subprocess.run and returns stdout.', [{ file: 'service.py', line: 24 }]),

  // Symbols \u2014 Rust (lib.rs, worker.rs)
  n('s:RouteBake', 'Symbol', 'RouteBake', 'Trait', 'rust/src/lib.rs', 'Provides route baking behavior.', [{ file: 'lib.rs', line: 6 }]),
  n('s:RouteBaker', 'Symbol', 'RouteBaker', 'Struct', 'rust/src/lib.rs', 'Configurable route baker (holds cache_dir).', [{ file: 'lib.rs', line: 12 }]),
  n('s:RouteBaker.from_env', 'Symbol', 'from_env', 'Method', 'rust/src/lib.rs', 'Builds a baker from RIDGELINE_CACHE_DIR (default target/cache).', [{ file: 'lib.rs', line: 17 }]),
  n('s:RouteBaker.bake', 'Symbol', 'bake', 'Method', 'rust/src/lib.rs', 'impl RouteBake for RouteBaker \u2014 formats "baked:{route}:{cache_dir}".', [{ file: 'lib.rs', line: 24 }]),
  n('s:bake_route_fn', 'Symbol', 'bake_route', 'Function', 'rust/src/lib.rs', 'Bakes a route with the default baker.', [{ file: 'lib.rs', line: 30 }]),
  n('s:main', 'Symbol', 'main', 'Function', 'rust/src/bin/worker.rs', 'Reads --route arg, prints bake_route(route).', [{ file: 'worker.rs', line: 3 }]),

  // Unresolved
  n('u:pathlib.Path', 'Unresolved', 'pathlib.Path', 'type ref', null, 'Stdlib type referenced as a parameter annotation; not modeled as a graph symbol.'),
  n('u:target/debug/worker', 'Unresolved', 'target/debug/worker', 'runtime literal', null, 'Fallback literal for RIDGELINE_WORKER when unset \u2014 cannot resolve statically.'),

  // Documentation
  n('d:readme.title', 'Documentation', 'Polyglot Fixture', 'h1', 'README.md', null, [{ file: 'README.md', line: 1 }]),
  n('d:readme.categories', 'Documentation', 'Expected Artifact Categories', 'h2', 'README.md', null, [{ file: 'README.md', line: 8 }]),
  n('d:readme.offline', 'Documentation', 'Offline Guarantees', 'h2', 'README.md', null, [{ file: 'README.md', line: 31 }]),
  n('d:arch.title', 'Documentation', 'Architecture Notes', 'h1', 'docs/architecture.md', 'Includes a Mermaid diagram of service \u2192 worker \u2192 web relationships.', [{ file: 'architecture.md', line: 1 }]),

  // Config
  n('c:api', 'Config', 'api', 'Service', 'docker-compose.yml', 'Builds from Dockerfile, publishes 8080, depends on web.', [{ file: 'docker-compose.yml', line: 2 }]),
  n('c:web', 'Config', 'web', 'Service', 'docker-compose.yml', 'node:24-alpine, runs "npm run dev".', [{ file: 'docker-compose.yml', line: 14 }]),
  n('c:8080', 'Config', '8080', 'Port', 'docker-compose.yml', 'Published container port.', [{ file: 'docker-compose.yml', line: 8 }]),
  n('c:checks', 'Config', 'checks', 'Job', '.github/workflows/ci.yml', 'runs-on ubuntu-latest \u2014 python, rust, web, and image build steps.', [{ file: 'ci.yml', line: 7 }]),

  // Container images
  n('img:rust', 'Container', 'rust:1.96', 'static', null, 'Builder stage base image.'),
  n('img:python', 'Container', 'python:3.13-slim', 'static', null, 'Runtime stage base image.'),
  n('img:node', 'Container', 'node:24-alpine', 'static', null, 'web service base image.'),
  n('img:route-api-version', 'Container', 'ghcr.io/example/route-api:${VERSION}', 'dynamic', null, 'compose api image \u2014 unresolved template expression.'),
  n('img:route-api-sha', 'Container', 'ghcr.io/example/route-api:${{ github.sha }}', 'dynamic', null, 'CI-built image tag \u2014 unresolved template expression.'),
  n('img:worker-schema', 'Container', 'ghcr.io/example/worker:1.0', 'static', null, 'Default value of worker_image in schema.json \u2014 low confidence.'),

  // Commands
  n('cmd:cargo-build', 'Command', 'cargo build --manifest-path rust/Cargo.toml --release', null, 'Dockerfile', null, [{ file: 'Dockerfile', line: 4 }]),
  n('cmd:pip-install', 'Command', 'pip install --no-cache-dir -r requirements.txt', null, 'Dockerfile', null, [{ file: 'Dockerfile', line: 8 }]),
  n('cmd:python-cmd', 'Command', 'python -m python_app.service', null, 'Dockerfile', null, [{ file: 'Dockerfile', line: 12 }]),
  n('cmd:npm-dev', 'Command', 'npm run dev', null, 'docker-compose.yml', null, [{ file: 'docker-compose.yml', line: 15 }]),
  n('cmd:pytest', 'Command', 'python -m pytest', null, '.github/workflows/ci.yml', null, [{ file: 'ci.yml', line: 9 }]),
  n('cmd:cargo-test', 'Command', 'cargo test --manifest-path rust/Cargo.toml', null, '.github/workflows/ci.yml', null, [{ file: 'ci.yml', line: 11 }]),
  n('cmd:npm-lint', 'Command', 'npm --prefix web run lint', null, '.github/workflows/ci.yml', null, [{ file: 'ci.yml', line: 13 }]),
  n('cmd:docker-build', 'Command', 'docker build -t ghcr.io/example/route-api:${{ github.sha }} .', null, '.github/workflows/ci.yml', null, [{ file: 'ci.yml', line: 15 }]),
  n('cmd:worker-invoke', 'Command', '<worker_path> --route <route_file>', null, 'src/python_app/service.py', 'Dynamic \u2014 worker_path is resolved at runtime.', [{ file: 'service.py', line: 26 }]),

  // Env vars
  n('e:RIDGELINE_WORKER', 'EnvVar', 'RIDGELINE_WORKER', null, null, 'Path to the compiled worker binary.'),
  n('e:RIDGELINE_CACHE_DIR', 'EnvVar', 'RIDGELINE_CACHE_DIR', null, null, 'Cache directory for baked routes.'),
  n('e:VERSION', 'EnvVar', 'VERSION', null, null, 'Image tag suffix, defaults to "dev".'),
];

// ---- Relations ----
function r(id, source, target, kind, confidence, evidence) {
  return { id, source, target, kind, confidence: confidence || 'High', evidence: evidence || [] };
}

export const relations = [
  // Contains
  r('r1', 'a:service.py', 's:RouteService', 'Contains'),
  r('r2', 'a:service.py', 's:run_worker', 'Contains'),
  r('r3', 's:RouteService', 's:RouteService.__init__', 'Contains'),
  r('r4', 's:RouteService', 's:RouteService.load_settings', 'Contains'),
  r('r5', 's:RouteService', 's:RouteService.bake_route', 'Contains'),
  r('r6', 'a:lib.rs', 's:RouteBake', 'Contains'),
  r('r7', 'a:lib.rs', 's:RouteBaker', 'Contains'),
  r('r8', 'a:lib.rs', 's:bake_route_fn', 'Contains'),
  r('r9', 's:RouteBaker', 's:RouteBaker.from_env', 'Contains'),
  r('r10', 's:RouteBaker', 's:RouteBaker.bake', 'Contains'),
  r('r11', 'a:worker.rs', 's:main', 'Contains'),
  r('r12', 'a:README.md', 'd:readme.title', 'Contains'),
  r('r13', 'a:README.md', 'd:readme.categories', 'Contains'),
  r('r14', 'a:README.md', 'd:readme.offline', 'Contains'),
  r('r15', 'a:architecture.md', 'd:arch.title', 'Contains'),
  r('r16', 'a:ci.yml', 'c:checks', 'Contains'),

  // BelongsToModule
  r('r17', 'a:service.py', 'm:python_app', 'BelongsToModule'),
  r('r18', 'a:__init__.py', 'm:python_app', 'BelongsToModule'),
  r('r19', 'a:lib.rs', 'm:fixture_worker', 'BelongsToModule'),
  r('r20', 'a:worker.rs', 'm:worker_bin', 'BelongsToModule'),

  // BelongsToPackage
  r('r21', 'm:python_app', 'pkg:polyglot-fixture', 'BelongsToPackage'),
  r('r22', 'm:fixture_worker', 'pkg:fixture-worker', 'BelongsToPackage'),
  r('r23', 'm:worker_bin', 'pkg:fixture-worker', 'BelongsToPackage'),

  // DependsOnPackage
  r('r24', 'pkg:polyglot-fixture', 'pkg:pydantic', 'DependsOnPackage'),
  r('r25', 'pkg:fixture-worker', 'pkg:anyhow', 'DependsOnPackage'),

  // Imports
  r('r26', 'a:__init__.py', 's:RouteService', 'Imports', 'High', [{ file: '__init__.py', line: 3 }]),
  r('r27', 'a:__init__.py', 's:run_worker', 'Imports', 'High', [{ file: '__init__.py', line: 3 }]),
  r('r28', 'a:worker.rs', 's:bake_route_fn', 'Imports', 'High', [{ file: 'worker.rs', line: 1 }]),

  // Calls
  r('r29', 's:RouteService.bake_route', 's:run_worker', 'Calls', 'High', [{ file: 'service.py', line: 22 }]),
  r('r30', 's:main', 's:bake_route_fn', 'Calls', 'High', [{ file: 'worker.rs', line: 7 }]),
  r('r31', 's:bake_route_fn', 's:RouteBaker.from_env', 'Calls', 'High', [{ file: 'lib.rs', line: 31 }]),
  r('r32', 's:bake_route_fn', 's:RouteBaker.bake', 'Calls', 'High', [{ file: 'lib.rs', line: 31 }]),

  // ReadsEnv
  r('r33', 's:RouteService.__init__', 'e:RIDGELINE_WORKER', 'ReadsEnv', 'High', [{ file: 'service.py', line: 14 }]),
  r('r34', 's:RouteBaker.from_env', 'e:RIDGELINE_CACHE_DIR', 'ReadsEnv', 'High', [{ file: 'lib.rs', line: 18 }]),

  // RunsCommand
  r('r35', 's:run_worker', 'cmd:worker-invoke', 'RunsCommand', 'Medium', [{ file: 'service.py', line: 27 }]),
  r('r36', 'a:Dockerfile', 'cmd:cargo-build', 'RunsCommand'),
  r('r37', 'a:Dockerfile', 'cmd:pip-install', 'RunsCommand'),
  r('r38', 'a:Dockerfile', 'cmd:python-cmd', 'RunsCommand'),
  r('r39', 'a:docker-compose.yml', 'cmd:npm-dev', 'RunsCommand'),
  r('r40', 'c:checks', 'cmd:pytest', 'RunsCommand'),
  r('r41', 'c:checks', 'cmd:cargo-test', 'RunsCommand'),
  r('r42', 'c:checks', 'cmd:npm-lint', 'RunsCommand'),
  r('r43', 'c:checks', 'cmd:docker-build', 'RunsCommand'),

  // UsesImage
  r('r44', 'a:Dockerfile', 'img:rust', 'UsesImage'),
  r('r45', 'a:Dockerfile', 'img:python', 'UsesImage'),
  r('r46', 'c:web', 'img:node', 'UsesImage'),

  // BuildsImage
  r('r47', 'c:api', 'img:route-api-version', 'BuildsImage'),
  r('r48', 'c:checks', 'img:route-api-sha', 'BuildsImage'),

  // Implements
  r('r49', 's:RouteBaker', 's:RouteBake', 'Implements', 'High', [{ file: 'lib.rs', line: 23 }]),

  // TypeRefs
  r('r50', 's:RouteService.__init__', 'u:pathlib.Path', 'TypeRefs', 'Medium', [{ file: 'service.py', line: 13 }]),

  // Usages
  r('r51', 'a:__init__.py', 's:RouteService', 'Usages', 'Medium', [{ file: '__init__.py', line: 5 }]),
  r('r52', 'a:__init__.py', 's:run_worker', 'Usages', 'Medium', [{ file: '__init__.py', line: 5 }]),

  // References
  r('r53', 'c:api', 'e:RIDGELINE_WORKER', 'References', 'Medium', [{ file: 'docker-compose.yml', line: 9 }]),
  r('r54', 'a:settings.yaml', 'a:schema.json', 'References', 'Medium', [{ file: 'settings.yaml', line: 10 }]),
  r('r55', 'a:settings.yaml', 'e:VERSION', 'References', 'Medium', [{ file: 'settings.yaml', line: 3 }]),
  r('r56', 'a:settings.yaml', 'e:RIDGELINE_WORKER', 'References', 'Medium', [{ file: 'settings.yaml', line: 6 }]),
  r('r57', 'a:settings.yaml', 'e:RIDGELINE_CACHE_DIR', 'References', 'Medium', [{ file: 'settings.yaml', line: 7 }]),
  r('r58', 'a:docker-compose.yml', 'e:VERSION', 'References', 'Medium', [{ file: 'docker-compose.yml', line: 6 }]),
  r('r59', 'a:schema.json', 'img:worker-schema', 'References', 'Low', [{ file: 'schema.json', line: 9 }]),

  // DataFlows
  r('r60', 's:RouteService.__init__', 's:RouteService.bake_route', 'DataFlows', 'Medium', [{ file: 'service.py', line: 14 }]),

  // SimilarTo
  r('r61', 's:RouteService.bake_route', 's:RouteBaker.bake', 'SimilarTo', 'Low', [{ file: 'lexical: "bake_route" ~ "bake"', line: 0 }]),
];

export const nodeById = Object.fromEntries(nodes.map((n2) => [n2.id, n2]));
