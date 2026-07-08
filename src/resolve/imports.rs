//! Per-language import-reference extraction for [`LanguageImportResolver`]
//! (LIT-22.3.2). Each extractor turns one syntax-indexed language's raw
//! import statement text (captured verbatim by `TreeSitterParserAdapter`,
//! see LIT-22.2.3) into the best-effort bare module/package/path reference
//! a resolver can look up -- never inventing a reference the source text
//! doesn't actually contain (AC3).

use super::{ResolvedTarget, Resolver, ResolverContext};
use crate::domain::Confidence;
use crate::graph::Relation;
use std::path::{Component, Path, PathBuf};

/// Extracts the referenced module/package/path from one raw import
/// statement, based on `language` (a `LanguageRegistryEntry::id`, e.g.
/// `"typescript"` or `"c_sharp"`). Returns `None` when `language` isn't a
/// family this module extracts references for, or the statement has no
/// recognizable reference.
pub fn extract_import_reference(language: &str, raw_text: &str) -> Option<String> {
    match language {
        "typescript" | "tsx" | "javascript" | "go" => quoted_literal(raw_text),
        "java" => strip_import_statement(raw_text, &["import static ", "import "], ";"),
        "kotlin" => strip_import_statement(raw_text, &["import "], ""),
        "c_sharp" => strip_import_statement(raw_text, &["using static ", "using "], ";"),
        "php" => php_reference(raw_text),
        "c" | "cpp" => angle_or_quoted_literal(raw_text),
        _ => None,
    }
}

/// File extensions worth appending to a relative-import candidate path for
/// `language`, in preference order. Only languages whose import syntax is
/// itself extension-less (JS/TS's `from "./util"`) need this; the rest
/// return an empty slice and rely on the exact-path fallback.
fn candidate_extensions(language: &str) -> &'static [&'static str] {
    match language {
        "typescript" => &[".ts", ".tsx"],
        "tsx" => &[".tsx", ".ts"],
        "javascript" => &[".js", ".jsx"],
        _ => &[],
    }
}

fn quoted_literal(text: &str) -> Option<String> {
    let mut chars = text.char_indices();
    let (start, quote) = chars.find_map(|(index, character)| {
        (character == '"' || character == '\'').then_some((index, character))
    })?;
    let end = text[start + 1..].find(quote)? + start + 1;
    let literal = &text[start + 1..end];
    (!literal.is_empty()).then(|| literal.to_owned())
}

fn angle_or_quoted_literal(text: &str) -> Option<String> {
    if let Some(start) = text.find('<')
        && let Some(relative_end) = text[start + 1..].find('>')
    {
        let literal = &text[start + 1..start + 1 + relative_end];
        if !literal.is_empty() {
            return Some(literal.to_owned());
        }
    }
    quoted_literal(text)
}

fn strip_import_statement(text: &str, prefixes: &[&str], suffix: &str) -> Option<String> {
    let trimmed = text.trim();
    for prefix in prefixes {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            let name = rest.strip_suffix(suffix).unwrap_or(rest).trim();
            return (!name.is_empty()).then(|| name.to_owned());
        }
    }
    None
}

fn php_reference(text: &str) -> Option<String> {
    if let Some(rest) = text.trim().strip_prefix("use ") {
        let name = rest.trim_end_matches(';').trim();
        let name = name.split(" as ").next().unwrap_or(name).trim();
        return (!name.is_empty()).then(|| name.to_owned());
    }
    quoted_literal(text)
}

/// Resolves a whole-repo-relative candidate path from `source_dir` (the
/// importing artifact's own directory) and a `./`/`../`-relative
/// `reference`, collapsing `.`/`..` components lexically (no filesystem
/// access -- the candidate is checked against known artifact paths by the
/// caller, not the disk).
fn resolve_relative_path(source_dir: &Path, reference: &str) -> String {
    let mut components: Vec<Component<'_>> = source_dir.components().collect();
    for part in Path::new(reference).components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components
        .into_iter()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('\\', "/")
}

/// Resolves syntax-indexed (LIT-22.2.3) import facts for the languages
/// LIT-22.3.2 adds parity for: an exact package-map match, a relative
/// local-file match (JS/TS/PHP-style `./`/`../` references, resolved
/// against the importing artifact's own directory), or -- Go only, whose
/// import paths are module-path-relative rather than file-relative -- a
/// prefix match against the local package's own module path. Anything else
/// is left for [`super::PackageMapResolver`]/[`super::LocalArtifactPathResolver`]
/// or stays unresolved (AC3: never fabricate a match).
pub struct LanguageImportResolver;

impl Resolver for LanguageImportResolver {
    fn strategy(&self) -> &'static str {
        "language-import-reference"
    }

    fn resolve(
        &self,
        context: &ResolverContext<'_>,
        relation: &Relation,
        unresolved_value: &str,
    ) -> Option<ResolvedTarget> {
        let language = relation.provenance.as_ref()?.language.as_deref()?;
        let reference = extract_import_reference(language, unresolved_value)?;

        if let Some(target) = context.packages_by_name.get(reference.as_str()) {
            return Some(ResolvedTarget {
                target: (*target).clone(),
                confidence: Confidence::High,
            });
        }

        if reference.starts_with("./") || reference.starts_with("../") {
            let source_path = relation.source.as_str().strip_prefix("artifact:")?;
            let source_dir = Path::new(source_path).parent().unwrap_or(Path::new(""));
            let candidate = resolve_relative_path(source_dir, &reference);
            for extension in candidate_extensions(language) {
                let with_extension = format!("{candidate}{extension}");
                if let Some(target) = context.artifacts_by_path.get(with_extension.as_str()) {
                    return Some(ResolvedTarget {
                        target: (*target).clone(),
                        confidence: Confidence::High,
                    });
                }
            }
            if let Some(target) = context.artifacts_by_path.get(candidate.as_str()) {
                return Some(ResolvedTarget {
                    target: (*target).clone(),
                    confidence: Confidence::High,
                });
            }
        }

        if language == "go" {
            return context
                .local_package_names
                .iter()
                .find(|local_name| {
                    reference == **local_name || reference.starts_with(&format!("{local_name}/"))
                })
                .and_then(|local_name| context.packages_by_name.get(local_name))
                .map(|target| ResolvedTarget {
                    target: (*target).clone(),
                    confidence: Confidence::Low,
                });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::extract_import_reference;

    #[test]
    fn extracts_typescript_javascript_and_go_quoted_specifiers() {
        assert_eq!(
            extract_import_reference("typescript", "import { List } from \"immutable\";"),
            Some("immutable".to_owned())
        );
        assert_eq!(
            extract_import_reference("javascript", "import \"./setup\";"),
            Some("./setup".to_owned())
        );
        assert_eq!(
            extract_import_reference("go", "import \"github.com/gin-gonic/gin\""),
            Some("github.com/gin-gonic/gin".to_owned())
        );
    }

    #[test]
    fn extracts_java_kotlin_and_csharp_dotted_paths() {
        assert_eq!(
            extract_import_reference("java", "import java.util.List;"),
            Some("java.util.List".to_owned())
        );
        assert_eq!(
            extract_import_reference("java", "import static java.util.Arrays.asList;"),
            Some("java.util.Arrays.asList".to_owned())
        );
        assert_eq!(
            extract_import_reference("kotlin", "import kotlin.collections.List"),
            Some("kotlin.collections.List".to_owned())
        );
        assert_eq!(
            extract_import_reference("c_sharp", "using System.Collections.Generic;"),
            Some("System.Collections.Generic".to_owned())
        );
    }

    #[test]
    fn extracts_php_namespace_use_and_include_paths() {
        assert_eq!(
            extract_import_reference("php", "use Foo\\Bar;"),
            Some("Foo\\Bar".to_owned())
        );
        assert_eq!(
            extract_import_reference("php", "use Foo\\Bar as Baz;"),
            Some("Foo\\Bar".to_owned())
        );
        assert_eq!(
            extract_import_reference("php", "include 'legacy/bootstrap.php'"),
            Some("legacy/bootstrap.php".to_owned())
        );
    }

    #[test]
    fn extracts_c_and_cpp_angle_and_quoted_includes() {
        assert_eq!(
            extract_import_reference("c", "#include <stdio.h>"),
            Some("stdio.h".to_owned())
        );
        assert_eq!(
            extract_import_reference("cpp", "#include \"local_header.h\""),
            Some("local_header.h".to_owned())
        );
    }

    #[test]
    fn unknown_language_and_unrecognizable_text_extract_nothing() {
        assert_eq!(extract_import_reference("ruby", "require 'set'"), None);
        assert_eq!(
            extract_import_reference("java", "package com.example;"),
            None
        );
    }
}

/// End-to-end coverage (LIT-22.3.2 AC2/AC3/AC4): builds a real graph from
/// an isolated repo per language family (not the shared polyglot fixture,
/// to avoid golden-snapshot churn) and runs the default resolver pipeline
/// over it, the same way a real `init`/`update` would.
#[cfg(test)]
mod pipeline_tests {
    use crate::domain::Confidence;
    use crate::graph::{GraphBuilder, GraphNode, RelationKind, RelationResolution};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::resolve::HybridResolverPipeline;

    fn resolved_target<'a>(
        graph: &'a crate::graph::Graph,
        source_artifact: &str,
    ) -> Option<&'a crate::graph::Relation> {
        graph.relations.iter().find(|relation| {
            relation.kind == RelationKind::Imports
                && relation.source.as_str() == format!("artifact:{source_artifact}")
        })
    }

    #[test]
    fn typescript_relative_import_resolves_to_local_artifact()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("src/util.ts"),
            "export function noop(): void {}\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { noop } from \"./util\";\nnoop();\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let mut graph = GraphBuilder.build(temp.path(), &artifacts);
        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 1);
        let relation = resolved_target(&graph, "src/app.ts").ok_or("missing import relation")?;
        assert_eq!(relation.target.as_str(), "artifact:src/util.ts");
        assert_eq!(relation.confidence, Confidence::High);
        assert_eq!(
            relation
                .provenance
                .as_ref()
                .ok_or("missing provenance")?
                .resolution,
            RelationResolution::HybridResolved
        );

        Ok(())
    }

    #[test]
    fn go_import_resolves_against_declared_dependency_and_local_subpackage()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("go.mod"),
            "module github.com/acme/svc\n\nrequire github.com/gin-gonic/gin v1.9.1\n",
        )?;
        std::fs::write(
            temp.path().join("main.go"),
            "package main\n\nimport \"github.com/gin-gonic/gin\"\nimport \"github.com/acme/svc/internal/util\"\n\nfunc main() {}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let mut graph = GraphBuilder.build(temp.path(), &artifacts);
        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 2);
        let external = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.target.as_str() == "package:github.com/gin-gonic/gin"
            })
            .ok_or("missing external dependency relation")?;
        assert_eq!(external.confidence, Confidence::High);

        let local = graph
            .relations
            .iter()
            .find(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.target.as_str() == "package:github.com/acme/svc"
            })
            .ok_or("missing local subpackage relation")?;
        assert_eq!(local.confidence, Confidence::Low);

        Ok(())
    }

    #[test]
    fn unmatched_java_import_stays_unresolved_not_fabricated()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/main/java"))?;
        std::fs::write(
            temp.path().join("src/main/java/App.java"),
            "import com.example.totally.unknown.Widget;\nclass App {}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let mut graph = GraphBuilder.build(temp.path(), &artifacts);
        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        assert_eq!(report.resolved, 0);
        assert_eq!(report.still_unresolved, 1);
        let relation =
            resolved_target(&graph, "src/main/java/App.java").ok_or("missing import relation")?;
        assert!(matches!(
            graph
                .nodes
                .iter()
                .find(|node| node.id() == &relation.target),
            Some(GraphNode::Unresolved(_))
        ));
        assert_eq!(
            relation
                .provenance
                .as_ref()
                .ok_or("missing provenance")?
                .resolution,
            RelationResolution::SyntaxOnly
        );

        Ok(())
    }

    #[test]
    fn python_and_rust_imports_are_unaffected_by_the_resolver_pipeline()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let mut graph = GraphBuilder.build(&root, &artifacts);
        let before = graph.relations.clone();

        let report = HybridResolverPipeline::default_pipeline().resolve(&mut graph);

        let specialized_hybrid_count = before
            .iter()
            .filter(|relation| {
                relation
                    .provenance
                    .as_ref()
                    .is_some_and(|provenance| provenance.resolver_strategy == "specialized-hybrid")
            })
            .count();
        assert!(
            specialized_hybrid_count > 0,
            "fixture must contain already-HybridResolved python/rust relations"
        );
        assert_eq!(
            graph.relations, before,
            "the resolver pipeline must never touch relations that are already HybridResolved"
        );
        assert_eq!(report.resolved, 0);

        Ok(())
    }
}
