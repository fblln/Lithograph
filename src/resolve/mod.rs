//! Hybrid resolver framework (LIT-22.3.1): a deterministic post-build pass
//! over an already-constructed [`Graph`] that upgrades `SyntaxOnly`/
//! `Fallback` relations targeting an `Unresolved` node into resolved
//! targets, whenever a [`Resolver`] can prove the connection from typed
//! package/module/symbol indexes built from the graph itself.
//!
//! This module deliberately does not replace how `GraphBuilder` already
//! resolves Python/Rust imports at parse time (that stays `HybridResolved`
//! from the start, since those analyzers have full semantic context).
//! Instead it targets relations other analyzers could only resolve as
//! `SyntaxOnly`/`Fallback` -- e.g. LIT-22.2.3's generic tree-sitter import
//! facts, or LIT-22.2.4's package-manifest dependency edges -- by
//! cross-referencing them against the rest of the graph after the fact.
//! Per-language import resolvers (LIT-22.3.2) plug into this framework by
//! implementing [`Resolver`]; this module only owns the shared plumbing.

pub(crate) mod aliases;
pub(crate) mod barrels;
pub(crate) mod environment;
pub(crate) mod imports;
pub(crate) mod propagate;
pub(crate) mod symbols;

use crate::domain::Confidence;
use crate::graph::{
    Graph, GraphNode, GraphNodeId, NodeKindTag, Relation, RelationProvenance, RelationResolution,
    node_kind_tag, target_kind_allowed,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

pub(crate) use aliases::TsAliasMap;
pub(crate) use barrels::{ReExport, ReExportKind, ReExportMap, barrel_targets};
pub(crate) use environment::{
    ConfigFact, EnvFact, EnvironmentExplanation, FactRole, FactSourceKind, explain_environment,
    resolve_environment_links,
};
pub(crate) use imports::{LanguageImportResolver, extract_import_reference};
pub(crate) use imports::{
    extract_typescript_default_import_binding, extract_typescript_import_bindings,
    import_candidates, typescript_dependency_root,
};
#[cfg(test)]
pub(crate) use propagate::PROPAGATE_STRATEGY;
pub(crate) use propagate::re_export_map;
pub(crate) use propagate::{
    BaseClassFact, BindingFact, FileTypeFacts, ImportBindingFact, MemberCallFact, Receiver,
    TypeFacts, propagate_types,
};
pub(crate) use symbols::{ImportLookup, ImportMap, ProjectSymbolRegistry};

/// Typed indexes over one graph snapshot, built once per pipeline run and
/// shared by every resolver (AC1: typed syntax/package/module/symbol
/// inputs). Keyed by the same string a resolver would extract from a
/// syntax fact -- a dotted/`::` module path, a package name, or a fully
/// qualified symbol name -- so a resolver's lookup is a single map access.
pub(crate) struct ResolverContext<'a> {
    /// Package node ids by package name.
    pub packages_by_name: BTreeMap<&'a str, &'a GraphNodeId>,
    /// Names of packages built in-repo (`is_external == false`), a subset
    /// of `packages_by_name`'s keys.
    pub local_package_names: BTreeSet<&'a str>,
    /// Symbol node ids by fully qualified name.
    pub symbols_by_qualified_name: BTreeMap<&'a str, &'a GraphNodeId>,
    /// Artifact node ids by repository-relative path.
    pub artifacts_by_path: BTreeMap<&'a str, &'a GraphNodeId>,
    /// Node kinds by identifier. The resolver pipeline uses the graph
    /// validator's allow-list before accepting a candidate, keeping invalid
    /// relation targets unconstructible instead of merely detectable later.
    node_kinds: BTreeMap<&'a GraphNodeId, NodeKindTag>,
    /// Shared project-wide declaration index for symbol-aware resolvers.
    pub symbols: ProjectSymbolRegistry,
    /// LIT-37: `Imports` relations grouped by their source node id, in graph
    /// relation order. Lets a per-call resolver find a source file's imports
    /// with one map lookup instead of scanning every relation, turning the
    /// TypeScript call resolver's O(calls x relations) hot path into O(calls).
    imports_by_source: HashMap<&'a GraphNodeId, Vec<&'a Relation>>,
    /// LIT-37: `Unresolved` node literal values by node id, so an import's raw
    /// value is a map lookup instead of a full `graph.nodes` scan per import.
    unresolved_values_by_id: HashMap<&'a GraphNodeId, &'a str>,
    /// LIT-37: ids of callable symbols (functions/methods), so candidate
    /// filtering is a set membership test instead of a full `graph.nodes` scan.
    callable_symbol_ids: HashSet<&'a GraphNodeId>,
    /// LIT-45.2: tsconfig `compilerOptions.paths` aliases. Not derivable from
    /// the graph -- it comes from config files the builder reads -- so it is
    /// supplied rather than indexed here. Empty for repositories with no
    /// tsconfig, which leaves every resolver's behaviour unchanged.
    pub ts_aliases: TsAliasMap,
    /// LIT-79: `export ... from` re-export statements by containing artifact,
    /// so a use-site reference imported through a barrel can be chased to the
    /// module that actually declares it. Like `ts_aliases`, this is builder
    /// state (from `FileTypeFacts::re_exports`) rather than a graph fact, so it
    /// is supplied; empty for repositories with no barrels, leaving resolution
    /// unchanged.
    pub re_exports: ReExportMap,
}

impl<'a> ResolverContext<'a> {
    /// Builds every index in one pass over `graph.nodes`, with no tsconfig
    /// aliases. Test-only convenience over
    /// [`Self::build_with_aliases_and_re_exports`].
    #[cfg(test)]
    pub(crate) fn build(graph: &'a Graph) -> Self {
        Self::build_with_aliases_and_re_exports(graph, TsAliasMap::default(), ReExportMap::new())
    }

    /// Builds every index in one pass over `graph.nodes`, with tsconfig aliases
    /// (LIT-45.2) and barrel re-exports (LIT-79) both available to resolvers.
    pub(crate) fn build_with_aliases_and_re_exports(
        graph: &'a Graph,
        ts_aliases: TsAliasMap,
        re_exports: ReExportMap,
    ) -> Self {
        let mut packages_by_name = BTreeMap::new();
        let mut local_package_names = BTreeSet::new();
        let mut symbols_by_qualified_name = BTreeMap::new();
        let mut artifacts_by_path = BTreeMap::new();
        let mut node_kinds = BTreeMap::new();
        let mut unresolved_values_by_id = HashMap::new();
        let mut callable_symbol_ids = HashSet::new();

        for node in &graph.nodes {
            node_kinds.insert(node.id(), node_kind_tag(node));
            match node {
                GraphNode::Package(package) => {
                    packages_by_name.insert(package.name.as_str(), node.id());
                    if !package.is_external {
                        local_package_names.insert(package.name.as_str());
                    }
                }
                GraphNode::Symbol(symbol) => {
                    symbols_by_qualified_name.insert(symbol.qualified_name.as_str(), node.id());
                    if matches!(
                        symbol.kind,
                        crate::graph::SymbolKind::Function | crate::graph::SymbolKind::Method
                    ) {
                        callable_symbol_ids.insert(node.id());
                    }
                }
                GraphNode::Artifact(artifact) => {
                    artifacts_by_path.insert(artifact.path.as_str(), node.id());
                }
                GraphNode::Unresolved(unresolved) => {
                    unresolved_values_by_id.insert(node.id(), unresolved.value.as_str());
                }
                GraphNode::Module(_)
                | GraphNode::Config(_)
                | GraphNode::Documentation(_)
                | GraphNode::Container(_)
                | GraphNode::Command(_)
                | GraphNode::Rationale(_)
                | GraphNode::EnvVar(_) => {}
            }
        }

        // Group imports by source in relation order, so the per-call resolver
        // sees the same imports it would by scanning `graph.relations` (LIT-37).
        let mut imports_by_source: HashMap<&GraphNodeId, Vec<&Relation>> = HashMap::new();
        for relation in &graph.relations {
            if relation.kind == crate::graph::RelationKind::Imports {
                imports_by_source
                    .entry(&relation.source)
                    .or_default()
                    .push(relation);
            }
        }

        Self {
            packages_by_name,
            local_package_names,
            symbols_by_qualified_name,
            artifacts_by_path,
            node_kinds,
            symbols: ProjectSymbolRegistry::build(graph),
            imports_by_source,
            unresolved_values_by_id,
            callable_symbol_ids,
            ts_aliases,
            re_exports,
        }
    }
}

/// One upgraded relation target, returned by a [`Resolver`] that proved a
/// connection.
pub(crate) struct ResolvedTarget {
    /// The node this relation should now point to, in place of its
    /// `Unresolved` target.
    pub target: GraphNodeId,
    /// Confidence in this specific resolution.
    pub confidence: Confidence,
}

/// One resolution strategy over typed graph facts (AC1/AC2). A resolver
/// only ever sees relations whose current target is an `Unresolved` node
/// and whose provenance resolution is `SyntaxOnly` or `Fallback` -- the
/// pipeline never asks a resolver to touch an already-resolved relation.
pub(crate) trait Resolver {
    /// Stable strategy label recorded in the upgraded relation's
    /// provenance (e.g. `"package-map-exact-match"`).
    fn strategy(&self) -> &'static str;

    /// Attempts to resolve `relation`, whose unresolved literal value is
    /// `unresolved_value` (the matching [`UnresolvedNode::value`](crate::graph::UnresolvedNode::value)).
    /// Returns `None` when this resolver has no opinion on `relation`, so
    /// the pipeline can try the next resolver in order.
    fn resolve(
        &self,
        context: &ResolverContext<'_>,
        relation: &Relation,
        unresolved_value: &str,
    ) -> Option<ResolvedTarget>;
}

/// Outcome of one [`HybridResolverPipeline::resolve`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct ResolveReport {
    /// Relations upgraded from `SyntaxOnly`/`Fallback` to `HybridResolved`.
    pub resolved: usize,
    /// Eligible relations no resolver could upgrade.
    pub still_unresolved: usize,
}

/// An ordered sequence of [`Resolver`]s applied deterministically to a
/// graph's relations (AC3: resolver ordering and conflict handling are
/// deterministic). Relations are visited in the graph's existing
/// deterministic `(source, kind, target)` order; for each, resolvers run
/// in list order and the first one to return `Some` wins -- so two
/// resolvers can never both claim the same relation, and re-running the
/// pipeline on the same graph always produces the same result.
pub(crate) struct HybridResolverPipeline {
    resolvers: Vec<Box<dyn Resolver>>,
}

impl HybridResolverPipeline {
    /// Builds a pipeline that tries `resolvers` in order for every eligible
    /// relation.
    pub(crate) fn new(resolvers: Vec<Box<dyn Resolver>>) -> Self {
        Self { resolvers }
    }

    /// The framework's built-in resolvers, most specific first:
    /// [`LanguageImportResolver`] (LIT-22.3.2) parses the raw unresolved
    /// text per source language before matching, so it tries first; the
    /// two generic exact-match resolvers (LIT-22.3.1) catch anything whose
    /// raw unresolved value already happens to equal a known package name
    /// or artifact path verbatim.
    pub(crate) fn default_pipeline() -> Self {
        Self::new(vec![
            Box::new(TypeScriptImportBindingResolver),
            Box::new(SymbolNameResolver),
            Box::new(LanguageImportResolver),
            Box::new(PackageMapResolver),
            Box::new(LocalArtifactPathResolver),
        ])
    }

    /// Runs every resolver against every eligible relation in `graph`,
    /// mutating resolved relations in place. Test-only convenience over
    /// [`Self::resolve_with_aliases_and_re_exports`].
    #[cfg(test)]
    pub(crate) fn resolve(&self, graph: &mut Graph) -> ResolveReport {
        self.resolve_with_aliases(graph, TsAliasMap::default())
    }

    /// Resolves with tsconfig path aliases available to the resolvers
    /// (LIT-45.2). Test-only convenience over
    /// [`Self::resolve_with_aliases_and_re_exports`].
    #[cfg(test)]
    pub(crate) fn resolve_with_aliases(
        &self,
        graph: &mut Graph,
        ts_aliases: TsAliasMap,
    ) -> ResolveReport {
        self.resolve_with_aliases_and_re_exports(graph, ts_aliases, ReExportMap::new())
    }

    /// Resolves with tsconfig aliases (LIT-45.2) and barrel re-exports
    /// (LIT-79) both available, so a use-site reference imported through a
    /// barrel can be chased to its declaring module.
    pub(crate) fn resolve_with_aliases_and_re_exports(
        &self,
        graph: &mut Graph,
        ts_aliases: TsAliasMap,
        re_exports: ReExportMap,
    ) -> ResolveReport {
        let unresolved_values = unresolved_node_values(graph);
        let mut report = ResolveReport::default();

        // Build the context from an immutable snapshot of the current
        // nodes/relations before mutating any relation, so a resolver's
        // lookup indexes never observe a partially-updated graph -- and so
        // resolvers can never observe each other's output within one run,
        // keeping ordering effects limited to "who claims a relation
        // first," not "what facts are visible."
        let context =
            ResolverContext::build_with_aliases_and_re_exports(&*graph, ts_aliases, re_exports);
        let mut updates: Vec<(usize, ResolvedTarget, &'static str)> = Vec::new();

        for (index, relation) in graph.relations.iter().enumerate() {
            if !eligible_for_resolution(relation) {
                continue;
            }
            let Some(unresolved_value) = unresolved_values.get(&relation.target) else {
                continue;
            };
            let resolved = self.resolvers.iter().find_map(|resolver| {
                let resolved = resolver.resolve(&context, relation, unresolved_value)?;
                let target_kind = context.node_kinds.get(&resolved.target)?;
                target_kind_allowed(relation.kind, *target_kind)
                    .then_some((resolved, resolver.strategy()))
            });
            match resolved {
                Some((resolved, strategy)) => updates.push((index, resolved, strategy)),
                None => report.still_unresolved += 1,
            }
        }

        for (index, resolved, strategy) in updates {
            let relation = &mut graph.relations[index];
            relation.target = resolved.target;
            relation.confidence = resolved.confidence;
            let language = relation
                .provenance
                .as_ref()
                .and_then(|provenance| provenance.language.clone());
            relation.provenance = Some(RelationProvenance {
                language,
                resolver_strategy: strategy.to_owned(),
                resolution: RelationResolution::HybridResolved,
                confidence: resolved.confidence,
            });
            report.resolved += 1;
        }

        report
    }
}

/// Resolves an unqualified call/type/use only when the project registry has a
/// single deterministic candidate; ambiguity deliberately remains visible.
struct SymbolNameResolver;

impl Resolver for SymbolNameResolver {
    fn strategy(&self) -> &'static str {
        "project-symbol-import-map"
    }

    fn resolve(
        &self,
        context: &ResolverContext<'_>,
        relation: &Relation,
        unresolved_value: &str,
    ) -> Option<ResolvedTarget> {
        use crate::graph::RelationKind;
        let map = ImportMap::new(&context.symbols);
        // The original whole-project unique-name/suffix match, for calls only.
        if relation.kind == RelationKind::Calls {
            match map.lookup(None, None, unresolved_value) {
                ImportLookup::Suffix { target, confidence }
                | ImportLookup::UniqueName { target, confidence } => {
                    return Some(ResolvedTarget { target, confidence });
                }
                ImportLookup::SameModule { .. }
                | ImportLookup::ExplicitImport { .. }
                | ImportLookup::Ambiguous { .. }
                | ImportLookup::Unresolved => {}
            }
        }
        // LIT-82: a call or type reference whose name is unique among symbols of
        // its own language resolves there even when a same-named symbol in
        // another language makes the whole-project lookup `Ambiguous` -- e.g.
        // `from app.models import Message; Message(...)` in a `.py` route, where
        // an unrelated TypeScript `Message` model shares the name. A reference
        // can only bind within its language, so this is a disambiguation, not a
        // new guess: two same-language rivals still stay unresolved.
        if matches!(relation.kind, RelationKind::Calls | RelationKind::UsesType) {
            let language = relation.provenance.as_ref()?.language.as_deref()?;
            if let Some(target) = map.unique_in_language(unresolved_value, language) {
                return Some(ResolvedTarget {
                    target,
                    confidence: Confidence::Low,
                });
            }
        }
        None
    }
}

/// Resolves a TypeScript/TSX reference -- a call, a JSX/use-site `Usages`, or
/// a `TypeRefs` -- to a local symbol reached through a direct binding from a
/// relative import. It runs before import resolution mutates the raw
/// statement away, handles both named (`import { X }`) and default
/// (`import X`) bindings, and accepts only one symbol match; namespace,
/// renamed-default, missing, and ambiguous bindings remain unresolved.
///
/// LIT-75: broadened from `Calls`-only to `Usages`/`TypeRefs` as well, which
/// is the largest single class of remaining Unresolved nodes on real
/// front-ends -- `<ActionsMenu/>` (a JSX `Usages`) and `props: ApiError` (a
/// `TypeRefs`) each imported once from a local file, previously minting a
/// per-file `unresolved:<name>` node instead of pointing at the definition
/// the graph already held. The honesty boundary is unchanged: the reference
/// name must be a binding of an actual relative import in this file, and the
/// import's target file must define a symbol with the corresponding name;
/// zero or multiple candidates stay unresolved rather than guess.
struct TypeScriptImportBindingResolver;

impl Resolver for TypeScriptImportBindingResolver {
    fn strategy(&self) -> &'static str {
        "typescript-import-binding"
    }

    fn resolve(
        &self,
        context: &ResolverContext<'_>,
        relation: &Relation,
        unresolved_value: &str,
    ) -> Option<ResolvedTarget> {
        use crate::graph::RelationKind;
        // A call must land on a callable symbol (preserving the original
        // conservatism); a use-site or type reference may name any local
        // symbol -- a component, class, interface, type alias, or enum.
        let require_callable = match relation.kind {
            RelationKind::Calls => true,
            RelationKind::Usages | RelationKind::TypeRefs => false,
            _ => return None,
        };
        let language = relation.provenance.as_ref()?.language.as_deref()?;
        if !matches!(language, "typescript" | "tsx") {
            return None;
        }
        let source_path = relation.source.as_str().strip_prefix("artifact:")?;
        let mut candidates = BTreeSet::new();

        // LIT-75 same-file case: a reference to a top-level symbol declared in
        // this very file (a component referencing its own name, a type used
        // where it is defined) needs no import -- the definition is right
        // here. The builder already resolves same-file *calls* via its
        // callable index; this covers the use-site and type references it
        // leaves behind. Matched by exact `{file}::{name}` qualified name, so
        // it never reaches into a nested class method or another file.
        if let Some(symbol_id) = context
            .symbols_by_qualified_name
            .get(format!("{source_path}::{unresolved_value}").as_str())
            && (!require_callable || context.callable_symbol_ids.contains(symbol_id))
        {
            candidates.insert((*symbol_id).clone());
        }

        // LIT-79 barrel-file case: the source file is itself a barrel that
        // re-exports this name (`client/index.ts` says
        // `export { ItemsService } from './services/ItemsService'`). The
        // re-export specifier's own identifier reaches this pass as an
        // unresolved `Usages`, and left alone it keeps the `unresolved:<name>`
        // node alive even after every importer has resolved. Chasing the file's
        // own re-export chain resolves that last edge to the declaring symbol,
        // turning a spurious use into the true fact that the barrel republishes
        // it. A no-op for a non-barrel file, whose re-export map entry is empty.
        for (declaring_module, declared_name) in
            barrel_targets(source_path, unresolved_value, &context.re_exports)
        {
            if let Some(symbol_id) = context
                .symbols_by_qualified_name
                .get(format!("{declaring_module}::{declared_name}").as_str())
                && (!require_callable || context.callable_symbol_ids.contains(symbol_id))
            {
                candidates.insert((*symbol_id).clone());
            }
        }

        // LIT-37: same-source imports come from the prebuilt index (relation
        // order preserved) rather than a full scan of every graph relation.
        let source_imports = context
            .imports_by_source
            .get(&relation.source)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        for import in source_imports.iter().filter(|import| {
            import
                .provenance
                .as_ref()
                .and_then(|provenance| provenance.language.as_deref())
                == Some(language)
        }) {
            let Some(raw_import) = context.unresolved_values_by_id.get(&import.target).copied()
            else {
                continue;
            };
            // The exported name this file's binding refers to. A named
            // binding carries it explicitly (`{ X as y }` -> X); a default
            // binding does not name the export, so we match by convention on
            // the local binding name and rely on the target file actually
            // defining a symbol of that name below -- a renamed default
            // (`import Foo from "./ActionsMenu"`) finds no `ActionsMenu.tsx::Foo`
            // and correctly stays unresolved.
            let exported = extract_typescript_import_bindings(raw_import)
                .into_iter()
                .find(|(_, local)| local == unresolved_value)
                .map(|(exported, _)| exported)
                .or_else(|| {
                    extract_typescript_default_import_binding(raw_import)
                        .filter(|local| local == unresolved_value)
                });
            let Some(exported) = exported else {
                continue;
            };
            let Some(reference) = extract_import_reference(language, raw_import) else {
                continue;
            };
            if !(reference.starts_with("./") || reference.starts_with("../")) {
                continue;
            }
            for artifact_path in typescript_import_candidates(source_path, &reference, language) {
                // LIT-79: the imported module may be a barrel that re-exports
                // `exported` from elsewhere (`client/index.ts` says
                // `export { ItemsService } from './services/ItemsService'`)
                // rather than declaring it. `barrel_targets` follows the
                // re-export chain -- named, aliased, and star -- and returns
                // every `(module, name)` the symbol could actually resolve to,
                // including the barrel itself, so a file that both re-exports
                // and declares still matches. Renames are handled by the pair's
                // name, not the original. With an empty re-export map this
                // yields exactly `[(artifact_path, exported)]`, i.e. the prior
                // direct-lookup behaviour.
                for (declaring_module, declared_name) in
                    barrel_targets(&artifact_path, &exported, &context.re_exports)
                {
                    let qualified = format!("{declaring_module}::{declared_name}");
                    let Some(symbol_id) = context.symbols_by_qualified_name.get(qualified.as_str())
                    else {
                        continue;
                    };
                    if !require_callable || context.callable_symbol_ids.contains(symbol_id) {
                        candidates.insert((*symbol_id).clone());
                    }
                }
            }
        }

        let mut candidates = candidates.into_iter();
        let target = candidates.next()?;
        candidates.next().is_none().then_some(ResolvedTarget {
            target,
            confidence: Confidence::High,
        })
    }
}

/// Artifact paths a relative TypeScript import specifier could name, most
/// likely first. Shared with the graph builder, which needs the same
/// candidates to seed a file's binding environment (LIT-57).
///
/// LIT-45.3: this used to join the path itself and try only `{base}.ts` and
/// `{base}.tsx`, so it silently missed every directory import -- `from
/// '../../client'` naming `client/index.ts` resolved to nothing. It now defers
/// to the same candidate order LIT-45.1 documented for the import resolver,
/// because two functions answering "what path does this specifier name" with
/// different answers is how that gap survived in the first place.
pub(crate) fn typescript_import_candidates(
    source_path: &str,
    reference: &str,
    language: &str,
) -> Vec<String> {
    let source_dir = std::path::Path::new(source_path)
        .parent()
        .unwrap_or(std::path::Path::new(""));
    let candidate = imports::resolve_relative_path(source_dir, reference);
    imports::import_candidates(&candidate, language)
}

/// A relation is eligible for hybrid resolution when it hasn't already
/// been resolved (or explicitly marked as a bare syntax/fallback fact by a
/// resolver that already tried and gave up -- there is no such marker
/// today, so every `SyntaxOnly`/`Fallback` relation is always retried).
fn eligible_for_resolution(relation: &Relation) -> bool {
    relation.provenance.as_ref().is_some_and(|provenance| {
        matches!(
            provenance.resolution,
            RelationResolution::SyntaxOnly | RelationResolution::Fallback
        )
    })
}

fn unresolved_node_values(graph: &Graph) -> BTreeMap<GraphNodeId, &str> {
    graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Unresolved(unresolved) => {
                Some((node.id().clone(), unresolved.value.as_str()))
            }
            _ => None,
        })
        .collect()
}

/// Resolves an `Unresolved` value that exactly matches a package name
/// already present in the graph's package map (LIT-22.2.4) -- e.g. an
/// `Imports` relation whose raw import text collapses to a bare package
/// name a `package.json`/`go.mod`/etc. also declared as a dependency.
struct PackageMapResolver;

impl Resolver for PackageMapResolver {
    fn strategy(&self) -> &'static str {
        "package-map-exact-match"
    }

    fn resolve(
        &self,
        context: &ResolverContext<'_>,
        _relation: &Relation,
        unresolved_value: &str,
    ) -> Option<ResolvedTarget> {
        context
            .packages_by_name
            .get(unresolved_value)
            .map(|target| ResolvedTarget {
                target: (*target).clone(),
                confidence: Confidence::High,
            })
    }
}

/// Resolves an `Unresolved` value that is a repository-relative (or
/// `./`-prefixed) path exactly matching a known artifact.
struct LocalArtifactPathResolver;

impl Resolver for LocalArtifactPathResolver {
    fn strategy(&self) -> &'static str {
        "local-artifact-path-exact-match"
    }

    fn resolve(
        &self,
        context: &ResolverContext<'_>,
        _relation: &Relation,
        unresolved_value: &str,
    ) -> Option<ResolvedTarget> {
        let normalized = unresolved_value.trim_start_matches("./");
        context
            .artifacts_by_path
            .get(normalized)
            .map(|target| ResolvedTarget {
                target: (*target).clone(),
                confidence: Confidence::High,
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{HybridResolverPipeline, ResolvedTarget, Resolver, ResolverContext};
    use crate::domain::{Confidence, EvidenceRef};
    use crate::graph::{
        Graph, GraphNode, GraphNodeId, PackageNode, Relation, RelationKind, RelationProvenance,
        RelationResolution, UnresolvedNode,
    };

    fn package(name: &str, is_external: bool) -> GraphNode {
        GraphNode::Package(PackageNode {
            id: GraphNodeId::new(format!("package:{name}")),
            name: name.to_owned(),
            is_external,
        })
    }

    fn unresolved(value: &str) -> GraphNode {
        GraphNode::Unresolved(UnresolvedNode {
            id: GraphNodeId::new(format!("unresolved:{value}")),
            value: value.to_owned(),
        })
    }

    fn relation(id: &str, source: &str, target: &str, resolution: RelationResolution) -> Relation {
        relation_of_kind(id, source, target, RelationKind::Imports, resolution)
    }

    fn relation_of_kind(
        id: &str,
        source: &str,
        target: &str,
        kind: RelationKind,
        resolution: RelationResolution,
    ) -> Relation {
        Relation {
            id: id.to_owned(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence: Confidence::Low,
            evidence: Vec::new(),
            provenance: Some(RelationProvenance {
                language: Some("javascript".to_owned()),
                resolver_strategy: "syntax-extraction".to_owned(),
                resolution,
                confidence: Confidence::Low,
            }),
        }
    }

    #[test]
    fn package_map_resolver_upgrades_exact_name_match() -> Result<(), Box<dyn std::error::Error>> {
        let mut graph = Graph {
            nodes: vec![package("react", true), unresolved("react")],
            relations: vec![relation(
                "relation:1",
                "artifact:App.tsx",
                "unresolved:react",
                RelationResolution::SyntaxOnly,
            )],
        };

        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 1);
        assert_eq!(report.still_unresolved, 0);
        assert_eq!(graph.relations[0].target, GraphNodeId::new("package:react"));
        assert_eq!(graph.relations[0].confidence, Confidence::High);
        let provenance = graph.relations[0]
            .provenance
            .as_ref()
            .ok_or("missing provenance")?;
        assert_eq!(provenance.resolution, RelationResolution::HybridResolved);
        assert_eq!(provenance.resolver_strategy, "package-map-exact-match");
        assert_eq!(provenance.language.as_deref(), Some("javascript"));

        Ok(())
    }

    #[test]
    fn package_targets_are_rejected_for_calls_and_uses_type()
    -> Result<(), Box<dyn std::error::Error>> {
        for (kind, package_name) in [
            (RelationKind::Calls, "react"),
            (RelationKind::UsesType, "json"),
        ] {
            let mut graph = Graph {
                nodes: vec![package(package_name, true), unresolved(package_name)],
                relations: vec![relation_of_kind(
                    "relation:1",
                    "artifact:App.tsx",
                    &format!("unresolved:{package_name}"),
                    kind,
                    RelationResolution::SyntaxOnly,
                )],
            };

            let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

            assert_eq!(report.resolved, 0, "{kind:?} must not target a package");
            assert_eq!(report.still_unresolved, 1);
            assert_eq!(
                graph.relations[0].target,
                GraphNodeId::new(format!("unresolved:{package_name}"))
            );
        }

        Ok(())
    }

    #[test]
    fn unresolvable_relations_are_left_syntax_only() -> Result<(), Box<dyn std::error::Error>> {
        let mut graph = Graph {
            nodes: vec![unresolved("some-unknown-thing")],
            relations: vec![relation(
                "relation:1",
                "artifact:App.tsx",
                "unresolved:some-unknown-thing",
                RelationResolution::SyntaxOnly,
            )],
        };

        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 0);
        assert_eq!(report.still_unresolved, 1);
        assert_eq!(
            graph.relations[0].target,
            GraphNodeId::new("unresolved:some-unknown-thing")
        );
        assert_eq!(
            graph.relations[0]
                .provenance
                .as_ref()
                .ok_or("missing provenance")?
                .resolution,
            RelationResolution::SyntaxOnly
        );

        Ok(())
    }

    #[test]
    fn already_hybrid_resolved_relations_are_left_untouched() {
        let mut graph = Graph {
            nodes: vec![package("react", true)],
            relations: vec![relation(
                "relation:1",
                "artifact:App.tsx",
                "package:react",
                RelationResolution::HybridResolved,
            )],
        };
        let original = graph.relations[0].clone();

        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 0);
        assert_eq!(report.still_unresolved, 0);
        assert_eq!(graph.relations[0], original);
    }

    #[test]
    fn first_matching_resolver_wins_deterministically() -> Result<(), Box<dyn std::error::Error>> {
        struct AlwaysResolvesToA;
        impl Resolver for AlwaysResolvesToA {
            fn strategy(&self) -> &'static str {
                "always-a"
            }
            fn resolve(
                &self,
                _context: &ResolverContext<'_>,
                _relation: &Relation,
                _unresolved_value: &str,
            ) -> Option<ResolvedTarget> {
                Some(ResolvedTarget {
                    target: GraphNodeId::new("package:a"),
                    confidence: Confidence::High,
                })
            }
        }
        struct AlwaysResolvesToB;
        impl Resolver for AlwaysResolvesToB {
            fn strategy(&self) -> &'static str {
                "always-b"
            }
            fn resolve(
                &self,
                _context: &ResolverContext<'_>,
                _relation: &Relation,
                _unresolved_value: &str,
            ) -> Option<ResolvedTarget> {
                Some(ResolvedTarget {
                    target: GraphNodeId::new("package:b"),
                    confidence: Confidence::High,
                })
            }
        }

        let mut graph = Graph {
            nodes: vec![
                package("a", true),
                package("b", true),
                unresolved("anything"),
            ],
            relations: vec![relation(
                "relation:1",
                "artifact:App.tsx",
                "unresolved:anything",
                RelationResolution::SyntaxOnly,
            )],
        };
        let pipeline = HybridResolverPipeline::new(vec![
            Box::new(AlwaysResolvesToA),
            Box::new(AlwaysResolvesToB),
        ]);

        pipeline.resolve(&mut graph);

        assert_eq!(graph.relations[0].target, GraphNodeId::new("package:a"));
        assert_eq!(
            graph.relations[0]
                .provenance
                .as_ref()
                .ok_or("missing provenance")?
                .resolver_strategy,
            "always-a"
        );

        Ok(())
    }

    #[test]
    fn local_artifact_path_resolver_matches_dot_slash_prefixed_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo_path = crate::domain::RepoPath::new("src/lib.rs")?;
        let mut graph = Graph {
            nodes: vec![
                GraphNode::Artifact(crate::graph::ArtifactNode {
                    id: GraphNodeId::new("artifact:src/lib.rs"),
                    path: "src/lib.rs".to_owned(),
                    category: crate::domain::ArtifactCategory::SourceCode,
                    evidence: EvidenceRef::file(
                        crate::domain::ArtifactId::from_path(&repo_path),
                        repo_path,
                    ),
                }),
                unresolved("./src/lib.rs"),
            ],
            relations: vec![relation(
                "relation:1",
                "artifact:main.rs",
                "unresolved:./src/lib.rs",
                RelationResolution::Fallback,
            )],
        };

        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 1);
        assert_eq!(
            graph.relations[0].target,
            GraphNodeId::new("artifact:src/lib.rs")
        );

        Ok(())
    }

    #[test]
    fn resolver_context_indexes_are_deterministic_and_provenance_round_trips_json()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = Graph {
            nodes: vec![package("left-pad", true), package("react", true)],
            relations: vec![relation(
                "relation:1",
                "artifact:App.tsx",
                "package:react",
                RelationResolution::HybridResolved,
            )],
        };
        let context = ResolverContext::build(&graph);
        assert_eq!(context.packages_by_name.len(), 2);
        assert!(context.packages_by_name.contains_key("left-pad"));
        assert!(context.packages_by_name.contains_key("react"));

        let json = serde_json::to_string(&graph.relations[0])?;
        let round_tripped: Relation = serde_json::from_str(&json)?;
        assert_eq!(round_tripped, graph.relations[0]);

        Ok(())
    }
}
