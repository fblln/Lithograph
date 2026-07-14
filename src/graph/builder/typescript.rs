use super::*;

impl BuilderState {
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
        for class in &analysis.classes {
            let qualified = format!("{}::{}", artifact.path, class.name);
            let class_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Class,
                qualified_name: qualified,
                doc: None,
                evidence: class.evidence.clone(),
            }));
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
            &[
                "class_declaration",
                "abstract_class_declaration",
                "function_declaration",
                "generator_function_declaration",
                "method_definition",
            ],
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
        assert_eq!(
            provenance.resolver_strategy,
            "typescript-import-binding-call"
        );

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
