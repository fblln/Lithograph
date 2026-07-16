use super::*;

impl BuilderState {
    pub(super) fn process_rust(
        &mut self,
        artifact: &Artifact,
        analysis: RustAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        // `RustAnalyzer` only sees this one file's own path, so its
        // `analysis.module_path` is always the naive whole-repo-relative
        // guess; the graph builder has the cross-artifact `cargo metadata`
        // knowledge needed to correct it, via `rust_module_path`.
        let module_path = self.rust_module_path(artifact.path.as_str());
        let module_id = self
            .rust_modules
            .get(&module_path)
            .cloned()
            .unwrap_or_else(|| {
                self.module(&module_path, ModuleLanguage::Rust, file_evidence(artifact))
            });
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        let mut symbol_ids: BTreeMap<String, GraphNodeId> = BTreeMap::new();
        // LIT-46: spans of every symbol here, so a rationale comment attaches
        // to the item it sits inside.
        let mut symbol_spans: Vec<super::rationale::SymbolSpan> = Vec::new();

        for item in analysis
            .structs
            .iter()
            .map(|item| (item, SymbolKind::Struct))
            .chain(analysis.enums.iter().map(|item| (item, SymbolKind::Enum)))
        {
            let (item, kind) = item;
            let qualified = format!("{module_path}::{}", item.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind,
                qualified_name: qualified,
                doc: item.doc.clone(),
                evidence: item.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![item.evidence.clone()],
            );
            symbol_ids.insert(item.name.clone(), id);
        }

        for trait_item in &analysis.traits {
            let qualified = format!("{module_path}::{}", trait_item.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Trait,
                qualified_name: qualified,
                doc: trait_item.doc.clone(),
                evidence: trait_item.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![trait_item.evidence.clone()],
            );
            symbol_ids.insert(trait_item.name.clone(), id);
        }

        for function in &analysis.functions {
            let qualified = format!("{module_path}::{}", function.name);
            let id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{}#{qualified}", artifact.path)),
                kind: SymbolKind::Function,
                qualified_name: qualified,
                doc: function.doc.clone(),
                evidence: function.evidence.clone(),
            }));
            if let Some(span) = function.evidence.span.clone() {
                symbol_spans.push(super::rationale::SymbolSpan {
                    id: id.clone(),
                    span,
                });
            }
            self.relate(
                artifact_node.clone(),
                id,
                RelationKind::Contains,
                Confidence::High,
                vec![function.evidence.clone()],
            );
        }

        self.process_rationale(artifact, artifact_node, &analysis.comments, &symbol_spans);

        for imp in &analysis.impls {
            let Some(trait_name) = &imp.trait_name else {
                continue;
            };
            let source = symbol_ids
                .get(&imp.target_type)
                .cloned()
                .unwrap_or_else(|| self.unresolved(&imp.target_type));
            let target = symbol_ids
                .get(trait_name)
                .cloned()
                .unwrap_or_else(|| self.unresolved(trait_name));
            self.relate(
                source,
                target,
                RelationKind::Implements,
                Confidence::High,
                vec![imp.evidence.clone()],
            );
        }

        for use_ in &analysis.uses {
            let candidate = use_.path.strip_prefix("crate::").unwrap_or(&use_.path);
            let target = self
                .rust_modules
                .get(candidate)
                .cloned()
                .unwrap_or_else(|| match rust_std_crate(candidate) {
                    Some(crate_name) => self.package(crate_name, true),
                    None => self.unresolved(&use_.path),
                });
            self.relate_with_provenance(
                artifact_node.clone(),
                target,
                RelationKind::Imports,
                Confidence::High,
                vec![use_.evidence.clone()],
                Some(artifact_provenance(
                    artifact,
                    RelationResolution::HybridResolved,
                    Confidence::High,
                )),
            );
        }

        for reference in &analysis.references {
            self.process_rust_reference(artifact, artifact_node, reference);
        }
    }
    fn process_rust_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::RustReference,
    ) {
        match reference.kind {
            RustReferenceKind::EnvRead => {
                let target = self.env_var(&reference.value);
                self.relate(
                    artifact_node.clone(),
                    target,
                    RelationKind::ReadsEnv,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                );
            }
            RustReferenceKind::Subprocess => {
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
            RustReferenceKind::Ffi => {
                let target = self.unresolved(&reference.value);
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Ffi,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(format_provenance(
                        "rust",
                        RelationResolution::SyntaxOnly,
                        reference.confidence,
                    )),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::GraphValidator;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    #[test]
    fn rust_module_paths_are_corrected_by_cargo_metadata_crate_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        // Before RustWorkspaceAnalyzer was wired in, `rust_source::module_path`
        // treated the whole repo-relative path as the module path, so
        // `rust/src/lib.rs` (the crate root, per `cargo_metadata`) wrongly
        // became module `rust::src` and `rust/src/bin/worker.rs` (its own
        // binary target root) wrongly became `rust::src::bin::worker`.
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let has_node = |id: &str| graph.nodes.iter().any(|node| node.id().as_str() == id);

        assert!(
            has_node("module:"),
            "expected the lib target's crate root to map to the empty module path"
        );
        assert!(
            has_node("module:worker"),
            "expected the bin target's own root to map to just its target name"
        );
        assert!(
            !has_node("module:rust::src"),
            "the old naive whole-path module id must no longer appear"
        );
        assert!(
            !has_node("module:rust::src::bin::worker"),
            "the old naive whole-path module id must no longer appear"
        );

        let lib_belongs_to_root = graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::BelongsToModule
                && relation.source.as_str() == "artifact:rust/src/lib.rs"
                && relation.target.as_str() == "module:"
        });
        assert!(lib_belongs_to_root);

        Ok(())
    }

    #[test]
    fn stdlib_and_prelude_references_become_external_packages_not_unresolved()
    -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("app.py"),
            "\
import os
import requests
from __future__ import annotations

def run():
    os.getenv(\"HOME\")
",
        )?;
        std::fs::write(
            repo.path().join("lib.rs"),
            "\
use std::collections::HashMap;
use core::fmt::Debug;
use serde::Serialize;

struct Foo;

impl Debug for Foo {}
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);

        let has_node = |id: &str| graph.nodes.iter().any(|node| node.id().as_str() == id);

        // Known stdlib imports resolve to shared external Package nodes
        // rather than per-file Unresolved noise.
        assert!(has_node("package:os"), "expected package:os");
        assert!(
            has_node("package:__future__"),
            "expected package:__future__"
        );
        assert!(has_node("package:std"), "expected package:std");
        assert!(
            !has_node("unresolved:os"),
            "os should not be Unresolved once classified as stdlib"
        );
        assert!(
            !has_node("unresolved:__future__"),
            "__future__ should not be Unresolved once classified as stdlib"
        );
        assert!(
            !has_node("unresolved:std::collections::HashMap"),
            "std:: use path should not be Unresolved once classified as stdlib"
        );

        // Genuinely unknown third-party references still fall through to
        // Unresolved -- this is a classification split, not a blanket
        // silencer of every import.
        assert!(
            has_node("unresolved:requests"),
            "third-party requests import should remain Unresolved"
        );
        assert!(
            !has_node("package:requests"),
            "third-party requests import must not be misclassified as stdlib"
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str().starts_with("unresolved:serde")),
            "third-party serde use path should remain Unresolved"
        );

        Ok(())
    }

    #[test]
    fn rust_impls_do_not_target_prelude_package_nodes() -> Result<(), Box<dyn std::error::Error>> {
        let repo = tempfile::TempDir::new()?;
        std::fs::write(
            repo.path().join("lib.rs"),
            "\
struct Route;

impl Drop for Route {
    fn drop(&mut self) {}
}
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);
        let issues = GraphValidator.validate(&graph, &artifacts);

        assert_eq!(issues, Vec::new());
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Implements
                    && relation.target.as_str() == "unresolved:Drop"),
            "expected the external Drop trait to stay unresolved for Implements"
        );
        assert!(
            !graph.relations.iter().any(|relation| {
                relation.kind == RelationKind::Implements
                    && relation.target.as_str() == "package:Drop"
            }),
            "Implements must not target package nodes"
        );

        Ok(())
    }

    /// LIT-22.3.3 AC1: `extern "C" { ... }` declarations produce `Ffi`
    /// relations to `Unresolved` nodes (the C symbol they name has no
    /// corresponding Rust graph node).
    #[test]
    fn rust_extern_block_produces_ffi_relations() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/lib.rs"),
            "extern \"C\" {\n    fn c_add(a: i32, b: i32) -> i32;\n    static VERSION: i32;\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let ffi: Vec<&Relation> = graph
            .relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::Ffi)
            .collect();
        assert_eq!(ffi.len(), 2);
        for relation in &ffi {
            assert_eq!(relation.confidence, Confidence::High);
            assert!(matches!(
                graph
                    .nodes
                    .iter()
                    .find(|node| node.id() == &relation.target),
                Some(GraphNode::Unresolved(_))
            ));
        }
        assert!(ffi.iter().any(|relation| {
            graph.nodes.iter().any(|node| {
                node.id() == &relation.target
                    && matches!(node, GraphNode::Unresolved(u) if u.value == "c_add")
            })
        }));

        Ok(())
    }
}
