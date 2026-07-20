//! LIT-45.3: traces a symbol imported from a barrel back to the files that
//! could declare it.
//!
//! TypeScript packages funnel their public surface through `index.ts` barrels
//! (`export { X } from './b'`, `export * from './a'`). A consumer importing
//! from the barrel names a file that declares nothing, so symbol resolution
//! stops there and every call or type edge into the real module is lost.
//!
//! This module answers one question: given a symbol imported from some file,
//! which `(file, name)` pairs could actually declare it? Re-export chains
//! rename (`export { A as B }`) and fan out (`export *`), so the name travels
//! with the file rather than being assumed constant. Callers check each pair
//! against their own declaration index; nothing here decides what exists.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// How a barrel republishes names from another module.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReExportKind {
    /// `export * from './a'` -- every name, unrenamed.
    Star,
    /// `export { A as B } from './b'` -- `A` in the target, `B` here.
    Named {
        /// Name as the target module declares it.
        exported: String,
        /// Name as this barrel publishes it.
        local: String,
    },
}

/// One `export ... from` statement, with its specifier already resolved to the
/// artifact paths it could name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReExport {
    /// Candidate artifact paths for the specifier, most likely first. More
    /// than one is normal: `./util` may be `util.ts` or `util/index.ts`.
    pub targets: Vec<String>,
    /// What is being republished.
    pub kind: ReExportKind,
}

/// Re-export statements by the artifact path that contains them.
pub(crate) type ReExportMap = BTreeMap<String, Vec<ReExport>>;

/// How many barrel hops to follow before giving up.
///
/// Real barrel chains are two or three deep (`client` -> `services` ->
/// `ItemsService`). The bound is a backstop for pathological graphs; the
/// visited set below is what actually terminates cycles (AC4).
const MAX_BARREL_DEPTH: usize = 8;

/// Every `(artifact, name)` a symbol imported from `origin` as `symbol` could
/// resolve to, including `(origin, symbol)` itself.
///
/// Returns pairs rather than files because a re-export chain can rename: a
/// consumer importing `B` from a barrel that says `export { A as B } from
/// './b'` is really asking for `A` in `./b`. Following the chain while
/// assuming the name is constant would look up the wrong declaration.
///
/// Deterministic: results are collected through sorted structures and the
/// traversal is breadth-first from `origin`, so nearer declarations come
/// first and the order never depends on map iteration.
pub(crate) fn barrel_targets(
    origin: &str,
    symbol: &str,
    re_exports: &ReExportMap,
) -> Vec<(String, String)> {
    let mut queue = VecDeque::from([(origin.to_owned(), symbol.to_owned(), 0usize)]);
    let mut visited = BTreeSet::new();
    let mut results = Vec::new();

    while let Some((artifact, name, depth)) = queue.pop_front() {
        // A cyclic barrel (`a` re-exports from `b`, `b` from `a`) revisits the
        // same pair forever without this; the depth bound alone would only
        // make it terminate slowly.
        if !visited.insert((artifact.clone(), name.clone())) {
            continue;
        }
        results.push((artifact.clone(), name.clone()));
        if depth >= MAX_BARREL_DEPTH {
            continue;
        }

        for re_export in re_exports.get(&artifact).into_iter().flatten() {
            let next = match &re_export.kind {
                // `export * from './a'` republishes this name unchanged, so
                // the symbol may be declared anywhere down the star chain.
                ReExportKind::Star => Some(name.clone()),
                // A named re-export only matters when it publishes the name
                // being looked for, and it hands the *target's* name onward.
                ReExportKind::Named { exported, local } => {
                    (*local == name).then(|| exported.clone())
                }
            };
            let Some(next_name) = next else {
                continue;
            };
            for target in &re_export.targets {
                queue.push_back((target.clone(), next_name.clone(), depth + 1));
            }
        }
    }

    results
}

#[cfg(test)]
mod tests {
    use super::{ReExport, ReExportKind, ReExportMap, barrel_targets};

    fn named(target: &str, exported: &str, local: &str) -> ReExport {
        ReExport {
            targets: vec![target.to_owned()],
            kind: ReExportKind::Named {
                exported: exported.to_owned(),
                local: local.to_owned(),
            },
        }
    }

    fn star(target: &str) -> ReExport {
        ReExport {
            targets: vec![target.to_owned()],
            kind: ReExportKind::Star,
        }
    }

    /// AC1: the shape the pinned Full Stack FastAPI corpus actually uses --
    /// `import { ItemsService } from '../../client'` where `client/index.ts`
    /// says `export { ItemsService } from './services/ItemsService'`.
    #[test]
    fn a_named_re_export_resolves_to_the_declaring_module() {
        let re_exports = ReExportMap::from([(
            "frontend/src/client/index.ts".to_owned(),
            vec![named(
                "frontend/src/client/services/ItemsService.ts",
                "ItemsService",
                "ItemsService",
            )],
        )]);

        let targets = barrel_targets("frontend/src/client/index.ts", "ItemsService", &re_exports);

        assert_eq!(
            targets,
            vec![
                // The barrel itself stays a candidate: a file can both
                // re-export and declare.
                (
                    "frontend/src/client/index.ts".to_owned(),
                    "ItemsService".to_owned()
                ),
                (
                    "frontend/src/client/services/ItemsService.ts".to_owned(),
                    "ItemsService".to_owned()
                ),
            ],
        );
    }

    /// AC2: `export *` chains resolve through at least two hops.
    #[test]
    fn star_export_chains_resolve_through_two_hops() {
        let re_exports = ReExportMap::from([
            ("src/index.ts".to_owned(), vec![star("src/models/index.ts")]),
            (
                "src/models/index.ts".to_owned(),
                vec![star("src/models/item.ts")],
            ),
        ]);

        let targets = barrel_targets("src/index.ts", "Item", &re_exports);

        assert_eq!(
            targets,
            vec![
                ("src/index.ts".to_owned(), "Item".to_owned()),
                ("src/models/index.ts".to_owned(), "Item".to_owned()),
                // Two hops down, still looking for the same name.
                ("src/models/item.ts".to_owned(), "Item".to_owned()),
            ],
        );
    }

    /// AC3: an aliased re-export resolves under the alias, and the name that
    /// travels onward is the *target's* name, not the alias.
    #[test]
    fn aliased_re_exports_resolve_under_the_alias_and_rename_the_lookup() {
        let re_exports = ReExportMap::from([(
            "src/index.ts".to_owned(),
            vec![named("src/impl.ts", "InternalName", "PublicName")],
        )]);

        assert_eq!(
            barrel_targets("src/index.ts", "PublicName", &re_exports),
            vec![
                ("src/index.ts".to_owned(), "PublicName".to_owned()),
                // Renamed on the way through.
                ("src/impl.ts".to_owned(), "InternalName".to_owned()),
            ],
        );
        // The internal name is not importable from the barrel.
        assert_eq!(
            barrel_targets("src/index.ts", "InternalName", &re_exports),
            vec![("src/index.ts".to_owned(), "InternalName".to_owned())],
            "the barrel publishes only `PublicName`; `InternalName` must not leak through it",
        );
    }

    /// AC4: cyclic barrels terminate, with no panic and no unbounded work.
    #[test]
    fn cyclic_barrel_chains_terminate_deterministically() {
        let re_exports = ReExportMap::from([
            ("src/a.ts".to_owned(), vec![star("src/b.ts")]),
            ("src/b.ts".to_owned(), vec![star("src/a.ts")]),
        ]);

        let targets = barrel_targets("src/a.ts", "X", &re_exports);

        assert_eq!(
            targets,
            vec![
                ("src/a.ts".to_owned(), "X".to_owned()),
                ("src/b.ts".to_owned(), "X".to_owned()),
            ],
            "each (file, name) pair is visited once, so the cycle closes",
        );
        assert_eq!(
            barrel_targets("src/a.ts", "X", &re_exports),
            targets,
            "the result must be a pure function of the facts",
        );
    }

    /// A rename cycle cannot be closed by the visited set alone -- each hop
    /// invents a new name, so the pair is always fresh. The depth bound is
    /// what stops it.
    #[test]
    fn a_renaming_cycle_is_stopped_by_the_depth_bound() {
        let re_exports = ReExportMap::from([
            (
                "src/a.ts".to_owned(),
                vec![ReExport {
                    targets: vec!["src/b.ts".to_owned()],
                    // Every pass through renames `X` -> `Xx`, so no pair repeats.
                    kind: ReExportKind::Named {
                        exported: "Xx".to_owned(),
                        local: "X".to_owned(),
                    },
                }],
            ),
            (
                "src/b.ts".to_owned(),
                vec![ReExport {
                    targets: vec!["src/a.ts".to_owned()],
                    kind: ReExportKind::Named {
                        exported: "X".to_owned(),
                        local: "Xx".to_owned(),
                    },
                }],
            ),
        ]);

        let targets = barrel_targets("src/a.ts", "X", &re_exports);

        assert!(
            targets.len() <= super::MAX_BARREL_DEPTH + 1,
            "a renaming cycle must be bounded, got {} results",
            targets.len(),
        );
    }

    /// A file with no re-exports resolves to itself, which is what keeps
    /// non-barrel imports (and every Python import) unchanged.
    #[test]
    fn a_module_without_re_exports_resolves_to_itself() {
        assert_eq!(
            barrel_targets("src/provider.ts", "Provider", &ReExportMap::new()),
            vec![("src/provider.ts".to_owned(), "Provider".to_owned())],
        );
    }
}
