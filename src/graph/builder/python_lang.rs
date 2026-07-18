use super::*;

impl BuilderState {
    pub(super) fn process_python(
        &mut self,
        artifact: &Artifact,
        analysis: PythonAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        let module_id = self
            .python_modules
            .get(&analysis.module_path)
            .cloned()
            .unwrap_or_else(|| {
                self.module(
                    &analysis.module_path,
                    ModuleLanguage::Python,
                    file_evidence(artifact),
                )
            });
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        // LIT-44.1: this file's own bound-name -> origin-module map (`from
        // pydantic import BaseModel` binds `BaseModel` to `pydantic`, `import
        // fastapi as fa` binds `fa` to `fastapi`), built up front so base
        // classes/decorators/return types/calls below can classify a
        // reference through the same file's own imports rather than always
        // falling to `Unresolved`. Deliberately same-file only: no attempt to
        // trace which module a local variable like `app = FastAPI()` came
        // from, so `@app.get(...)` stays `Unresolved` exactly as before.
        let mut imported_modules: BTreeMap<String, PythonImportBinding> = BTreeMap::new();
        for import in &analysis.imports {
            match import.kind {
                PythonImportKind::Import => {
                    for name in &import.names {
                        let bound = name.alias.clone().unwrap_or_else(|| {
                            name.name.split('.').next().unwrap_or(&name.name).to_owned()
                        });
                        imported_modules.insert(
                            bound,
                            PythonImportBinding {
                                module: name.name.clone(),
                                symbol: None,
                            },
                        );
                    }
                }
                PythonImportKind::ImportFrom => {
                    if import.relative_level == 0
                        && let Some(module) = &import.module
                    {
                        for name in &import.names {
                            let bound = name.alias.clone().unwrap_or_else(|| name.name.clone());
                            imported_modules.insert(
                                bound,
                                PythonImportBinding {
                                    module: module.clone(),
                                    symbol: Some(name.name.clone()),
                                },
                            );
                        }
                    }
                }
            }
        }

        let mut symbol_ids: BTreeMap<String, GraphNodeId> = BTreeMap::new();
        let mut callable_ids: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();
        // LIT-46: spans of every symbol here, so a rationale comment attaches
        // to the class, method, or function it sits inside.
        let mut symbol_spans: Vec<super::rationale::SymbolSpan> = Vec::new();

        for class in &analysis.classes {
            let qualified = format!("{}::{}", analysis.module_path, class.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Class,
                qualified_name: qualified,
                doc: class.docstring.clone(),
                evidence: class.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![class.evidence.clone()],
            );
            symbol_ids.insert(class.name.clone(), id.clone());
            if let Some(span) = class.evidence.span.clone() {
                symbol_spans.push(super::rationale::SymbolSpan {
                    id: id.clone(),
                    span,
                });
            }
            callable_ids
                .entry(class.name.clone())
                .or_default()
                .push(id.clone());
            for decorator in &class.decorators {
                let target = self
                    .same_file_import_external_symbol(&imported_modules, decorator, &class.evidence)
                    .unwrap_or_else(|| self.unresolved(decorator));
                self.relate_with_provenance(
                    id.clone(),
                    target,
                    RelationKind::Decorates,
                    Confidence::Low,
                    vec![class.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }

            for method in &class.methods {
                let method_qualified =
                    format!("{}::{}::{}", analysis.module_path, class.name, method.name);
                let method_id = self.insert(GraphNode::Symbol(SymbolNode {
                    id: GraphNodeId::new(format!("symbol:{}#{method_qualified}", artifact.path)),
                    kind: SymbolKind::Method,
                    qualified_name: method_qualified,
                    doc: method.docstring.clone(),
                    evidence: method.evidence.clone(),
                }));
                self.relate(
                    id.clone(),
                    method_id.clone(),
                    RelationKind::Contains,
                    Confidence::High,
                    vec![method.evidence.clone()],
                );
                self.relate_with_provenance(
                    id.clone(),
                    method_id.clone(),
                    RelationKind::HasMethod,
                    Confidence::High,
                    vec![method.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::High,
                    )),
                );
                self.relate_with_provenance(
                    method_id.clone(),
                    id.clone(),
                    RelationKind::MemberOf,
                    Confidence::High,
                    vec![method.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::High,
                    )),
                );
                if let Some(return_type) = &method.return_type {
                    let target = self
                        .same_file_import_external_symbol(
                            &imported_modules,
                            return_type,
                            &method.evidence,
                        )
                        .unwrap_or_else(|| self.unresolved(return_type));
                    self.relate_with_provenance(
                        method_id.clone(),
                        target,
                        RelationKind::UsesType,
                        Confidence::Low,
                        vec![method.evidence.clone()],
                        Some(format_provenance(
                            "python",
                            RelationResolution::SyntaxOnly,
                            Confidence::Low,
                        )),
                    );
                }
                if let Some(span) = method.evidence.span.clone() {
                    symbol_spans.push(super::rationale::SymbolSpan {
                        id: method_id.clone(),
                        span,
                    });
                }
                callable_ids
                    .entry(method.name.clone())
                    .or_default()
                    .push(method_id);
            }
        }

        for function in &analysis.functions {
            let qualified = format!("{}::{}", analysis.module_path, function.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Function,
                qualified_name: qualified,
                doc: function.docstring.clone(),
                evidence: function.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
            );
            self.process_python_route_decorators(artifact, artifact_node, function, &id);
            for decorator in &function.decorators {
                // Route decorators already have a typed Route node and a
                // HandlesRoute edge. Emitting the raw invocation again as a
                // low-confidence Unresolved Decorates target duplicates the
                // fact and makes a valid route look like resolver failure.
                if python::parse_route_decorator(decorator).is_some() {
                    continue;
                }
                let target = self
                    .same_file_import_external_symbol(
                        &imported_modules,
                        decorator,
                        &function.evidence,
                    )
                    .unwrap_or_else(|| self.unresolved(decorator));
                self.relate_with_provenance(
                    id.clone(),
                    target,
                    RelationKind::Decorates,
                    Confidence::Low,
                    vec![function.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
            if let Some(return_type) = &function.return_type {
                let target = self
                    .same_file_import_external_symbol(
                        &imported_modules,
                        return_type,
                        &function.evidence,
                    )
                    .unwrap_or_else(|| self.unresolved(return_type));
                self.relate_with_provenance(
                    id.clone(),
                    target,
                    RelationKind::UsesType,
                    Confidence::Low,
                    vec![function.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
            symbol_ids.insert(function.name.clone(), id.clone());
            if let Some(span) = function.evidence.span.clone() {
                symbol_spans.push(super::rationale::SymbolSpan {
                    id: id.clone(),
                    span,
                });
            }
            callable_ids
                .entry(function.name.clone())
                .or_default()
                .push(id);
        }

        for import in &analysis.imports {
            self.process_python_import(
                artifact,
                artifact_node,
                &analysis.module_path,
                analysis.is_package_init,
                import,
            );
        }

        self.process_rationale(artifact, artifact_node, &analysis.comments, &symbol_spans);

        self.record_python_type_facts(artifact, &analysis);

        for reference in &analysis.references {
            self.process_python_reference(
                artifact,
                artifact_node,
                reference,
                &symbol_ids,
                &callable_ids,
                &imported_modules,
            );
        }

        // Base classes (LIT-22.3.3): resolves to a same-file class by bare
        // name, or (LIT-44.1) to a known stdlib/manifest-declared package
        // when the base was itself imported directly in this file (e.g.
        // `from pydantic import BaseModel`) -- anything else (a cross-module
        // base from an intra-repo module, or one this file doesn't import
        // under that exact name) stays `Unresolved` rather than guessing.
        for class in &analysis.classes {
            let Some(class_id) = symbol_ids.get(&class.name) else {
                continue;
            };
            for base in &class.bases {
                let base_name = base.rsplit('.').next().unwrap_or(base.as_str());
                let target = symbol_ids
                    .get(base_name)
                    .cloned()
                    .or_else(|| {
                        self.same_file_import_external_symbol(
                            &imported_modules,
                            base,
                            &class.evidence,
                        )
                    })
                    .unwrap_or_else(|| self.unresolved(base));
                self.relate_with_provenance(
                    class_id.clone(),
                    target,
                    RelationKind::Inherits,
                    Confidence::Low,
                    vec![class.evidence.clone()],
                    Some(format_provenance(
                        "python",
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
        }
    }
    fn process_python_import(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        module_path: &str,
        is_package_init: bool,
        import: &crate::analysis::PythonImport,
    ) {
        match import.kind {
            PythonImportKind::Import => {
                for name in &import.names {
                    let target = self
                        .python_modules
                        .get(&name.name)
                        .or_else(|| self.python_package_relative_modules.get(&name.name))
                        .cloned()
                        .unwrap_or_else(|| self.python_external_target(&name.name));
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::Imports,
                        Confidence::High,
                        vec![import.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::HybridResolved,
                            Confidence::High,
                        )),
                    );
                }
            }
            PythonImportKind::ImportFrom => {
                let resolved = python_relative_target(
                    module_path,
                    is_package_init,
                    import.relative_level,
                    import.module.as_deref(),
                );
                let target = resolved
                    .as_ref()
                    .and_then(|resolved| {
                        // LIT-76: an absolute import of the repo's own package
                        // (`from app.core.config import ...`) matches by
                        // package-relative path when the whole-repo key misses.
                        self.python_modules
                            .get(resolved)
                            .or_else(|| self.python_package_relative_modules.get(resolved))
                            .cloned()
                    })
                    .unwrap_or_else(|| match (import.relative_level, &resolved) {
                        // Only an absolute (non-relative) import can ever name
                        // a stdlib module, so relative imports always fall
                        // through to the marker/unresolved path below.
                        (0, Some(module)) => self.python_external_target(module),
                        _ => {
                            let marker = resolved.clone().unwrap_or_else(|| {
                                format!(
                                    "{}{}",
                                    ".".repeat(import.relative_level as usize),
                                    import.module.clone().unwrap_or_default()
                                )
                            });
                            self.unresolved(&marker)
                        }
                    });
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Imports,
                    Confidence::High,
                    vec![import.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::HybridResolved,
                        Confidence::High,
                    )),
                );
            }
        }
    }
    /// Turns a module-level function's decorators that look like an HTTP
    /// route registration into a first-class `Route` config node
    /// (LIT-22.3.4 AC1). Class-based views/routers aren't covered -- most
    /// Flask/FastAPI routes are plain module-level functions, and guessing
    /// at class-method route registration without more evidence would risk
    /// false positives.
    fn process_python_route_decorators(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        function: &crate::analysis::PythonFunction,
        handler: &GraphNodeId,
    ) {
        for (index, decorator) in function.decorators.iter().enumerate() {
            let Some((method, path)) = python::parse_route_decorator(decorator) else {
                continue;
            };
            let key = format!("route.{}.{index}", function.name);
            let route_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Route,
                &format!("{method} {path}"),
                function.evidence.clone(),
            );
            self.relate_with_provenance(
                artifact_node.clone(),
                route_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
                Some(artifact_provenance(
                    artifact,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            self.relate_with_provenance(
                route_id,
                handler.clone(),
                RelationKind::HandlesRoute,
                Confidence::High,
                vec![function.evidence.clone()],
                Some(format_provenance(
                    "python",
                    RelationResolution::HybridResolved,
                    Confidence::High,
                )),
            );
        }
    }
    /// LIT-57: hands this file's receiver-typing facts to the cross-file
    /// propagation pass, which runs once every file's symbols exist.
    ///
    /// Only `from x import y` bindings are forwarded. `import x` binds a
    /// module, and a module member call (`x.f()`) is an import fact that
    /// LIT-44.1 already classifies -- it is not a receiver to type.
    ///
    /// These bindings are built from the raw imports rather than reused from
    /// `imported_modules`, which drops relative imports because LIT-44.1 only
    /// cares about external packages. The opposite is true here: a package
    /// imports its own modules relatively (`from .json.tag import ...`), so
    /// skipping those would leave the environment empty for exactly the
    /// intra-repository imports this pass exists to follow.
    fn record_python_type_facts(&mut self, artifact: &Artifact, analysis: &PythonAnalysis) {
        // Runtime imports and `if TYPE_CHECKING:` imports bind names the same
        // way for annotation purposes; the latter exist for nothing else.
        let imports = analysis
            .imports
            .iter()
            .chain(&analysis.typing_imports)
            .filter(|import| import.kind == PythonImportKind::ImportFrom)
            .filter_map(|import| {
                let module = absolute_python_module(
                    &analysis.module_path,
                    analysis.is_package_init,
                    import.relative_level,
                    import.module.as_deref(),
                )?;
                Some(
                    import
                        .names
                        .iter()
                        .map(move |name| crate::resolve::ImportBindingFact {
                            local: name.alias.clone().unwrap_or_else(|| name.name.clone()),
                            modules: vec![module.clone()],
                            symbol: name.name.clone(),
                        }),
                )
            })
            .flatten()
            .collect();
        let bindings = analysis
            .bindings
            .iter()
            .map(|binding| crate::resolve::BindingFact {
                name: binding.name.clone(),
                constructor: binding.constructor.clone(),
                is_module_level: binding.is_module_level,
            })
            .collect();
        let member_calls = analysis
            .member_calls
            .iter()
            .map(|call| crate::resolve::MemberCallFact {
                // `self`/`cls` are conventions rather than keywords, so a
                // method that names its receiver parameter anything else is
                // not typed here. Every such method in the pinned corpora
                // follows the convention, and inferring the first parameter's
                // role instead would type receivers this pass cannot check.
                receiver: match call.receiver.as_str() {
                    "self" | "cls" => crate::resolve::Receiver::Enclosing,
                    name => crate::resolve::Receiver::Named(name.to_owned()),
                },
                method: call.method.clone(),
                enclosing_class: call.enclosing_class.clone(),
                evidence: call.evidence.clone(),
            })
            .collect();

        self.type_facts.insert(
            artifact.path.as_str().to_owned(),
            crate::resolve::FileTypeFacts {
                module: analysis.module_path.clone(),
                language: "python".to_owned(),
                imports,
                bindings,
                bases: Vec::new(),
                member_calls,
                // Python has no barrel re-exports: `__init__.py` republishes
                // by importing, which the import bindings above already carry.
                re_exports: Vec::new(),
            },
        );
    }

    fn process_python_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::PythonReference,
        symbol_ids: &BTreeMap<String, GraphNodeId>,
        callable_ids: &BTreeMap<String, Vec<GraphNodeId>>,
        imported_modules: &BTreeMap<String, PythonImportBinding>,
    ) {
        match reference.kind {
            PythonReferenceKind::Call => {
                let simple = reference
                    .value
                    .rsplit('.')
                    .next()
                    .unwrap_or(&reference.value);
                if let Some([target]) = callable_ids.get(simple).map(Vec::as_slice) {
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target.clone(),
                        RelationKind::Calls,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::HybridResolved,
                            reference.confidence,
                        )),
                    );
                } else {
                    // LIT-44.1: a bare call to a name this file imported
                    // directly (`Depends(...)` after `from fastapi import
                    // Depends`) resolves to the known stdlib/manifest package
                    // member instead of `Unresolved`; anything else is
                    // unchanged. LIT-56: the target is the member's own
                    // symbol, since `Calls` may not point at a package.
                    let target = self
                        .same_file_import_external_symbol(
                            imported_modules,
                            &reference.value,
                            &reference.evidence,
                        )
                        .unwrap_or_else(|| self.unresolved(&reference.value));
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::Calls,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::SyntaxOnly,
                            reference.confidence,
                        )),
                    );
                }
            }
            PythonReferenceKind::EnvRead => {
                let target = self.env_var(&reference.value);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::ReadsEnv,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            PythonReferenceKind::Subprocess => {
                let key = format!(
                    "{}",
                    reference
                        .evidence
                        .span
                        .as_ref()
                        .map(|span| span.start_line)
                        .unwrap_or(0)
                );
                let target =
                    self.command(artifact, &key, &reference.value, reference.evidence.clone());
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::RunsCommand,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            PythonReferenceKind::DynamicImport => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Imports,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::Fallback,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::Ctypes => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::Fallback,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::ConfigPath => {
                let (target, path_confidence) = self.reference_target(&reference.value);
                let confidence = reference.confidence.min(path_confidence);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::HybridResolved,
                        confidence,
                    )),
                );
            }
            PythonReferenceKind::HttpCall => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::References,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::SyntaxOnly,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::Emits | PythonReferenceKind::ListensOn => {
                let kind = if reference.kind == PythonReferenceKind::Emits {
                    RelationKind::Emits
                } else {
                    RelationKind::ListensOn
                };
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    kind,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(artifact_provenance(
                        artifact,
                        RelationResolution::SyntaxOnly,
                        reference.confidence,
                    )),
                );
            }
            PythonReferenceKind::DataFlows => {
                if let Some(target) = symbol_ids.get(&reference.value).cloned() {
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::DataFlows,
                        reference.confidence,
                        vec![reference.evidence.clone()],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::HybridResolved,
                            reference.confidence,
                        )),
                    );
                }
            }
        }
    }
    /// LIT-44.1: resolves `text` (a decorator, base class, return-type
    /// annotation, or bare call target, as written) through this file's own
    /// `imported_modules` map (built in [`Self::process_python`]) when its
    /// leading identifier was itself imported directly in this file. Returns
    /// `None` -- leaving the caller's existing `unresolved(text)` fallback
    /// unchanged -- when the leading identifier isn't an import in this file
    /// (e.g. a local variable like `app` in `app.get(...)`) or the import's
    /// origin module is neither stdlib nor a manifest-declared dependency.
    /// Resolves `text` to an external symbol node when the name it starts
    /// with was imported from a standard-library or manifest-declared
    /// package in this same file.
    ///
    /// LIT-56: this used to return the *package* node, which is an illegal
    /// target for every relation kind that calls it (`Calls`, `Decorates`,
    /// `UsesType` accept only `Symbol`), so real repositories failed
    /// validation. It now names the member itself.
    fn same_file_import_external_symbol(
        &mut self,
        imported_modules: &BTreeMap<String, PythonImportBinding>,
        text: &str,
        evidence: &EvidenceRef,
    ) -> Option<GraphNodeId> {
        let root = python_identifier_root(text);
        let binding = imported_modules.get(root)?;
        let top_level = binding
            .module
            .split('.')
            .next()
            .unwrap_or(binding.module.as_str());
        if !is_python_stdlib_module(&binding.module)
            && !self
                .python_manifest_packages
                .contains(&normalize_python_package_name(top_level))
        {
            return None;
        }
        let name = binding.symbol_name(text)?;
        Some(self.python_external_symbol(&binding.module, &name, evidence.clone()))
    }
}

/// One name a Python file's imports bind, and where it comes from.
///
/// Both halves matter for naming the external symbol (LIT-56): `symbol` keeps
/// the name as the *package* declares it, so `from pydantic import Field as F`
/// records `pydantic::Field` rather than the local alias `F`, and its absence
/// marks a plain `import x` binding, where the member is instead the segment
/// that follows the module in the reference text.
/// LIT-57: the absolute module a `from ... import ...` names, so an imported
/// symbol is an exact `{module}::{symbol}` lookup.
///
/// Python resolves a relative import against the importing file's *package*,
/// not its module: one leading dot means the current package, and each extra
/// dot climbs one level above it. A regular module's package is its path minus
/// its own name, while a package's `__init__.py` is already that package --
/// getting that distinction wrong shifts every relative import by one level.
///
/// Returns `None` when the import climbs above the repository root, which is a
/// module outside the scanned tree rather than one to guess at.
fn absolute_python_module(
    module_path: &str,
    is_package_init: bool,
    relative_level: u32,
    module: Option<&str>,
) -> Option<String> {
    if relative_level == 0 {
        return module.map(str::to_owned);
    }

    let mut package: Vec<&str> = module_path
        .split('.')
        .filter(|part| !part.is_empty())
        .collect();
    if !is_package_init {
        package.pop();
    }
    // The first dot names the package itself, so only the extra dots climb.
    for _ in 0..relative_level - 1 {
        package.pop()?;
    }

    let mut parts: Vec<&str> = package;
    parts.extend(module.into_iter().flat_map(|value| value.split('.')));
    let joined = parts.join(".");
    (!joined.is_empty()).then_some(joined)
}

#[derive(Debug, Clone)]
struct PythonImportBinding {
    /// Dotted module path the name resolves to.
    module: String,
    /// Name as declared by the module, for `from x import y` bindings.
    /// `None` for `import x`, which binds the module itself.
    symbol: Option<String>,
}

impl PythonImportBinding {
    /// The external member `text` refers to, if any.
    fn symbol_name(&self, text: &str) -> Option<String> {
        match &self.symbol {
            // `from multiprocessing import cpu_count` + `cpu_count()`: the
            // bound name is the member; any `.attr` suffix is access on it.
            Some(symbol) => Some(symbol.clone()),
            // `import pydantic` + `pydantic.BaseModel`: the member is the
            // segment after the module. A bare `pydantic` names no member.
            None => python_identifier_path(text).nth(1).map(str::to_owned),
        }
    }
}

/// The leading dotted identifier of a decorator/base-class/return-type/call
/// text, stopping at the first character that can't be part of one (a call's
/// `(`, a generic's `[`, ...), then reduced to its first segment: `app.get`
/// -> `app`, `Optional[Item]` -> `Optional`, `validator("field")` ->
/// `validator`, `pydantic.BaseModel` -> `pydantic`.
fn python_identifier_root(text: &str) -> &str {
    python_identifier_path(text).next().unwrap_or("")
}

/// The dotted segments of a decorator/base-class/return-type/call text:
/// `pydantic.BaseModel` -> [`pydantic`, `BaseModel`], `app.get("/")` ->
/// [`app`, `get`]. Shares [`python_identifier_root`]'s trimming rule so the
/// root and the segments after it can never disagree.
fn python_identifier_path(text: &str) -> impl Iterator<Item = &str> {
    text.find(|character: char| {
        !(character.is_alphanumeric() || character == '_' || character == '.')
    })
    .map_or(text, |end| &text[..end])
    .split('.')
    .filter(|segment| !segment.is_empty())
}

fn python_relative_target(
    module_path: &str,
    is_package_init: bool,
    level: u32,
    module: Option<&str>,
) -> Option<String> {
    if level == 0 {
        return module.map(str::to_owned);
    }
    let mut segments: Vec<&str> = module_path
        .split('.')
        .filter(|segment| !segment.is_empty())
        .collect();
    if !is_package_init && !segments.is_empty() {
        segments.pop();
    }
    for _ in 1..level {
        segments.pop();
    }
    let mut target = segments.join(".");
    if let Some(module) = module {
        if !target.is_empty() {
            target.push('.');
        }
        target.push_str(module);
    }
    if target.is_empty() {
        None
    } else {
        Some(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-57: a relative import resolves against the importing file's
    /// package, and a package's `__init__` *is* that package. An off-by-one
    /// here silently points every relative import at the wrong module, which
    /// yields no edge rather than a visible failure.
    #[test]
    fn relative_imports_resolve_against_the_importing_package() {
        // `src/flask/sessions.py` doing `from .json.tag import X`: one dot is
        // the package `src.flask`, not the module `src.flask.sessions`.
        assert_eq!(
            absolute_python_module("src.flask.sessions", false, 1, Some("json.tag")),
            Some("src.flask.json.tag".to_owned()),
        );
        // `src/flask/__init__.py` IS `src.flask`, so one dot stays there.
        assert_eq!(
            absolute_python_module("src.flask", true, 1, Some("app")),
            Some("src.flask.app".to_owned()),
        );
        // Each extra dot climbs one level above the package.
        assert_eq!(
            absolute_python_module("src.flask.json.tag", false, 2, Some("app")),
            Some("src.flask.app".to_owned()),
        );
        // `from . import x` has no module clause; the package itself is meant.
        assert_eq!(
            absolute_python_module("src.flask.sessions", false, 1, None),
            Some("src.flask".to_owned()),
        );
        // Absolute imports pass through untouched.
        assert_eq!(
            absolute_python_module("src.flask.app", false, 0, Some("os.path")),
            Some("os.path".to_owned()),
        );
        // Climbing above the root names a module outside the scanned tree.
        assert_eq!(
            absolute_python_module("app", false, 2, Some("x")),
            None,
            "an import that escapes the repository root must not be guessed at",
        );
    }

    #[test]
    fn python_cross_file_calls_resolve_to_symbols() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("worker.py"),
            "def exported():\n    return 1\n",
        )?;
        std::fs::write(
            repo.path().join("app.py"),
            "from worker import exported\n\ndef start():\n    return exported()\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);

        let target = "symbol:worker.py#worker::exported";
        assert!(
            graph.relations.iter().any(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.target.as_str() == target
                    && relation.provenance.as_ref().is_some_and(|provenance| {
                        provenance.resolution == RelationResolution::HybridResolved
                    })
            }),
            "missing resolved call target {target}"
        );
        Ok(())
    }

    #[test]
    fn graph_resolves_python_relative_import_and_same_file_call()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let service_module = "module:src.python_app.service";
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == service_module)
        );

        let resolved_relative_import = graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Imports
                && relation.source.as_str() == "artifact:src/python_app/__init__.py"
                && relation.target.as_str() == service_module
        });
        assert!(
            resolved_relative_import,
            "expected resolved relative import to service module"
        );

        let same_file_call = graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Calls
                && relation.source.as_str() == "artifact:src/python_app/service.py"
        });
        assert!(
            same_file_call,
            "expected a resolved same-file call relation"
        );

        Ok(())
    }

    /// LIT-76: a repo whose top-level package `app` lives under a source-root
    /// directory (`backend/`) imports its own modules by their installed name
    /// (`from app.core.config import ...`). That absolute import resolves to
    /// the local module, not an `Unresolved` node, while a genuinely
    /// third-party absolute import is left for external classification.
    #[test]
    fn python_intra_package_absolute_import_resolves_to_local_module()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        std::fs::create_dir_all(root.join("backend/app/core"))?;
        // `app` is a package (has __init__.py); `backend` is not, so it is
        // the source root and imports start at `app`.
        std::fs::write(root.join("backend/app/__init__.py"), "")?;
        std::fs::write(root.join("backend/app/core/__init__.py"), "")?;
        std::fs::write(root.join("backend/app/core/config.py"), "settings = 1\n")?;
        std::fs::write(
            root.join("backend/app/main.py"),
            "from app.core.config import settings\nimport requests\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let graph = GraphBuilder.build(root, &artifacts);

        // The intra-package import resolves to the local module node, whose
        // identity remains the whole-repo path.
        assert!(
            graph.relations.iter().any(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:backend/app/main.py"
                    && relation.target.as_str() == "module:backend.app.core.config"
            }),
            "intra-package `from app.core.config` should resolve to the local module"
        );
        // A third-party absolute import is unaffected: no local module named
        // `requests` exists, so it must not fabricate one.
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == "module:requests"),
            "a third-party import must not resolve to a local module"
        );
        Ok(())
    }

    /// LIT-22.3.3 AC1/AC3: a same-file base class resolves to the base
    /// class's own `Symbol` node; a base class defined elsewhere (no
    /// same-file evidence) stays `Unresolved` rather than being guessed.
    #[test]
    fn python_base_classes_produce_inherits_relations() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("models.py"),
            "class Base:\n    pass\n\n\nclass Derived(Base):\n    pass\n\n\nclass External(SomeImportedBase):\n    pass\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let inherits: Vec<&Relation> = graph
            .relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::Inherits)
            .collect();
        assert_eq!(inherits.len(), 2);
        let base_symbol_id = graph
            .nodes
            .iter()
            .find_map(|node| match node {
                GraphNode::Symbol(symbol) if symbol.qualified_name.ends_with("::Base") => {
                    Some(node.id())
                }
                _ => None,
            })
            .ok_or("missing Base symbol node")?;
        assert!(
            inherits
                .iter()
                .any(|relation| &relation.target == base_symbol_id)
        );
        assert!(inherits.iter().any(|relation| {
            graph
                .nodes
                .iter()
                .any(|node| node.id() == &relation.target
                    && matches!(node, GraphNode::Unresolved(unresolved) if unresolved.value == "SomeImportedBase"))
        }));

        Ok(())
    }

    /// LIT-44.1 AC1: a top-level import of a name declared as a dependency
    /// in `pyproject.toml` resolves to a shared external `Package` node
    /// instead of `Unresolved`; an import of a name this repo never
    /// declares (and that isn't stdlib) stays `Unresolved` exactly as
    /// before -- this is a classification split, not a blanket silencer.
    #[test]
    fn python_import_of_manifest_dependency_resolves_to_package_node()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("pyproject.toml"),
            "[project]\nname = \"demo\"\ndependencies = [\"fastapi>=0.100\"]\n",
        )?;
        std::fs::write(
            temp.path().join("main.py"),
            "from fastapi import FastAPI\nfrom some_undeclared_lib import Thing\n\napp = FastAPI()\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let fastapi_package = graph
            .nodes
            .iter()
            .find_map(|node| match node {
                GraphNode::Package(package) if package.name == "fastapi" => Some(node.id()),
                _ => None,
            })
            .ok_or("expected a package:fastapi node for the manifest-declared dependency")?;
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Imports
                    && &relation.target == fastapi_package)
        );

        assert!(
            graph.nodes.iter().any(|node| matches!(
                node,
                GraphNode::Unresolved(unresolved) if unresolved.value == "some_undeclared_lib"
            )),
            "expected the undeclared import to remain Unresolved"
        );

        Ok(())
    }

    /// LIT-44.1 AC2, as amended by LIT-56: a base class, decorator, and bare
    /// call imported directly in the same file (`from pydantic import
    /// BaseModel, Field, validate_call`) resolve through that file's own
    /// import to `pydantic` rather than staying `Unresolved`.
    ///
    /// They land on the imported *member's* symbol, not on `package:pydantic`.
    /// `Inherits`/`Decorates`/`Calls` accept only `Symbol` targets, so the
    /// original package target built graphs `GraphValidator` rejects, failing
    /// `init` on any repository that calls an imported name. The symbol is
    /// also the more precise answer: `pydantic::Field`, not all of pydantic.
    #[test]
    fn python_same_file_import_resolves_base_class_decorator_and_call()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(temp.path().join("requirements.txt"), "pydantic==2.5.0\n")?;
        std::fs::write(
            temp.path().join("models.py"),
            concat!(
                "from pydantic import BaseModel, Field, validate_call\n\n\n",
                "class Item(BaseModel):\n",
                "    value: int = Field(default=0)\n\n\n",
                "@validate_call\n",
                "def make_item():\n",
                "    return Item()\n",
            ),
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        for (kind, member, what) in [
            (
                RelationKind::Inherits,
                "pydantic::BaseModel",
                "Item's base class",
            ),
            (
                RelationKind::Decorates,
                "pydantic::validate_call",
                "@validate_call",
            ),
            (RelationKind::Calls, "pydantic::Field", "Field(...)"),
        ] {
            let target = GraphNodeId::new(format!("symbol:{member}"));
            assert!(
                graph
                    .relations
                    .iter()
                    .any(|relation| relation.kind == kind && relation.target == target),
                "expected {what} to resolve to symbol:{member}, got {:?}",
                graph
                    .relations
                    .iter()
                    .filter(|relation| relation.kind == kind)
                    .map(|relation| relation.target.as_str())
                    .collect::<Vec<_>>(),
            );
        }

        // The package attribution LIT-44.1 wanted is kept, one hop away.
        let pydantic_package = graph
            .nodes
            .iter()
            .find_map(|node| match node {
                GraphNode::Package(package) if package.name == "pydantic" => Some(node.id()),
                _ => None,
            })
            .ok_or("missing package:pydantic node")?;
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::BelongsToPackage
                    && &relation.target == pydantic_package),
            "expected the external symbols to belong to package:pydantic"
        );

        // The property that actually failed in production: builder output
        // must satisfy the validator that gates `init`. Asserting graph
        // shape alone is what let `Calls -> Package` ship green.
        let invalid: Vec<_> = crate::graph::GraphValidator
            .validate(&graph, &artifacts)
            .into_iter()
            .filter(|issue| issue.kind == crate::graph::GraphIssueKind::InvalidRelationTarget)
            .collect();
        assert!(
            invalid.is_empty(),
            "builder produced invalid targets: {invalid:?}"
        );

        Ok(())
    }

    /// LIT-56: an external symbol is named as the package declares it. An
    /// aliased `from x import y as z` records `y`, and a plain `import x`
    /// reference (`x.Member`) records the member rather than the module.
    ///
    /// This is also the ripgrep corpus regression: `from multiprocessing
    /// import cpu_count` followed by `cpu_count()` produced `Calls ->
    /// package:multiprocessing`, which failed `init` at graph validation.
    #[test]
    fn python_external_symbols_use_declared_names_not_local_aliases()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("app.py"),
            concat!(
                "import collections\n",
                "from multiprocessing import cpu_count as cpus\n\n\n",
                "class Counts(collections.OrderedDict):\n",
                "    pass\n\n\n",
                "def sizes():\n",
                "    return cpus()\n",
            ),
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let target_of = |kind: RelationKind| {
            graph
                .relations
                .iter()
                .filter(|relation| relation.kind == kind)
                .map(|relation| relation.target.as_str().to_owned())
                .collect::<Vec<_>>()
        };

        assert!(
            target_of(RelationKind::Calls)
                .contains(&"symbol:multiprocessing::cpu_count".to_owned()),
            "aliased `cpus()` must record the declared name cpu_count, got {:?}",
            target_of(RelationKind::Calls),
        );
        assert!(
            target_of(RelationKind::Inherits)
                .contains(&"symbol:collections::OrderedDict".to_owned()),
            "`collections.OrderedDict` must name the member, not the module, got {:?}",
            target_of(RelationKind::Inherits),
        );

        let invalid: Vec<_> = crate::graph::GraphValidator
            .validate(&graph, &artifacts)
            .into_iter()
            .filter(|issue| issue.kind == crate::graph::GraphIssueKind::InvalidRelationTarget)
            .collect();
        assert!(
            invalid.is_empty(),
            "builder produced invalid targets: {invalid:?}"
        );

        Ok(())
    }

    /// LIT-22.3.4 AC1/AC4: a route-decorated handler produces a `Route`
    /// config node; a literal HTTP client call produces a high-confidence
    /// reference; a *dynamic* call target (an f-string, not a literal)
    /// stays low-confidence rather than being reported as if resolved.
    #[test]
    fn python_http_routes_and_calls_are_first_class_graph_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "import requests\n\n\n@app.get(\"/users/{id}\")\ndef get_user(id, dynamic_url):\n    requests.get(\"https://auth.example.test/verify\")\n    requests.get(dynamic_url)\n    return None\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let route = graph
            .nodes
            .iter()
            .find_map(|node| match node {
                GraphNode::Config(config) if config.name == "GET /users/{id}" => Some(config),
                _ => None,
            })
            .ok_or("missing route config node")?;
        assert_eq!(route.kind, crate::graph::model::ConfigNodeKind::Route);
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Contains && relation.target == route.id
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::HandlesRoute
                && relation.source == route.id
                && matches!(
                    graph.nodes.iter().find(|node| node.id() == &relation.target),
                    Some(GraphNode::Symbol(symbol)) if symbol.qualified_name == "service::get_user"
                )
                && relation.provenance.as_ref().is_some_and(|provenance| {
                    provenance.resolution == RelationResolution::HybridResolved
                        && provenance.language.as_deref() == Some("python")
                })
        }));
        assert!(graph.nodes.iter().all(|node| {
            !matches!(node, GraphNode::Unresolved(unresolved) if unresolved.value == "app.get(\"/users/{id}\")")
        }));

        let literal_call = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::References
                    && graph.nodes.iter().any(|node| {
                        node.id() == &relation.target
                            && matches!(node, GraphNode::Unresolved(u) if u.value == "https://auth.example.test/verify")
                    })
            })
            .ok_or("missing literal HTTP call relation")?;
        assert_eq!(literal_call.confidence, Confidence::High);

        let dynamic_call = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::References && graph.nodes.iter().any(|node| {
                    node.id() == &relation.target
                        && matches!(node, GraphNode::Unresolved(u) if u.value.contains("dynamic"))
                })
            })
            .ok_or("missing dynamic HTTP call relation")?;
        assert_eq!(dynamic_call.confidence, Confidence::Low);

        Ok(())
    }

    #[test]
    fn relations_store_language_and_resolution_provenance() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let python_import = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/python_app/__init__.py"
                    && relation.target.as_str() == "module:src.python_app.service"
            })
            .ok_or_else(|| std::io::Error::other("missing python import relation"))?;
        let provenance = python_import
            .provenance
            .as_ref()
            .ok_or_else(|| std::io::Error::other("missing python import provenance"))?;
        assert_eq!(provenance.language.as_deref(), Some("python"));
        assert_eq!(provenance.resolution, RelationResolution::HybridResolved);
        assert_eq!(provenance.resolver_strategy, "specialized-hybrid");
        assert_eq!(provenance.confidence, python_import.confidence);

        // App.tsx is now syntax-indexed (LIT-22.2.3): its import is a
        // syntax-only fact, not a generic-text fallback finding.
        let tsx_import = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:web/src/App.tsx"
                    && relation.provenance.as_ref().is_some_and(|provenance| {
                        provenance.language.as_deref() == Some("tsx")
                            && provenance.resolution == RelationResolution::SyntaxOnly
                    })
            })
            .ok_or_else(|| std::io::Error::other("missing tsx syntax-indexed import relation"))?;
        assert_eq!(tsx_import.confidence, crate::domain::Confidence::Low);

        Ok(())
    }

    /// LIT-22.3.6 AC1/AC4: a call that passes an argument to a
    /// locally-defined function produces a `DataFlows` relation to that
    /// function's symbol, distinct from the always-present `Calls` relation.
    #[test]
    fn call_with_argument_produces_a_dataflows_relation() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "def build(x):\n    return x\n\n\ndef run(value):\n    return build(value)\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let data_flows = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::DataFlows)
            .ok_or("expected a DataFlows relation")?;
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Calls
                    && relation.target == data_flows.target)
        );

        Ok(())
    }
}
