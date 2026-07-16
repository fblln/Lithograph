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
            self.process_rust_reference(artifact, artifact_node, reference, &symbol_ids);
        }
    }
    /// Interns the external symbol a Rust path names, when its root crate is
    /// the standard library or a Cargo-declared dependency, plus the
    /// `BelongsToPackage` edge tying it to that crate.
    ///
    /// Mirrors LIT-56's Python equivalent: the member is named, not the whole
    /// package, because `Calls` may not target a `Package` and because
    /// `memchr::memchr_iter` is a more useful answer than `memchr`.
    /// Returns `None` for a path rooted at anything undeclared, which stays
    /// Unresolved rather than being assumed external.
    fn rust_external_symbol(&mut self, path: &str, evidence: &EvidenceRef) -> Option<GraphNodeId> {
        let trimmed = path.strip_prefix("::").unwrap_or(path);
        let root = trimmed.split("::").next()?;
        // A bare name has no crate root to attribute it to.
        if root == trimmed {
            return None;
        }
        // A crate that lives in this repository is never external, however
        // its manifest happens to declare it.
        if self.rust_local_crates.contains(root) {
            return None;
        }
        let crate_name = match rust_std_crate(trimmed) {
            Some(name) => name,
            None if self.rust_manifest_packages.contains(root) => root,
            None => return None,
        };
        let symbol_id = self.insert(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(format!("symbol:{trimmed}")),
            kind: SymbolKind::External,
            qualified_name: trimmed.to_owned(),
            doc: None,
            evidence: evidence.clone(),
        }));
        let package_id = self.package(crate_name, true);
        self.relate(
            symbol_id.clone(),
            package_id,
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![evidence.clone()],
        );
        Some(symbol_id)
    }

    fn process_rust_reference(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        reference: &crate::analysis::RustReference,
        symbol_ids: &BTreeMap<String, GraphNodeId>,
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
            // LIT-63: a call names its callee by its last path segment
            // (`flags::parse` calls `parse`). A unique match among this
            // file's own items is proof; anything else is left Unresolved for
            // the hybrid resolver, which can see the whole graph.
            RustReferenceKind::Call => {
                let simple = reference
                    .value
                    .rsplit("::")
                    .next()
                    .unwrap_or(&reference.value);
                let (target, resolution) = match symbol_ids.get(simple) {
                    Some(id) => (id.clone(), RelationResolution::HybridResolved),
                    // LIT-66: a path rooted at std or a Cargo-declared crate
                    // names code that is not in this repository and never will
                    // be. That is a fact about a dependency, not a gap in
                    // resolution, and leaving it Unresolved says the resolver
                    // failed at something it was never going to find.
                    None => {
                        match self.rust_external_symbol(&reference.value, &reference.evidence) {
                            Some(id) => (id, RelationResolution::HybridResolved),
                            None => (
                                self.unresolved(&reference.value),
                                RelationResolution::SyntaxOnly,
                            ),
                        }
                    }
                };
                self.relate_with_provenance(
                    artifact_node.clone(),
                    target,
                    RelationKind::Calls,
                    reference.confidence,
                    vec![reference.evidence.clone()],
                    Some(format_provenance("rust", resolution, reference.confidence)),
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

    /// LIT-66: a call path rooted at std or a declared crate names code
    /// outside this repository -- a fact about a dependency, not a resolution
    /// gap. A path rooted at an in-repository crate, or at nothing declared,
    /// must not be called external.
    #[test]
    fn rust_calls_into_std_and_declared_crates_become_external_symbols()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\n\n[dependencies]\nmemchr = \"2\"\n",
        )?;
        std::fs::write(
            temp.path().join("src/lib.rs"),
            concat!(
                "pub fn run() {\n",
                "    let _ = std::cmp::max(1, 2);\n",
                "    let _ = memchr::memchr_iter(b'x', b\"y\");\n",
                "    let _ = undeclared_crate::helper();\n",
                "}\n",
            ),
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let external: Vec<&str> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Symbol(symbol) if symbol.kind == SymbolKind::External => {
                    Some(symbol.qualified_name.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(
            external.contains(&"std::cmp::max"),
            "stdlib path must be external, got {external:?}",
        );
        assert!(
            external.contains(&"memchr::memchr_iter"),
            "declared crate path must be external, got {external:?}",
        );
        assert!(
            !external
                .iter()
                .any(|name| name.starts_with("undeclared_crate")),
            "an undeclared root must stay Unresolved, got {external:?}",
        );
        assert!(
            graph.relations.iter().any(|relation| {
                relation.kind == RelationKind::BelongsToPackage
                    && relation.target.as_str() == "package:memchr"
            }),
            "an external symbol must be tied to its package",
        );

        let invalid: Vec<_> = GraphValidator
            .validate(&graph, &artifacts)
            .into_iter()
            .filter(|issue| issue.kind == crate::graph::GraphIssueKind::InvalidRelationTarget)
            .collect();
        assert!(invalid.is_empty(), "invalid targets: {invalid:?}");

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
