//! LIT-57: cross-file type propagation in topological import order.
//!
//! Lithograph resolved a call only when exactly one graph-wide symbol shared
//! the callee's name, so `receiver.method()` had nowhere to go: the call site
//! alone does not say what `receiver` is. Rather than guess per language, this
//! pass carries the evidence that answers it.
//!
//! Each file gets a binding environment mapping a local name to the class it
//! denotes, seeded by that file's resolved imports and extended by its own
//! `name = Ctor(...)` bindings. Files are visited in topological import order,
//! so a module-level binding in `b.py` is typed before `c.py`, which imports
//! it, is visited. A receiver typed through that environment resolves its
//! member call to a method on that class; a receiver that stays untyped
//! resolves to nothing.
//!
//! The pass never falls back to a name match. An unknown receiver produces no
//! edge, because a missing edge is recoverable and a fabricated one makes the
//! graph lie (LIT-63).

use crate::domain::{Confidence, EvidenceRef};
use crate::graph::{
    Graph, GraphNode, GraphNodeId, Relation, RelationKind, RelationProvenance, RelationResolution,
    SymbolKind,
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Stable strategy label recorded on every relation this pass creates.
pub const PROPAGATE_STRATEGY: &str = "cross-file-type-propagation";

/// What a member call's receiver refers to. Producers normalize the language's
/// own spelling (`self`/`cls` in Python, `this` in TypeScript) into
/// [`Receiver::Enclosing`] so this pass stays language-neutral.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Receiver {
    /// The enclosing class instance, typed by the class the call sits in.
    Enclosing,
    /// A bare name, typed by the file's binding environment.
    Named(String),
}

/// One `receiver.method(...)` call awaiting a receiver type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberCallFact {
    /// What the call is invoked on.
    pub receiver: Receiver,
    /// Called method name.
    pub method: String,
    /// Enclosing class name, when the call sits inside a class body.
    pub enclosing_class: Option<String>,
    /// Evidence for the call expression.
    pub evidence: EvidenceRef,
}

/// One `name = Ctor(...)` binding, which types `name` as `Ctor`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingFact {
    /// Bound name.
    pub name: String,
    /// Constructor name as written, resolved through the same environment.
    pub constructor: String,
    /// True when the binding is visible to importing files.
    pub is_module_level: bool,
}

/// One declared superclass awaiting exact resolution through the file's
/// import/local-class environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BaseClassFact {
    /// Class that declares the superclass.
    pub class: String,
    /// Superclass expression as written.
    pub base: String,
    /// Evidence for the class declaration.
    pub evidence: EvidenceRef,
}

/// One imported name, already resolved to the module that declares it.
///
/// `module` is whatever namespace the declaring file's symbols are qualified
/// by -- a dotted module path for Python, an artifact path for TypeScript --
/// so `{module}::{symbol}` is an exact lookup rather than a name search.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImportBindingFact {
    /// Name as bound in the importing file, after any alias.
    pub local: String,
    /// Namespace that declares the imported name.
    pub modules: Vec<String>,
    /// Name as declared by that module.
    pub symbol: String,
}

/// Every fact one file contributes to propagation.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FileTypeFacts {
    /// Namespace this file's own symbols are qualified by.
    pub module: String,
    /// Source language label, recorded on created relations.
    pub language: String,
    /// Names this file imports, with the module declaring each.
    pub imports: Vec<ImportBindingFact>,
    /// Names this file binds to a construction.
    pub bindings: Vec<BindingFact>,
    /// Declared superclass relationships resolved before member calls.
    pub bases: Vec<BaseClassFact>,
    /// Member calls awaiting a receiver type.
    pub member_calls: Vec<MemberCallFact>,
    /// LIT-45.3: `export ... from` statements, when this file is a barrel.
    /// Empty for languages without them, which leaves resolution unchanged.
    pub re_exports: Vec<crate::resolve::ReExport>,
}

/// Per-artifact facts, keyed by repository-relative path.
pub type TypeFacts = BTreeMap<String, FileTypeFacts>;

/// LIT-45.3: re-export statements by containing artifact, built from
/// [`FileTypeFacts::re_exports`] once per run.
fn re_export_map(facts: &TypeFacts) -> crate::resolve::ReExportMap {
    facts
        .iter()
        .filter(|(_, file)| !file.re_exports.is_empty())
        .map(|(artifact, file)| (artifact.clone(), file.re_exports.clone()))
        .collect()
}

/// Outcome of one [`propagate_types`] run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PropagateReport {
    /// Member calls resolved to a method, each of which created a relation.
    pub resolved: usize,
    /// Member calls whose receiver could not be typed. No edge was created
    /// for these; they are counted so the cost of the conservative choice
    /// stays visible rather than silently absorbed.
    pub untyped: usize,
}

/// Resolves member calls through per-file binding environments and appends a
/// `Calls` relation for each one whose receiver types to a class with that
/// method.
pub fn propagate_types(graph: &mut Graph, facts: &TypeFacts) -> PropagateReport {
    let mut index = SymbolIndex::build(graph);
    let re_exports = re_export_map(facts);
    let mut report = PropagateReport::default();
    let mut created: Vec<Relation> = Vec::new();

    // Module-level bindings exported by each already-visited module. This is
    // the state that makes the visit order matter: `c.py` importing a name
    // bound in `b.py` can only type it once `b.py` has been visited.
    let mut exported: BTreeMap<&str, BTreeMap<String, GraphNodeId>> = BTreeMap::new();

    for artifact in visit_order(facts) {
        let Some(file) = facts.get(artifact) else {
            continue;
        };
        let environment = build_environment(artifact, file, &index, &exported, &re_exports);

        // TypeScript inheritance reaches this pass as analyzer evidence rather
        // than an unresolved name edge. Resolve it through the same exact
        // import/local-class environment used for receivers, emit the graph
        // fact, and update the in-memory index before resolving `this` calls.
        for base in &file.bases {
            let Some(child) = index.class(&format!("{}::{}", file.module, base.class)) else {
                continue;
            };
            let Some(parent) = resolve_constructor(&base.base, &environment, &index) else {
                continue;
            };
            index.add_base(child.clone(), parent.clone());
            created.push(inheritance_relation(
                artifact,
                &child,
                &parent,
                base,
                &file.language,
            ));
        }
        exported.insert(
            file.module.as_str(),
            file.bindings
                .iter()
                .filter(|binding| binding.is_module_level)
                .filter_map(|binding| {
                    Some((
                        binding.name.clone(),
                        environment.get(&binding.name)?.clone(),
                    ))
                })
                .collect(),
        );

        for (call_index, call) in file.member_calls.iter().enumerate() {
            match resolve_member_call(artifact, file, call, &environment, &index) {
                Some(target) => {
                    created.push(call_relation(
                        artifact,
                        &target,
                        call,
                        &file.language,
                        call_index,
                    ));
                    report.resolved += 1;
                }
                None => report.untyped += 1,
            }
        }
    }

    graph.relations.extend(created);
    report
}

/// Types every name this file can see: imported names first, then the file's
/// own classes, then its constructions, which may use either.
fn build_environment(
    artifact: &str,
    file: &FileTypeFacts,
    index: &SymbolIndex,
    exported: &BTreeMap<&str, BTreeMap<String, GraphNodeId>>,
    re_exports: &crate::resolve::ReExportMap,
) -> BTreeMap<String, GraphNodeId> {
    let mut environment = BTreeMap::new();

    let mut imported: BTreeMap<&str, BTreeSet<GraphNodeId>> = BTreeMap::new();
    for import in &file.imports {
        // LIT-45.3: the imported module may be a barrel that only republishes
        // the name, so every module the chain reaches is a candidate -- under
        // whatever name that module knows it by, since a re-export can rename.
        // With no re-exports this yields just the imported module itself,
        // which is why Python behaviour is unchanged.
        let targets: BTreeSet<_> = import
            .modules
            .iter()
            .flat_map(|module| crate::resolve::barrel_targets(module, &import.symbol, re_exports))
            .filter_map(|(module, symbol)| {
                // `from a import Provider` binds a class directly; `from b
                // import provider` binds a name `b` itself bound to a
                // construction, typed only because `b` was visited first.
                index.class(&format!("{module}::{symbol}")).or_else(|| {
                    exported
                        .get(module.as_str())
                        .and_then(|names| names.get(&symbol))
                        .cloned()
                })
            })
            .collect();
        for target in targets {
            imported
                .entry(import.local.as_str())
                .or_default()
                .insert(target);
        }
    }
    for (local, mut targets) in imported {
        if targets.len() == 1
            && let Some(target) = targets.pop_first()
        {
            environment.insert(local.to_owned(), target);
        }
    }

    // A class declared here shadows an import of the same name, matching how
    // both languages bind a local declaration over an imported one.
    for (name, id) in index.classes_in(artifact) {
        environment.insert(name, id);
    }

    // Bindings are resolved against imports and local classes only, then added
    // together -- a name is never typed by another binding, because `y = x()`
    // calls an instance rather than naming a class.
    //
    // This environment is per file, not per scope, so a name a file binds
    // twice to different classes has no single answer here. That is common in
    // test files, where each case rebinds the same local:
    //
    //     it('a', () => { const logger = new ConsoleLogger(); logger.error(...) })
    //     it('b', () => { const logger = new Logger();        logger.error(...) })
    //
    // Letting either binding win types the other case's calls wrong. Measured
    // on the pinned NestJS corpus, taking the last binding produced 20 edges
    // into a class the receiver never held -- `Logger::error` for a receiver
    // built as `ConsoleLogger`, which merely shares an interface with it. So
    // an over-bound name is treated as ambiguous and typed not at all, the
    // same rule the rest of the resolver applies to a name it cannot pin down.
    //
    // A binding whose constructor does not resolve counts too, as `None`: the
    // class may be declared inside a function body, which no analyzer here
    // extracts, so the name is bound to something real the graph cannot see.
    // Typing that call from a different binding of the same name is the same
    // mistake, made against a class we cannot even inspect. On this corpus it
    // is `class CustomConsoleLogger extends ConsoleLogger` declared inside a
    // test callback -- were it to override the method being called, the
    // visible base would be the wrong answer.
    let mut bound: BTreeMap<&str, BTreeSet<Option<GraphNodeId>>> = BTreeMap::new();
    for binding in &file.bindings {
        let target = resolve_constructor(&binding.constructor, &environment, index);
        bound
            .entry(binding.name.as_str())
            .or_default()
            .insert(target);
    }
    for (name, mut targets) in bound {
        if targets.len() == 1
            && let Some(Some(target)) = targets.pop_first()
        {
            environment.insert(name.to_owned(), target);
        }
    }

    environment
}

/// Types a constructor name through the environment. A dotted constructor
/// (`module.Provider`) is typed only by its bound head, never by its trailing
/// segment alone -- matching on the last segment is the name match this pass
/// exists to replace.
fn resolve_constructor(
    constructor: &str,
    environment: &BTreeMap<String, GraphNodeId>,
    index: &SymbolIndex,
) -> Option<GraphNodeId> {
    if let Some(target) = environment.get(constructor) {
        return Some(target.clone());
    }
    let (head, rest) = constructor.split_once('.')?;
    let head_module = index.module_of(environment.get(head)?)?;
    index.class(&format!("{head_module}::{rest}"))
}

/// Types the receiver, then looks for the method on that class or a base.
fn resolve_member_call(
    artifact: &str,
    file: &FileTypeFacts,
    call: &MemberCallFact,
    environment: &BTreeMap<String, GraphNodeId>,
    index: &SymbolIndex,
) -> Option<GraphNodeId> {
    let class = match &call.receiver {
        Receiver::Enclosing => {
            let name = call.enclosing_class.as_deref()?;
            index.class(&format!("{}::{}", file.module, name))?
        }
        Receiver::Named(name) => {
            // A name bound inside a class body is only reachable from the
            // file's environment; `artifact` is unused for named receivers
            // beyond the environment already built for it.
            let _ = artifact;
            environment.get(name)?.clone()
        }
    };
    index.method(&class, &call.method)
}

/// The graph facts this pass reads, indexed once.
struct SymbolIndex {
    /// Class symbol id by qualified name. Names shared by two classes are
    /// excluded outright: an ambiguous class cannot type a receiver.
    classes_by_qualified_name: BTreeMap<String, GraphNodeId>,
    /// Method symbol id by qualified name, excluding ambiguous names.
    methods_by_qualified_name: BTreeMap<String, GraphNodeId>,
    /// Qualified name by class symbol id, for typing a dotted constructor.
    qualified_name_by_id: BTreeMap<GraphNodeId, String>,
    /// Class symbol ids declared by each artifact, with their simple names.
    classes_by_artifact: BTreeMap<String, Vec<(String, GraphNodeId)>>,
    /// Declared base classes of each class, in declaration order.
    bases: BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
}

impl SymbolIndex {
    fn build(graph: &Graph) -> Self {
        let mut classes: BTreeMap<String, BTreeSet<GraphNodeId>> = BTreeMap::new();
        let mut methods: BTreeMap<String, BTreeSet<GraphNodeId>> = BTreeMap::new();
        let mut qualified_name_by_id = BTreeMap::new();
        let mut classes_by_artifact: BTreeMap<String, Vec<(String, GraphNodeId)>> = BTreeMap::new();

        for node in &graph.nodes {
            let GraphNode::Symbol(symbol) = node else {
                continue;
            };
            match symbol.kind {
                SymbolKind::Class => {
                    classes
                        .entry(symbol.qualified_name.clone())
                        .or_default()
                        .insert(symbol.id.clone());
                    qualified_name_by_id.insert(symbol.id.clone(), symbol.qualified_name.clone());
                    let simple = symbol
                        .qualified_name
                        .rsplit("::")
                        .next()
                        .unwrap_or(&symbol.qualified_name)
                        .to_owned();
                    classes_by_artifact
                        .entry(symbol.evidence.path.as_str().to_owned())
                        .or_default()
                        .push((simple, symbol.id.clone()));
                }
                SymbolKind::Method => {
                    methods
                        .entry(symbol.qualified_name.clone())
                        .or_default()
                        .insert(symbol.id.clone());
                }
                _ => {}
            }
        }

        // Only a class that inherits from a resolved class symbol contributes
        // a base. An `Inherits` edge still pointing at an `Unresolved` node
        // names a base we cannot see, and walking it would invent a chain.
        let class_ids: BTreeSet<_> = qualified_name_by_id.keys().cloned().collect();
        let mut bases: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
        for relation in &graph.relations {
            if relation.kind == RelationKind::Inherits
                && class_ids.contains(&relation.source)
                && class_ids.contains(&relation.target)
            {
                bases
                    .entry(relation.source.clone())
                    .or_default()
                    .push(relation.target.clone());
            }
        }

        Self {
            classes_by_qualified_name: singletons(classes),
            methods_by_qualified_name: singletons(methods),
            qualified_name_by_id,
            classes_by_artifact,
            bases,
        }
    }

    fn class(&self, qualified_name: &str) -> Option<GraphNodeId> {
        self.classes_by_qualified_name.get(qualified_name).cloned()
    }

    fn module_of(&self, class: &GraphNodeId) -> Option<&str> {
        let qualified = self.qualified_name_by_id.get(class)?;
        qualified.rsplit_once("::").map(|(module, _)| module)
    }

    fn classes_in(&self, artifact: &str) -> Vec<(String, GraphNodeId)> {
        self.classes_by_artifact
            .get(artifact)
            .cloned()
            .unwrap_or_default()
    }

    /// Finds `method` on `class`, then on its bases in declaration order --
    /// the order both languages resolve an inherited method in. Visited
    /// classes are tracked so an inheritance cycle terminates.
    fn method(&self, class: &GraphNodeId, method: &str) -> Option<GraphNodeId> {
        let mut queue = VecDeque::from([class.clone()]);
        let mut visited = BTreeSet::new();
        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            let qualified = self.qualified_name_by_id.get(&current)?;
            if let Some(target) = self
                .methods_by_qualified_name
                .get(&format!("{qualified}::{method}"))
            {
                return Some(target.clone());
            }
            queue.extend(self.bases.get(&current).into_iter().flatten().cloned());
        }
        None
    }

    fn add_base(&mut self, child: GraphNodeId, parent: GraphNodeId) {
        let bases = self.bases.entry(child).or_default();
        if !bases.contains(&parent) {
            bases.push(parent);
        }
    }
}

/// Keeps only names with exactly one declaration. A name two classes share
/// cannot type anything, and picking either is the fabrication this pass is
/// meant to remove.
fn singletons(
    candidates: BTreeMap<String, BTreeSet<GraphNodeId>>,
) -> BTreeMap<String, GraphNodeId> {
    candidates
        .into_iter()
        .filter_map(|(name, ids)| {
            (ids.len() == 1).then(|| Some((name, ids.into_iter().next()?)))?
        })
        .collect()
}

/// Orders artifacts so a file is visited after the files it imports.
///
/// Import cycles have no topological order at all, so the remaining files are
/// released in repository-path order once no file is free of dependencies.
/// Both that tie-break and the queue are driven by sorted keys, making the
/// result a pure function of the facts -- a cycle changes which environment a
/// file sees, but never makes it depend on iteration order (AC3).
fn visit_order(facts: &TypeFacts) -> Vec<&str> {
    let module_owner: BTreeMap<&str, &str> = facts
        .iter()
        .map(|(artifact, file)| (file.module.as_str(), artifact.as_str()))
        .collect();

    // Edge direction: imported file -> importing file, so a file is released
    // only once everything it imports has been.
    let mut dependents: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    let mut remaining: BTreeMap<&str, usize> = BTreeMap::new();
    for (artifact, file) in facts {
        let imported: BTreeSet<&str> = file
            .imports
            .iter()
            .flat_map(|import| import.modules.iter())
            .filter_map(|module| module_owner.get(module.as_str()).copied())
            .filter(|imported| *imported != artifact.as_str())
            .collect();
        remaining.insert(artifact.as_str(), imported.len());
        for source in imported {
            dependents.entry(source).or_default().insert(artifact);
        }
    }

    let mut order = Vec::with_capacity(facts.len());
    let mut ready: BTreeSet<&str> = remaining
        .iter()
        .filter(|(_, count)| **count == 0)
        .map(|(artifact, _)| *artifact)
        .collect();

    while order.len() < facts.len() {
        // `ready` is a sorted set, so equally-ready files are always visited
        // in the same order.
        let next = match ready.iter().next().copied() {
            Some(next) => next,
            // Everything left is in a cycle. Releasing the smallest path is
            // arbitrary but fixed, which is what determinism requires.
            None => match remaining
                .iter()
                .filter(|(artifact, _)| !order.contains(*artifact))
                .map(|(artifact, _)| *artifact)
                .next()
            {
                Some(next) => next,
                None => break,
            },
        };
        ready.remove(next);
        remaining.remove(next);
        order.push(next);

        for dependent in dependents.get(next).into_iter().flatten() {
            if let Some(count) = remaining.get_mut(dependent) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.insert(dependent);
                }
            }
        }
    }

    order
}

fn call_relation(
    artifact: &str,
    target: &GraphNodeId,
    call: &MemberCallFact,
    language: &str,
    call_index: usize,
) -> Relation {
    let source = GraphNodeId::new(format!("artifact:{artifact}"));
    Relation {
        id: format!(
            "relation:{}:{}:{}:{}:{}",
            PROPAGATE_STRATEGY,
            artifact,
            target.as_str(),
            call.evidence
                .span
                .as_ref()
                .map_or(0, |span| span.start_line),
            call_index,
        ),
        source,
        target: target.clone(),
        kind: RelationKind::Calls,
        // The receiver is proven, not guessed: it is typed by a construction
        // or an import this file actually contains.
        confidence: Confidence::High,
        evidence: vec![call.evidence.clone()],
        provenance: Some(RelationProvenance {
            language: Some(language.to_owned()),
            resolver_strategy: PROPAGATE_STRATEGY.to_owned(),
            resolution: RelationResolution::HybridResolved,
            confidence: Confidence::High,
        }),
    }
}

fn inheritance_relation(
    artifact: &str,
    child: &GraphNodeId,
    parent: &GraphNodeId,
    base: &BaseClassFact,
    language: &str,
) -> Relation {
    Relation {
        id: format!(
            "relation:{PROPAGATE_STRATEGY}:inherits:{artifact}:{}:{}",
            child.as_str(),
            parent.as_str(),
        ),
        source: child.clone(),
        target: parent.clone(),
        kind: RelationKind::Inherits,
        confidence: Confidence::High,
        evidence: vec![base.evidence.clone()],
        provenance: Some(RelationProvenance {
            language: Some(language.to_owned()),
            resolver_strategy: PROPAGATE_STRATEGY.to_owned(),
            resolution: RelationResolution::HybridResolved,
            confidence: Confidence::High,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BindingFact, FileTypeFacts, ImportBindingFact, MemberCallFact, Receiver, TypeFacts,
        propagate_types, visit_order,
    };
    use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath};
    use crate::graph::{
        Graph, GraphNode, GraphNodeId, Relation, RelationKind, RelationProvenance,
        RelationResolution, SymbolKind, SymbolNode,
    };

    fn evidence(path: &str) -> EvidenceRef {
        let path = RepoPath::new(path).unwrap_or_else(|_| unreachable!());
        EvidenceRef::file(ArtifactId::from_path(&path), path)
    }

    /// A class or method symbol, qualified exactly as the builders qualify
    /// them: `{module}::{Class}` and `{module}::{Class}::{method}`.
    fn symbol(path: &str, qualified: &str, kind: SymbolKind) -> GraphNode {
        GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(format!("symbol:{path}#{qualified}")),
            kind,
            qualified_name: qualified.to_owned(),
            doc: None,
            evidence: evidence(path),
        })
    }

    fn inherits(path: &str, child: &str, base: &str) -> Relation {
        Relation {
            id: format!("relation:inherits:{child}"),
            source: GraphNodeId::new(format!("symbol:{path}#{child}")),
            target: GraphNodeId::new(format!("symbol:{path}#{base}")),
            kind: RelationKind::Inherits,
            confidence: Confidence::High,
            evidence: vec![evidence(path)],
            provenance: Some(RelationProvenance {
                language: Some("python".to_owned()),
                resolver_strategy: "syntax-extraction".to_owned(),
                resolution: RelationResolution::SyntaxOnly,
                confidence: Confidence::High,
            }),
        }
    }

    fn member_call(receiver: Receiver, method: &str, class: Option<&str>) -> MemberCallFact {
        MemberCallFact {
            receiver,
            method: method.to_owned(),
            enclosing_class: class.map(str::to_owned),
            evidence: evidence("src/app.py"),
        }
    }

    /// Targets of every `Calls` relation this pass appended.
    fn resolved_calls(graph: &Graph) -> Vec<&str> {
        graph
            .relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::Calls)
            .map(|relation| relation.target.as_str())
            .collect()
    }

    /// AC2: the receiver's type comes from an imported symbol, across files.
    #[test]
    fn receiver_typed_by_an_imported_class_resolves_across_files() {
        let mut graph = Graph {
            nodes: vec![
                symbol(
                    "src/provider.py",
                    "src.provider::Provider",
                    SymbolKind::Class,
                ),
                symbol(
                    "src/provider.py",
                    "src.provider::Provider::dumps",
                    SymbolKind::Method,
                ),
            ],
            relations: Vec::new(),
        };
        let facts = TypeFacts::from([(
            "src/app.py".to_owned(),
            FileTypeFacts {
                module: "src.app".to_owned(),
                language: "python".to_owned(),
                imports: vec![ImportBindingFact {
                    local: "Provider".to_owned(),
                    modules: vec!["src.provider".to_owned()],
                    symbol: "Provider".to_owned(),
                }],
                bindings: vec![BindingFact {
                    name: "provider".to_owned(),
                    constructor: "Provider".to_owned(),
                    is_module_level: true,
                }],
                member_calls: vec![member_call(
                    Receiver::Named("provider".to_owned()),
                    "dumps",
                    None,
                )],
                ..FileTypeFacts::default()
            },
        )]);

        let report = propagate_types(&mut graph, &facts);

        assert_eq!(report.resolved, 1);
        assert_eq!(report.untyped, 0);
        assert_eq!(
            resolved_calls(&graph),
            vec!["symbol:src/provider.py#src.provider::Provider::dumps"],
        );
    }

    /// The case that needs topological order specifically: `app.py` imports a
    /// name that `middle.py` bound to a construction, so `middle.py` must be
    /// typed first. Ordering by artifact path alone would visit app.py first
    /// and lose the edge.
    #[test]
    fn module_level_binding_propagates_to_an_importing_file() {
        let mut graph = Graph {
            nodes: vec![
                symbol(
                    "src/provider.py",
                    "src.provider::Provider",
                    SymbolKind::Class,
                ),
                symbol(
                    "src/provider.py",
                    "src.provider::Provider::dumps",
                    SymbolKind::Method,
                ),
            ],
            relations: Vec::new(),
        };
        let facts = TypeFacts::from([
            (
                // Sorts before `middle.py`, so a naive walk visits it first.
                "src/app.py".to_owned(),
                FileTypeFacts {
                    module: "src.app".to_owned(),
                    language: "python".to_owned(),
                    imports: vec![ImportBindingFact {
                        local: "shared".to_owned(),
                        modules: vec!["src.middle".to_owned()],
                        symbol: "shared".to_owned(),
                    }],
                    bindings: Vec::new(),
                    member_calls: vec![member_call(
                        Receiver::Named("shared".to_owned()),
                        "dumps",
                        None,
                    )],
                    ..FileTypeFacts::default()
                },
            ),
            (
                "src/middle.py".to_owned(),
                FileTypeFacts {
                    module: "src.middle".to_owned(),
                    language: "python".to_owned(),
                    imports: vec![ImportBindingFact {
                        local: "Provider".to_owned(),
                        modules: vec!["src.provider".to_owned()],
                        symbol: "Provider".to_owned(),
                    }],
                    bindings: vec![BindingFact {
                        name: "shared".to_owned(),
                        constructor: "Provider".to_owned(),
                        is_module_level: true,
                    }],
                    member_calls: Vec::new(),
                    ..FileTypeFacts::default()
                },
            ),
        ]);

        assert_eq!(
            visit_order(&facts),
            vec!["src/middle.py", "src/app.py"],
            "the imported file must be visited before the file importing it",
        );
        let report = propagate_types(&mut graph, &facts);

        assert_eq!(report.resolved, 1);
        assert_eq!(
            resolved_calls(&graph),
            vec!["symbol:src/provider.py#src.provider::Provider::dumps"],
        );
    }

    /// AC2 (self/this) and inheritance: `self.method()` resolves within the
    /// enclosing class, then through a declared base.
    #[test]
    fn enclosing_receiver_resolves_in_the_class_and_through_its_bases() {
        let mut graph = Graph {
            nodes: vec![
                symbol("src/app.py", "src.app::Base", SymbolKind::Class),
                symbol("src/app.py", "src.app::Base::inherited", SymbolKind::Method),
                symbol("src/app.py", "src.app::Child", SymbolKind::Class),
                symbol("src/app.py", "src.app::Child::own", SymbolKind::Method),
            ],
            relations: vec![inherits("src/app.py", "src.app::Child", "src.app::Base")],
        };
        let facts = TypeFacts::from([(
            "src/app.py".to_owned(),
            FileTypeFacts {
                module: "src.app".to_owned(),
                language: "python".to_owned(),
                imports: Vec::new(),
                bindings: Vec::new(),
                member_calls: vec![
                    member_call(Receiver::Enclosing, "own", Some("Child")),
                    member_call(Receiver::Enclosing, "inherited", Some("Child")),
                ],
                ..FileTypeFacts::default()
            },
        )]);

        let report = propagate_types(&mut graph, &facts);

        assert_eq!(report.resolved, 2);
        assert_eq!(
            resolved_calls(&graph),
            vec![
                "symbol:src/app.py#src.app::Child::own",
                "symbol:src/app.py#src.app::Base::inherited",
            ],
        );
    }

    /// AC4: the whole point of the pass. A method name shared by two
    /// unrelated classes must not resolve just because the name exists, and
    /// an untyped receiver must produce nothing at all.
    #[test]
    fn ambiguous_and_untyped_receivers_create_no_edge() {
        let mut graph = Graph {
            nodes: vec![
                symbol("src/a.py", "src.a::Alpha", SymbolKind::Class),
                symbol("src/a.py", "src.a::Alpha::run", SymbolKind::Method),
                symbol("src/b.py", "src.b::Beta", SymbolKind::Class),
                symbol("src/b.py", "src.b::Beta::run", SymbolKind::Method),
            ],
            relations: Vec::new(),
        };
        let facts = TypeFacts::from([(
            "src/app.py".to_owned(),
            FileTypeFacts {
                module: "src.app".to_owned(),
                language: "python".to_owned(),
                imports: Vec::new(),
                bindings: Vec::new(),
                member_calls: vec![
                    // Never bound, never imported: nothing types it, even
                    // though exactly one `dangling` method could be searched
                    // for by name.
                    member_call(Receiver::Named("mystery".to_owned()), "run", None),
                    // `self` outside any class has no enclosing type.
                    member_call(Receiver::Enclosing, "run", None),
                    // Names a class that does not exist in this module.
                    member_call(Receiver::Enclosing, "run", Some("Missing")),
                ],
                ..FileTypeFacts::default()
            },
        )]);

        let report = propagate_types(&mut graph, &facts);

        assert_eq!(report.resolved, 0);
        assert_eq!(report.untyped, 3);
        assert!(
            resolved_calls(&graph).is_empty(),
            "an untyped receiver must not resolve by method-name match",
        );
    }

    /// AC3: a cycle has no topological order, so the tie-break must make the
    /// result depend only on the facts. Both files are always emitted, and
    /// the same one is always released first regardless of insertion order.
    #[test]
    fn import_cycles_terminate_in_a_fact_determined_order() {
        let cycle = |first: &str, second: &str| {
            TypeFacts::from([
                (
                    first.to_owned(),
                    FileTypeFacts {
                        module: "src.left".to_owned(),
                        imports: vec![ImportBindingFact {
                            local: "Right".to_owned(),
                            modules: vec!["src.right".to_owned()],
                            symbol: "Right".to_owned(),
                        }],
                        ..FileTypeFacts::default()
                    },
                ),
                (
                    second.to_owned(),
                    FileTypeFacts {
                        module: "src.right".to_owned(),
                        imports: vec![ImportBindingFact {
                            local: "Left".to_owned(),
                            modules: vec!["src.left".to_owned()],
                            symbol: "Left".to_owned(),
                        }],
                        ..FileTypeFacts::default()
                    },
                ),
            ])
        };

        let facts = cycle("src/left.py", "src/right.py");
        let order = visit_order(&facts);

        assert_eq!(
            order,
            vec!["src/left.py", "src/right.py"],
            "a cycle must still visit every file, in repository-path order",
        );
        let repeated = cycle("src/left.py", "src/right.py");
        assert_eq!(
            visit_order(&repeated),
            order,
            "the order must be a pure function of the facts",
        );
    }

    /// A file-level environment has no answer for a name each test case
    /// rebinds to a different class, so it must decline to type it.
    ///
    /// Found on the pinned NestJS corpus, where taking the last binding
    /// resolved `logger.error()` to `Logger::error` for a receiver built as
    /// `ConsoleLogger` -- two classes that merely share an interface. Twenty
    /// edges said the receiver held a class it never held.
    #[test]
    fn a_name_bound_to_two_classes_in_one_file_is_not_typed() {
        let mut graph = Graph {
            nodes: vec![
                symbol("src/log.py", "src.log::ConsoleLogger", SymbolKind::Class),
                symbol(
                    "src/log.py",
                    "src.log::ConsoleLogger::error",
                    SymbolKind::Method,
                ),
                symbol("src/log.py", "src.log::Logger", SymbolKind::Class),
                symbol("src/log.py", "src.log::Logger::error", SymbolKind::Method),
                symbol("src/log.py", "src.log::Only", SymbolKind::Class),
                symbol("src/log.py", "src.log::Only::run", SymbolKind::Method),
            ],
            relations: Vec::new(),
        };
        let facts = TypeFacts::from([(
            "src/log.py".to_owned(),
            FileTypeFacts {
                module: "src.log".to_owned(),
                language: "python".to_owned(),
                bindings: vec![
                    BindingFact {
                        name: "logger".to_owned(),
                        constructor: "ConsoleLogger".to_owned(),
                        is_module_level: false,
                    },
                    // The same local name, a different class, another scope.
                    BindingFact {
                        name: "logger".to_owned(),
                        constructor: "Logger".to_owned(),
                        is_module_level: false,
                    },
                    // A name bound only once stays typable.
                    BindingFact {
                        name: "single".to_owned(),
                        constructor: "Only".to_owned(),
                        is_module_level: false,
                    },
                    // Bound once to a visible class and once to one declared
                    // inside a function body, which is not a graph symbol. The
                    // invisible class may override `run`, so the visible one
                    // is not a safe answer either.
                    BindingFact {
                        name: "shadowed".to_owned(),
                        constructor: "Only".to_owned(),
                        is_module_level: false,
                    },
                    BindingFact {
                        name: "shadowed".to_owned(),
                        constructor: "DeclaredInsideAFunction".to_owned(),
                        is_module_level: false,
                    },
                ],
                member_calls: vec![
                    member_call(Receiver::Named("logger".to_owned()), "error", None),
                    member_call(Receiver::Named("single".to_owned()), "run", None),
                    member_call(Receiver::Named("shadowed".to_owned()), "run", None),
                ],
                ..FileTypeFacts::default()
            },
        )]);

        propagate_types(&mut graph, &facts);

        assert_eq!(
            resolved_calls(&graph),
            vec!["symbol:src/log.py#src.log::Only::run"],
            "`logger` holds two unrelated classes and `shadowed` holds one the graph \
             cannot see, so neither may claim its call; only the singly-bound name resolves",
        );
    }

    /// A class declared in the file wins over an import of the same name,
    /// and a self-import never deadlocks the walk.
    #[test]
    fn a_local_class_shadows_an_imported_name_of_the_same_name() {
        let mut graph = Graph {
            nodes: vec![
                symbol("src/app.py", "src.app::Provider", SymbolKind::Class),
                symbol("src/app.py", "src.app::Provider::local", SymbolKind::Method),
                symbol("src/other.py", "src.other::Provider", SymbolKind::Class),
                symbol(
                    "src/other.py",
                    "src.other::Provider::local",
                    SymbolKind::Method,
                ),
            ],
            relations: Vec::new(),
        };
        let facts = TypeFacts::from([(
            "src/app.py".to_owned(),
            FileTypeFacts {
                module: "src.app".to_owned(),
                language: "python".to_owned(),
                imports: vec![ImportBindingFact {
                    local: "Provider".to_owned(),
                    modules: vec!["src.other".to_owned()],
                    symbol: "Provider".to_owned(),
                }],
                bindings: vec![BindingFact {
                    name: "provider".to_owned(),
                    constructor: "Provider".to_owned(),
                    is_module_level: true,
                }],
                member_calls: vec![member_call(
                    Receiver::Named("provider".to_owned()),
                    "local",
                    None,
                )],
                ..FileTypeFacts::default()
            },
        )]);

        propagate_types(&mut graph, &facts);

        assert_eq!(
            resolved_calls(&graph),
            vec!["symbol:src/app.py#src.app::Provider::local"],
        );
    }
}
