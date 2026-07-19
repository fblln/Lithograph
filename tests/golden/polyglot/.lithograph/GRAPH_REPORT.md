# Lithograph Graph Report

## Summary and resolution

- Nodes: 133
- Relations: 160
- Resolved relations: 141 of 160 (88.1%)
- Unresolved relations: 19
- Low-confidence relations: 25

## God nodes by degree

- `artifact:README.md` — README.md (in 1, out 22, total 23)
- `artifact:config/settings.yaml` — config/settings.yaml (in 1, out 22, total 23)
- `artifact:config/schema.json` — config/schema.json (in 2, out 14, total 16)
- `artifact:src/python_app/service.py` — src/python_app/service.py (in 1, out 14, total 15)
- `artifact:web/src/App.tsx` — web/src/App.tsx (in 1, out 12, total 13)
- `symbol:src/python_app/service.py#src.python_app.service::RouteService` — src.python_app.service::RouteService (in 4, out 6, total 10)
- `artifact:web/index.html` — web/index.html (in 1, out 8, total 9)
- `artifact:Dockerfile` — Dockerfile (in 1, out 7, total 8)
- `artifact:Makefile` — Makefile (in 1, out 7, total 8)
- `artifact:rust/src/lib.rs` — rust/src/lib.rs (in 1, out 7, total 8)

## Cross-cluster bridges by betweenness

No positive-betweenness cross-cluster bridges detected.

## Import and dependency cycles

No module dependency cycles detected.

## Knowledge gaps

### Isolated nodes

- `artifact:LICENSE` — LICENSE (in 0, out 0, total 0)
- `module:vendor::example` — vendor::example (in 0, out 0, total 0)

### Unresolved hotspots

- `unresolved:str` — str (4 inbound relations)
- `unresolved:ghcr.io/example/route-api:dev` — ghcr.io/example/route-api:dev (2 inbound relations)
- `unresolved:/usr/local/bin/worker` — /usr/local/bin/worker (1 inbound relations)
- `unresolved:/var/cache/ridgeline` — /var/cache/ridgeline (1 inbound relations)
- `unresolved:None` — None (1 inbound relations)
- `unresolved:React` — React (1 inbound relations)
- `unresolved:RouteBaker::from_env` — RouteBaker::from_env (1 inbound relations)
- `unresolved:assets/` — assets/ (1 inbound relations)
- `unresolved:dict[str, object]` — dict[str, object] (1 inbound relations)
- `unresolved:fixture_worker::bake_route` — fixture_worker::bake_route (1 inbound relations)
- `unresolved:h1` — h1 (1 inbound relations)
- `unresolved:https://json-schema.org/draft/2020-12/schema` — https://json-schema.org/draft/2020-12/schema (1 inbound relations)
- `unresolved:import React from "react";` — import React from "react"; (1 inbound relations)
- `unresolved:main` — main (1 inbound relations)
- `unresolved:p` — p (1 inbound relations)

### Low-confidence relations to audit

- `artifact:Makefile` → `command:Makefile#12` (RunsCommand, relation `relation:10`)
- `artifact:Makefile` → `command:Makefile#15` (RunsCommand, relation `relation:11`)
- `artifact:Makefile` → `command:Makefile#6` (RunsCommand, relation `relation:7`)
- `artifact:Makefile` → `command:Makefile#9` (RunsCommand, relation `relation:8`)
- `artifact:Makefile` → `artifact:rust/Cargo.toml` (References, relation `relation:9`)
- `artifact:Makefile` → `unresolved:ghcr.io/example/route-api:dev` (References, relation `relation:12`)
- `artifact:Makefile` → `unresolved:ghcr.io/example/route-api:dev` (References, relation `relation:13`)
- `artifact:config/schema.json` → `unresolved:https://json-schema.org/draft/2020-12/schema` (References, relation `relation:36`)
- `artifact:config/settings.yaml` → `unresolved:/usr/local/bin/worker` (References, relation `relation:44`)
- `artifact:config/settings.yaml` → `unresolved:/var/cache/ridgeline` (References, relation `relation:43`)
- `artifact:config/settings.yaml` → `unresolved:assets/` (References, relation `relation:39`)
- `artifact:rust/src/bin/worker.rs` → `symbol:std::env::args` (Calls, relation `relation:73`)
- `artifact:rust/src/lib.rs` → `unresolved:RouteBaker::from_env` (Calls, relation `relation:81`)
- `artifact:src/python_app/service.py` → `unresolved:str` (Calls, relation `relation:111`)
- `artifact:src/python_app/service.py` → `command:src/python_app/service.py#31` (RunsCommand, relation `relation:110`)
- `artifact:web/src/App.tsx` → `unresolved:import React from "react";` (Imports, relation `relation:127`)
- `artifact:web/src/App.tsx` → `unresolved:React` (Usages, relation `relation:128`)
- `artifact:web/src/App.tsx` → `unresolved:h1` (Usages, relation `relation:133`)
- `artifact:web/src/App.tsx` → `unresolved:main` (Usages, relation `relation:132`)
- `artifact:web/src/App.tsx` → `unresolved:p` (Usages, relation `relation:134`)

## Suggested audit questions

- What responsibilities make `artifact:README.md` highly connected, and should any be separated?
- What responsibilities make `artifact:config/settings.yaml` highly connected, and should any be separated?
- What responsibilities make `artifact:config/schema.json` highly connected, and should any be separated?
- What repository evidence can resolve `unresolved:str` (str)?
- What repository evidence can resolve `unresolved:ghcr.io/example/route-api:dev` (ghcr.io/example/route-api:dev)?
- What repository evidence can resolve `unresolved:/usr/local/bin/worker` (/usr/local/bin/worker)?
- Which of the graph's 25 low-confidence relations are justified by source evidence, and which should be corrected?

