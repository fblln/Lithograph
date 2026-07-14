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

        let mut symbol_ids: BTreeMap<String, GraphNodeId> = BTreeMap::new();
        let mut callable_ids: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();

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
            callable_ids
                .entry(class.name.clone())
                .or_default()
                .push(id.clone());
            for decorator in &class.decorators {
                let target = self.unresolved(decorator);
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
                    let target = self.unresolved(return_type);
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
                let target = self.unresolved(decorator);
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
                let target = self.unresolved(return_type);
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

        for reference in &analysis.references {
            self.process_python_reference(
                artifact,
                artifact_node,
                reference,
                &symbol_ids,
                &callable_ids,
            );
        }

        // Base classes (LIT-22.3.3): only resolves to a same-file class by
        // bare name -- cross-module base classes stay `Unresolved` rather
        // than guessing which import they came from (AC3).
        for class in &analysis.classes {
            let Some(class_id) = symbol_ids.get(&class.name) else {
                continue;
            };
            for base in &class.bases {
                let base_name = base.rsplit('.').next().unwrap_or(base.as_str());
                let target = symbol_ids
                    .get(base_name)
                    .cloned()
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
                    .and_then(|resolved| self.python_modules.get(resolved).cloned())
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
    fn process_python_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::PythonReference,
        symbol_ids: &BTreeMap<String, GraphNodeId>,
        callable_ids: &BTreeMap<String, Vec<GraphNodeId>>,
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
                    let target = self.unresolved(&reference.value);
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
