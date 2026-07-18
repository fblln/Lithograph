use super::*;

impl BuilderState {
    /// LIT-71: this file's own bare (non-relative, non-tsconfig-alias) npm
    /// imports, local binding name -> (declared package name, member name as
    /// exported), restricted to packages this repo's own `package.json`
    /// actually declares (`js_manifest_packages`, populated by a pre-pass
    /// mirroring LIT-44.1's Python one). An undeclared or unknown bare
    /// specifier contributes no binding, leaving its usage sites exactly as
    /// `Unresolved` as before -- classification only, never fabrication.
    ///
    /// Deliberately separate from `record_typescript_type_facts`'s `imports`
    /// (LIT-57): that list exists to type call receivers against this
    /// repository's *own* symbols, so it intentionally skips bare package
    /// specifiers entirely. This is the complementary case.
    fn typescript_bare_package_imports(
        &self,
        source_path: &str,
        analysis: &TypeScriptAnalysis,
        language_id: &str,
    ) -> BTreeMap<String, (String, String)> {
        let mut bindings = BTreeMap::new();
        for import in &analysis.syntax.imports {
            let Some(reference) =
                crate::resolve::extract_import_reference(language_id, &import.text)
            else {
                continue;
            };
            if reference.starts_with("./") || reference.starts_with("../") {
                continue;
            }
            if !self.ts_aliases.resolve(source_path, &reference).is_empty() {
                continue;
            }
            let Some(root) = crate::resolve::typescript_dependency_root(&reference) else {
                continue;
            };
            if !self.js_manifest_packages.contains(root) {
                continue;
            }
            for (exported, local) in
                crate::resolve::extract_typescript_import_bindings(&import.text)
            {
                bindings
                    .entry(local)
                    .or_insert_with(|| (root.to_owned(), exported));
            }
            if let Some(local) =
                crate::resolve::extract_typescript_default_import_binding(&import.text)
            {
                bindings
                    .entry(local)
                    .or_insert_with(|| (root.to_owned(), "default".to_owned()));
            }
        }
        bindings
    }

    /// LIT-57: hands this file's receiver-typing facts to the cross-file
    /// propagation pass, which runs once every file's symbols exist.
    ///
    /// TypeScript symbols are qualified by artifact path, so an import's
    /// "module" is the imported file's path. A specifier can name more than
    /// one candidate path (`./util` may be `util.ts` or `util.tsx`), so every
    /// candidate is forwarded; at most one exists as a symbol, and the pass
    /// takes only exact qualified-name hits. Bare package specifiers are
    /// skipped: their declarations are not in this repository.
    fn record_typescript_type_facts(
        &mut self,
        artifact: &Artifact,
        analysis: &TypeScriptAnalysis,
        language_id: &str,
    ) {
        let source_path = artifact.path.as_str();
        let mut imports = Vec::new();
        for import in &analysis.syntax.imports {
            let Some(reference) =
                crate::resolve::extract_import_reference(language_id, &import.text)
            else {
                continue;
            };
            // LIT-45.2: a first-party module may be imported by tsconfig alias
            // rather than relatively, so those specifiers carry receiver types
            // too. An alias that matches nothing yields no candidates.
            let relative = reference.starts_with("./") || reference.starts_with("../");
            let bases: Vec<String> = if relative {
                crate::resolve::typescript_import_candidates(source_path, &reference, language_id)
            } else {
                self.ts_aliases
                    .resolve(source_path, &reference)
                    .into_iter()
                    .flat_map(|aliased| crate::resolve::import_candidates(&aliased, language_id))
                    .collect()
            };
            if bases.is_empty() {
                continue;
            }
            for (exported, local) in
                crate::resolve::extract_typescript_import_bindings(&import.text)
            {
                imports.push(crate::resolve::ImportBindingFact {
                    local,
                    modules: bases.clone(),
                    symbol: exported,
                });
            }
        }

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
                receiver: match call.receiver.as_str() {
                    "this" => crate::resolve::Receiver::Enclosing,
                    name => crate::resolve::Receiver::Named(name.to_owned()),
                },
                method: call.method.clone(),
                enclosing_class: call.enclosing_class.clone(),
                evidence: call.evidence.clone(),
            })
            .collect();
        let bases = analysis
            .classes
            .iter()
            .flat_map(|class| {
                class
                    .bases
                    .iter()
                    .map(|base| crate::resolve::BaseClassFact {
                        class: class.name.clone(),
                        base: base.clone(),
                        evidence: class.evidence.clone(),
                    })
            })
            .collect();

        // LIT-45.3: resolve each re-export's specifier to candidate artifact
        // paths here, where the source path is known, so the barrel walker
        // works in artifact paths only.
        let re_exports = analysis
            .re_exports
            .iter()
            .filter(|re_export| {
                re_export.specifier.starts_with("./") || re_export.specifier.starts_with("../")
            })
            .map(|re_export| crate::resolve::ReExport {
                targets: crate::resolve::typescript_import_candidates(
                    source_path,
                    &re_export.specifier,
                    language_id,
                ),
                kind: match &re_export.kind {
                    TypeScriptReExportKind::Star => crate::resolve::ReExportKind::Star,
                    TypeScriptReExportKind::Named { exported, local } => {
                        crate::resolve::ReExportKind::Named {
                            exported: exported.clone(),
                            local: local.clone(),
                        }
                    }
                },
            })
            .collect();

        self.type_facts.insert(
            source_path.to_owned(),
            crate::resolve::FileTypeFacts {
                module: source_path.to_owned(),
                language: language_id.to_owned(),
                imports,
                bindings,
                bases,
                member_calls,
                re_exports,
            },
        );
    }

    /// Adds TypeScript/TSX's typed declaration symbols, then reuses the
    /// syntax-indexed fact pass for imports, type references, identifier
    /// usages, and definitions that do not yet have a richer symbol kind
    /// (such as type aliases and enums).
    pub(super) fn process_typescript(
        &mut self,
        artifact: &Artifact,
        analysis: TypeScriptAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        let module_id = self.module(
            artifact.path.as_str(),
            ModuleLanguage::TypeScript(analysis.language),
            file_evidence(artifact),
        );
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        let language_id = analysis.language.registry_id();
        // A name can legitimately identify several methods in different
        // classes. Keeping every candidate lets us resolve only singleton
        // same-file names and leave ambiguous calls for the conservative
        // post-build resolver instead of choosing a plausible-but-wrong one.
        let mut callable_ids: BTreeMap<String, Vec<GraphNodeId>> = BTreeMap::new();
        // LIT-46: spans of the symbols this pass creates, so rationale
        // comments attach to the class or method they sit inside.
        let mut typed_symbols: Vec<super::rationale::SymbolSpan> = Vec::new();
        for class in &analysis.classes {
            let qualified = format!("{}::{}", artifact.path, class.name);
            let class_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Class,
                qualified_name: qualified,
                doc: None,
                evidence: class.evidence.clone(),
            }));
            if let Some(span) = class.evidence.span.clone() {
                typed_symbols.push(super::rationale::SymbolSpan {
                    id: class_id.clone(),
                    span,
                });
            }
            self.relate_with_provenance(
                artifact_node.clone(),
                class_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![class.evidence.clone()],
                Some(format_provenance(
                    language_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            for method in &class.methods {
                let qualified = format!("{}::{}::{}", artifact.path, class.name, method.name);
                let method_id = self.insert(GraphNode::Symbol(SymbolNode {
                    id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                    kind: SymbolKind::Method,
                    qualified_name: qualified,
                    doc: None,
                    evidence: method.evidence.clone(),
                }));
                if let Some(span) = method.evidence.span.clone() {
                    typed_symbols.push(super::rationale::SymbolSpan {
                        id: method_id.clone(),
                        span,
                    });
                }
                self.relate_with_provenance(
                    class_id.clone(),
                    method_id.clone(),
                    RelationKind::Contains,
                    Confidence::High,
                    vec![method.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::SyntaxOnly,
                        Confidence::High,
                    )),
                );
                callable_ids
                    .entry(method.name.clone())
                    .or_default()
                    .push(method_id);
            }
        }

        for function in &analysis.functions {
            let qualified = format!("{}::{}", artifact.path, function.name);
            let function_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Function,
                qualified_name: qualified,
                doc: None,
                evidence: function.evidence.clone(),
            }));
            self.relate_with_provenance(
                artifact_node.clone(),
                function_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
                Some(format_provenance(
                    language_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            callable_ids
                .entry(function.name.clone())
                .or_default()
                .push(function_id);
        }

        self.record_typescript_type_facts(artifact, &analysis, language_id);
        let bare_imports =
            self.typescript_bare_package_imports(artifact.path.as_str(), &analysis, language_id);

        for call in &analysis.calls {
            if let Some([target]) = callable_ids.get(call.name.as_str()).map(Vec::as_slice) {
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target.clone(),
                    RelationKind::Calls,
                    Confidence::High,
                    vec![call.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::HybridResolved,
                        Confidence::High,
                    )),
                );
            } else if let Some((package, member)) = bare_imports.get(call.name.as_str()) {
                // LIT-71: a bare call to a name this file imported directly
                // from a declared npm dependency (`useForm(...)` after
                // `import { useForm } from "react-hook-form"`) resolves to
                // the known package member instead of `Unresolved`.
                let target =
                    self.typescript_external_symbol(package, member, call.evidence.clone());
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Calls,
                    Confidence::Low,
                    vec![call.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            } else {
                let target = self.unresolved(&call.name);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Calls,
                    Confidence::Low,
                    vec![call.evidence.clone()],
                    Some(format_provenance(
                        language_id,
                        RelationResolution::SyntaxOnly,
                        Confidence::Low,
                    )),
                );
            }
        }

        self.process_syntax_indexed_facts(
            artifact,
            match analysis.language {
                TypeScriptLanguage::TypeScript => SyntaxIndexedLanguage::TypeScript,
                TypeScriptLanguage::Tsx => SyntaxIndexedLanguage::Tsx,
            },
            analysis.syntax,
            artifact_node,
            super::evidence::TypedPassFacts {
                definition_kinds: &[
                    "class_declaration",
                    "abstract_class_declaration",
                    "function_declaration",
                    "generator_function_declaration",
                    "method_definition",
                ],
                symbol_spans: typed_symbols,
                bare_package_imports: bare_imports,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-23.5: TypeScript's specialized analyzer emits named typed
    /// symbols while its tree-sitter imports, type references, and generic
    /// usages remain part of the graph.
    #[test]
    fn typescript_deep_analysis_keeps_syntax_facts_and_typed_symbols()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.ts"),
            "import type { Config } from \"./types\";\nexport class Service {\n  run(config: Config): void {}\n}\nexport const start = (config: Config) => new Service().run(config);\n",
        )?;
        std::fs::write(temp.path().join("types.ts"), "export type Config = {};\n")?;
        std::fs::write(
            temp.path().join("App.tsx"),
            "export function App() { return <main />; }\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let typed_symbols: Vec<(&str, SymbolKind)> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Symbol(symbol)
                    if matches!(
                        symbol.qualified_name.as_str(),
                        "service.ts::Service"
                            | "service.ts::Service::run"
                            | "service.ts::start"
                            | "App.tsx::App"
                    ) =>
                {
                    Some((symbol.qualified_name.as_str(), symbol.kind))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            typed_symbols,
            vec![
                ("App.tsx::App", SymbolKind::Function),
                ("service.ts::Service", SymbolKind::Class),
                ("service.ts::Service::run", SymbolKind::Method),
                ("service.ts::start", SymbolKind::Function),
            ]
        );
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Imports
                && relation.source.as_str() == "artifact:service.ts"
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::TypeRefs
                && relation.source.as_str() == "artifact:service.ts"
        }));
        assert!(graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::Usages
                && relation.source.as_str() == "artifact:service.ts"
        }));

        Ok(())
    }

    /// LIT-23.6: direct TypeScript calls resolve only to a unique local
    /// callable symbol. Missing and ambiguous names remain low-confidence
    /// unresolved calls rather than being assigned a plausible target.
    #[test]
    fn typescript_same_file_calls_resolve_conservatively() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.ts"),
            "function helper() {}\nclass First { run() {} }\nclass Second { run() {} }\nhelper();\nrun();\nmissing();\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let resolved = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.target.as_str() == "symbol:service.ts#service.ts::helper"
            })
            .ok_or("missing resolved same-file call")?;
        assert_eq!(resolved.confidence, Confidence::High);
        assert_eq!(
            resolved
                .provenance
                .as_ref()
                .ok_or("missing call provenance")?
                .resolution,
            RelationResolution::HybridResolved
        );

        for name in ["run", "missing"] {
            let relation = graph
                .relations
                .iter()
                .find(|relation| {
                    relation.kind == RelationKind::Calls
                        && matches!(
                            graph.nodes.iter().find(|node| node.id() == &relation.target),
                            Some(GraphNode::Unresolved(unresolved)) if unresolved.value == name
                        )
                })
                .ok_or("missing unresolved conservative call")?;
            assert_eq!(relation.confidence, Confidence::Low);
            assert_eq!(
                relation
                    .provenance
                    .as_ref()
                    .ok_or("missing unresolved call provenance")?
                    .resolution,
                RelationResolution::SyntaxOnly
            );
        }

        Ok(())
    }

    /// LIT-23.6: a named binding from a relative local TypeScript import
    /// resolves after all artifacts have contributed their typed symbols.
    #[test]
    fn typescript_imported_call_resolves_to_local_export() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("lib.ts"),
            "export function greet(): void {}\n",
        )?;
        std::fs::write(
            temp.path().join("app.ts"),
            "import { greet as say } from \"./lib\";\nsay();\nunknown();\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let relation = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.source.as_str() == "artifact:app.ts"
                    && relation.target.as_str() == "symbol:lib.ts#lib.ts::greet"
            })
            .ok_or("missing imported TypeScript call")?;
        assert_eq!(relation.confidence, Confidence::High);
        let provenance = relation
            .provenance
            .as_ref()
            .ok_or("missing call provenance")?;
        assert_eq!(provenance.resolution, RelationResolution::HybridResolved);
        assert_eq!(provenance.resolver_strategy, "typescript-import-binding");

        let unknown = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Calls
                    && relation.source.as_str() == "artifact:app.ts"
                    && matches!(
                        graph.nodes.iter().find(|node| node.id() == &relation.target),
                        Some(GraphNode::Unresolved(unresolved)) if unresolved.value == "unknown"
                    )
            })
            .ok_or("missing unresolved imported-file call")?;
        assert_eq!(unknown.confidence, Confidence::Low);

        Ok(())
    }

    /// LIT-75: a use-site reference to a name this file imported from a
    /// relative local module resolves to that module's exported symbol, not
    /// a per-file `unresolved:<name>` node -- covering both a named import
    /// (`{ ApiError }`) and a default import (`Widget`) used as a JSX
    /// component. A name that is not imported at all stays unresolved rather
    /// than being guessed toward some same-named symbol elsewhere.
    #[test]
    fn typescript_local_import_references_resolve_at_use_sites()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("Widget.tsx"),
            "const Widget = () => null;\nexport default Widget;\n",
        )?;
        std::fs::write(temp.path().join("errors.ts"), "export class ApiError {}\n")?;
        std::fs::write(
            temp.path().join("Panel.tsx"),
            concat!(
                "import Widget from \"./Widget\";\n",
                "import { ApiError } from \"./errors\";\n",
                "interface PanelProps {\n  title: string;\n}\n",
                "const handle = (e: ApiError) => e;\n",
                "const view = (props: PanelProps) => <Widget />;\n",
                "const other = () => <Missing />;\n",
            ),
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let resolved_to = |target: &str| {
            graph.relations.iter().any(|relation| {
                relation.source.as_str() == "artifact:Panel.tsx"
                    && relation.target.as_str() == target
                    && relation.provenance.as_ref().is_some_and(|provenance| {
                        provenance.resolution == RelationResolution::HybridResolved
                            && provenance.resolver_strategy == "typescript-import-binding"
                    })
            })
        };
        // Named import referenced in a type annotation -> the local class.
        assert!(
            resolved_to("symbol:errors.ts#errors.ts::ApiError"),
            "ApiError reference should resolve to the local class"
        );
        // Default import used as a JSX component -> the local default export.
        assert!(
            resolved_to("symbol:Widget.tsx#Widget.tsx::Widget"),
            "Widget JSX usage should resolve to the local default export"
        );
        // A same-file interface referenced in a type annotation -> the
        // now name-bearing local interface symbol (LIT-75 AC2).
        assert!(
            resolved_to("symbol:Panel.tsx::PanelProps"),
            "PanelProps should resolve to the same-file interface"
        );
        // A name imported by nobody stays unresolved rather than being guessed.
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| matches!(node, GraphNode::Unresolved(node) if node.value == "Missing")),
            "an unimported JSX name must stay unresolved"
        );
        Ok(())
    }

    /// LIT-77: a bare reference to a JS/TS builtin global (`JSON`, `Array`)
    /// is classified as an external `javascript::<name>` symbol rather than a
    /// per-file `Unresolved` node, but a local declaration that shadows a
    /// builtin name still wins -- resolution runs before builtin
    /// classification, so the shadowed name never reaches it.
    #[test]
    fn typescript_builtins_classify_externally_but_locals_shadow_them()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        // `Array` is defined locally here, so it must stay local, not builtin.
        std::fs::write(
            temp.path().join("app.ts"),
            "class Array {}\nconst a = new Array();\nconst s = JSON.stringify(a);\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        // `JSON` has no local declaration -> external builtin symbol.
        assert!(
            graph.nodes.iter().any(|node| matches!(
                node,
                GraphNode::Symbol(symbol)
                    if symbol.qualified_name == "javascript::JSON"
                    && symbol.kind == SymbolKind::External
            )),
            "JSON should be classified as an external builtin"
        );
        // The locally-declared `Array` must NOT be reclassified as a builtin:
        // no `javascript::Array` symbol exists, and the reference resolves to
        // the local class instead.
        assert!(
            !graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == "symbol:javascript::Array"),
            "a locally-declared name that shadows a builtin must not be reclassified"
        );
        Ok(())
    }

    /// LIT-71: a name this file imports directly from a declared npm
    /// dependency resolves to that package's own member -- via the bare
    /// `Calls` fallback for a plain call, and via the generic Usages pass
    /// for a name used only as a receiver -- instead of a fresh per-file
    /// `Unresolved` node. A bare specifier with no matching `package.json`
    /// dependency is never fabricated into one.
    #[test]
    fn typescript_bare_package_imports_resolve_default_and_named_bindings()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name":"app","dependencies":{"react":"^18.0.0","react-hook-form":"^7.0.0"}}"#,
        )?;
        std::fs::write(
            temp.path().join("form.ts"),
            concat!(
                "import React from \"react\";\n",
                "import { useForm } from \"react-hook-form\";\n",
                "import { useQuery } from \"not-declared-pkg\";\n\n",
                "export function run() {\n",
                "  useForm();\n",
                "  React.createElement(\"div\");\n",
                "  useQuery();\n",
                "}\n",
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
            target_of(RelationKind::Calls).contains(&"symbol:react-hook-form::useForm".to_owned()),
            "a bare call to a directly-imported declared-dependency name must resolve to its \
             package member, got {:?}",
            target_of(RelationKind::Calls),
        );
        assert!(
            target_of(RelationKind::Usages).contains(&"symbol:react::default".to_owned()),
            "a default-imported declared-dependency name used as a receiver must resolve, got {:?}",
            target_of(RelationKind::Usages),
        );
        assert!(
            target_of(RelationKind::Calls).contains(&"unresolved:useQuery".to_owned()),
            "a bare specifier absent from package.json must never be fabricated into a package, \
             got {:?}",
            target_of(RelationKind::Calls),
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

    /// LIT-23.1: a multi-line, type-only TypeScript import resolves through
    /// the hybrid resolver pipeline (wired into `build`/`build_with_cache`)
    /// to a real Artifact target, and the now-orphaned raw-multi-line-text
    /// `Unresolved` node it originally targeted is pruned from the graph
    /// entirely -- reproducing and closing the exact bug found on an
    /// external repository, where such nodes previously lingered with a
    /// value containing the whole raw statement, newlines included.
    #[test]
    fn resolved_multi_line_import_prunes_its_orphaned_unresolved_node()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/types.ts"),
            "export type CameraRig = { fov: number };\nexport type RouteSummary = { name: string };\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import type {\n  CameraRig,\n  RouteSummary,\n} from \"./types\";\nexport function noop(): CameraRig | RouteSummary | null {\n  return null;\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph.nodes.iter().all(|node| match node {
                GraphNode::Unresolved(unresolved) => !unresolved.value.contains('\n'),
                _ => true,
            }),
            "no Unresolved node should retain a raw multi-line import statement as its value"
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Imports
                    && relation.target.as_str() == "artifact:src/types.ts"),
            "the import should still resolve to the real local artifact"
        );

        Ok(())
    }
}
