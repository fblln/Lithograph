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

/// Extracts named TypeScript import bindings as `(exported, local)` pairs.
/// Namespace and default imports deliberately stay out of this small parser:
/// resolving those requires member/constructor semantics rather than the
/// direct named-call contract this resolver can prove safely.
pub(crate) fn extract_typescript_import_bindings(raw_text: &str) -> Vec<(String, String)> {
    let Some(after_import) = raw_text.trim().strip_prefix("import") else {
        return Vec::new();
    };
    let clause = after_import
        .split_once(" from ")
        .map_or(after_import, |(clause, _)| clause)
        .trim();
    let clause = clause.strip_prefix("type ").unwrap_or(clause);
    let Some(open) = clause.find('{') else {
        return Vec::new();
    };
    let Some(close) = clause[open + 1..].find('}') else {
        return Vec::new();
    };
    clause[open + 1..open + 1 + close]
        .split(',')
        .filter_map(|binding| {
            let binding = binding
                .trim()
                .strip_prefix("type ")
                .unwrap_or(binding.trim());
            let mut parts = binding.split_whitespace();
            let exported = parts.next()?;
            let local = match (parts.next(), parts.next()) {
                (Some("as"), Some(local)) => local,
                (None, None) => exported,
                _ => return None,
            };
            (!exported.is_empty() && !local.is_empty())
                .then(|| (exported.to_owned(), local.to_owned()))
        })
        .collect()
}

/// File extensions worth appending to a relative-import candidate path for
/// `language`, in preference order. Only languages whose import syntax is
/// itself extension-less (JS/TS's `from "./util"`) need this; the rest
/// return an empty slice and rely on the exact-path fallback.
fn candidate_extensions(language: &str) -> &'static [&'static str] {
    // Ordered by which source the language most likely means: a `.ts` file's
    // extensionless import is a sibling `.ts` far more often than a `.js`.
    match language {
        "typescript" => &[".ts", ".tsx", ".mts", ".cts", ".js", ".jsx", ".mjs", ".cjs"],
        "tsx" => &[".tsx", ".ts", ".mts", ".cts", ".jsx", ".js", ".mjs", ".cjs"],
        "javascript" => &[".js", ".jsx", ".mjs", ".cjs", ".ts", ".tsx"],
        _ => &[],
    }
}

/// Sources an import specifier's written extension may actually name.
///
/// TypeScript's ESM output keeps the specifier the author wrote, so
/// `import "./util.js"` in a `.ts` file refers to `util.ts` -- the `.js` is
/// what the *emitted* code will import, not what exists on disk. Without this
/// the import resolves to nothing on any repository following the convention
/// (LIT-45.1).
fn remapped_extensions(extension: &str) -> &'static [&'static str] {
    match extension {
        ".js" => &[".ts", ".tsx"],
        ".jsx" => &[".tsx"],
        ".mjs" => &[".mts"],
        ".cjs" => &[".cts"],
        _ => &[],
    }
}

/// Every path an import specifier could name, in the order they should win.
///
/// The order is the resolution rule, so it is spelled out rather than left to
/// the caller's loop:
///
/// 1. the exact path, so a real `util.js` beats a same-named `util.ts`;
/// 2. the extension remap above, for the TS ESM convention;
/// 3. the specifier plus each candidate extension, covering extensionless
///    imports and multi-dot names like `Foo.svelte.ts`;
/// 4. directory index files -- last, because a file always beats a directory
///    of the same name.
pub(crate) fn import_candidates(candidate: &str, language: &str) -> Vec<String> {
    let mut candidates = vec![candidate.to_owned()];

    if let Some(extension_start) = file_extension_start(candidate) {
        let (stem, extension) = candidate.split_at(extension_start);
        for remapped in remapped_extensions(extension) {
            candidates.push(format!("{stem}{remapped}"));
        }
    }

    for extension in candidate_extensions(language) {
        candidates.push(format!("{candidate}{extension}"));
    }

    for extension in candidate_extensions(language) {
        candidates.push(format!("{candidate}/index{extension}"));
    }

    candidates.dedup();
    candidates
}

/// Byte offset of the final `.` of `path`'s file name, when it has one.
///
/// Scoped to the file name so a dotted directory (`./.config/thing`) is not
/// mistaken for an extension.
fn file_extension_start(path: &str) -> Option<usize> {
    let name_start = path.rfind('/').map_or(0, |index| index + 1);
    let dot = path[name_start..].rfind('.')? + name_start;
    (dot > name_start).then_some(dot)
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
pub(crate) fn resolve_relative_path(source_dir: &Path, reference: &str) -> String {
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
            for path in import_candidates(&candidate, language) {
                if let Some(target) = context.artifacts_by_path.get(path.as_str()) {
                    return Some(ResolvedTarget {
                        target: (*target).clone(),
                        confidence: Confidence::High,
                    });
                }
            }
        }

        // LIT-45.2: a non-relative specifier may be a tsconfig path alias for
        // a first-party file (`@app/util` -> `src/util.ts`). Tried after the
        // package map above, so a real dependency still wins, and before the
        // language fall-throughs below. An alias whose target does not exist
        // simply finds no artifact and falls through (AC4).
        if matches!(language, "typescript" | "tsx" | "javascript")
            && !context.ts_aliases.is_empty()
            && let Some(source_path) = relation.source.as_str().strip_prefix("artifact:")
        {
            for aliased in context.ts_aliases.resolve(source_path, &reference) {
                for path in import_candidates(&aliased, language) {
                    if let Some(target) = context.artifacts_by_path.get(path.as_str()) {
                        return Some(ResolvedTarget {
                            target: (*target).clone(),
                            confidence: Confidence::High,
                        });
                    }
                }
            }
        }

        // LIT-44.2: npm subpath imports belong to the package declared by
        // their bare dependency root (`react-dom/client` -> `react-dom`,
        // `@scope/pkg/sub` -> `@scope/pkg`). This runs after relative and
        // tsconfig-alias resolution so a first-party path keeps its existing
        // precedence, and matches only a package node the manifest already
        // created -- an undeclared name still falls through to Unresolved.
        if matches!(language, "typescript" | "tsx" | "javascript")
            && let Some(root) = typescript_dependency_root(&reference)
            && let Some(target) = context.packages_by_name.get(root)
        {
            return Some(ResolvedTarget {
                target: (*target).clone(),
                confidence: Confidence::High,
            });
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

/// Returns the npm dependency name at the root of a bare TS/JS specifier.
///
/// Plain packages use their first segment; scoped packages require both the
/// scope and package segment. Relative/absolute paths and malformed scopes
/// are not package names and deliberately produce no candidate.
fn typescript_dependency_root(specifier: &str) -> Option<&str> {
    if specifier.is_empty()
        || specifier.starts_with('.')
        || specifier.starts_with('/')
        || specifier.starts_with('\\')
    {
        return None;
    }
    if specifier.starts_with('@') {
        let mut separators = specifier.match_indices('/');
        let (first_end, _) = separators.next()?;
        if first_end <= 1 {
            return None;
        }
        let end = separators
            .next()
            .map_or(specifier.len(), |(index, _)| index);
        (end > first_end + 1).then_some(&specifier[..end])
    } else {
        let end = specifier.find('/').unwrap_or(specifier.len());
        (end > 0).then_some(&specifier[..end])
    }
}

#[cfg(test)]
mod tests {
    use super::{
        extract_import_reference, extract_typescript_import_bindings, typescript_dependency_root,
    };

    #[test]
    fn typescript_dependency_roots_preserve_scopes_and_strip_only_subpaths() {
        assert_eq!(typescript_dependency_root("react"), Some("react"));
        assert_eq!(
            typescript_dependency_root("react-dom/client"),
            Some("react-dom")
        );
        assert_eq!(typescript_dependency_root("@scope/pkg"), Some("@scope/pkg"));
        assert_eq!(
            typescript_dependency_root("@scope/pkg/sub/path"),
            Some("@scope/pkg")
        );
        assert_eq!(typescript_dependency_root("./local"), None);
        assert_eq!(typescript_dependency_root("../local"), None);
        assert_eq!(typescript_dependency_root("/absolute"), None);
        assert_eq!(typescript_dependency_root("@scope"), None);
        assert_eq!(typescript_dependency_root("@/pkg"), None);
    }

    #[test]
    fn extracts_named_typescript_import_bindings_without_guessing_defaults() {
        assert_eq!(
            extract_typescript_import_bindings(
                "import type { Service, start as run, type Config } from \"./service\";"
            ),
            vec![
                ("Service".to_owned(), "Service".to_owned()),
                ("start".to_owned(), "run".to_owned()),
                ("Config".to_owned(), "Config".to_owned()),
            ]
        );
        assert!(extract_typescript_import_bindings("import App from \"./App\";").is_empty());
    }

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

    /// LIT-23.1: a multi-line `import type { ... } from "module"` must
    /// extract the module specifier, not the raw multi-line statement text
    /// verbatim -- found live on an external repository's TypeScript files,
    /// where this produced thousands of Unresolved nodes whose "value" was
    /// an entire raw import statement (embedded newlines and all) instead
    /// of a package or module name.
    #[test]
    fn extracts_specifier_from_multi_line_and_type_only_imports() {
        assert_eq!(
            extract_import_reference(
                "typescript",
                "import type {\n  CameraRig,\n  RouteSummary,\n} from \"./types\";"
            ),
            Some("./types".to_owned())
        );
        assert_eq!(
            extract_import_reference(
                "typescript",
                "import {\n  useEffect,\n  useState,\n} from \"react\";"
            ),
            Some("react".to_owned())
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

/// End-to-end coverage (LIT-22.3.2 AC2/AC3/AC4; wired into `GraphBuilder`
/// itself by LIT-23.1): builds a real graph from an isolated repo per
/// language family (not the shared polyglot fixture, to avoid
/// golden-snapshot churn). `GraphBuilder::build`/`build_with_cache` already
/// run the default resolver pipeline internally as their last step (the
/// same way `detect_near_clones` does), so these assert directly against
/// its output rather than calling the pipeline a second time -- a relation
/// resolved during `build()` is already `HybridResolved` by the time a
/// caller sees the graph, so a second `resolve()` call would always be a
/// no-op.
#[cfg(test)]
mod pipeline_tests {
    use crate::domain::Confidence;
    use crate::graph::{GraphBuilder, GraphNode, RelationKind, RelationResolution};
    use crate::inventory::{RepositoryWalker, WalkOptions};

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
        let graph = GraphBuilder.build(temp.path(), &artifacts);

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

    /// LIT-44.2 AC1-AC3: manifest-declared npm roots classify plain, scoped,
    /// and subpath imports onto the existing package nodes. An undeclared
    /// root remains Unresolved, and repeated subpaths do not duplicate nodes.
    #[test]
    fn typescript_bare_imports_resolve_only_to_declared_dependency_roots()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(
            temp.path().join("package.json"),
            r#"{
  "name": "app",
  "dependencies": {
    "react": "^18",
    "react-dom": "^18",
    "@scope/pkg": "^1"
  }
}"#,
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { useState } from 'react';\n\
             import client from 'react-dom/client';\n\
             import server from 'react-dom/server';\n\
             import scoped from '@scope/pkg/sub';\n\
             import missing from 'left-pad/sub';\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let targets: Vec<_> = graph
            .relations
            .iter()
            .filter(|relation| {
                relation.kind == RelationKind::Imports
                    && relation.source.as_str() == "artifact:src/app.ts"
            })
            .map(|relation| relation.target.as_str())
            .collect();

        assert!(targets.contains(&"package:react"));
        assert_eq!(
            targets
                .iter()
                .filter(|target| **target == "package:react-dom")
                .count(),
            2,
            "both subpath relations reuse one manifest-derived package node",
        );
        assert!(targets.contains(&"package:@scope/pkg"));
        assert!(graph.nodes.iter().any(|node| matches!(
            node,
            GraphNode::Unresolved(unresolved) if unresolved.value.contains("left-pad/sub")
        )));
        assert_eq!(
            graph
                .nodes
                .iter()
                .filter(|node| node.id().as_str() == "package:react-dom")
                .count(),
            1,
        );

        Ok(())
    }

    /// A tsconfig alias is a first-party path even when its prefix is also a
    /// declared npm dependency; the existing alias precedence is preserved.
    #[test]
    fn typescript_alias_target_wins_before_dependency_root_classification()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/core"))?;
        std::fs::write(
            temp.path().join("package.json"),
            r#"{"name":"app","dependencies":{"@app/core":"1"}}"#,
        )?;
        std::fs::write(
            temp.path().join("tsconfig.json"),
            r#"{"compilerOptions":{"baseUrl":".","paths":{"@app/*":["src/*"]}}}"#,
        )?;
        std::fs::write(
            temp.path().join("src/core/util.ts"),
            "export const util = 1;\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { util } from '@app/core/util';\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let relation = resolved_target(&graph, "src/app.ts").ok_or("missing import relation")?;
        assert_eq!(relation.target.as_str(), "artifact:src/core/util.ts");

        Ok(())
    }

    /// LIT-23.1 end-to-end: a real multi-line, type-only import resolves to
    /// the local artifact its relative specifier names, through the full
    /// `GraphBuilder` pipeline -- reproducing the exact scenario found on an
    /// external repository's TypeScript files.
    #[test]
    fn multi_line_type_only_import_resolves_to_local_artifact()
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

        let relation = resolved_target(&graph, "src/app.ts").ok_or("missing import relation")?;
        assert_eq!(relation.target.as_str(), "artifact:src/types.ts");
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

    /// LIT-45.1 AC1: TypeScript's ESM output keeps the specifier the author
    /// wrote, so `./util.js` in a `.ts` file names `util.ts` on disk. Before
    /// the remap this resolved to nothing on every repository following the
    /// convention.
    #[test]
    fn js_suffixed_specifier_resolves_to_its_typescript_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(temp.path().join("src/util.ts"), "export const x = 1;\n")?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { x } from \"./util.js\";\nexport const y = x;\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let relation = resolved_target(&graph, "src/app.ts").ok_or("missing import relation")?;
        assert_eq!(relation.target.as_str(), "artifact:src/util.ts");

        Ok(())
    }

    /// LIT-45.1: a specifier naming a file that really exists must win over
    /// the remap -- the remap exists for a missing `.js`, not to redirect a
    /// present one.
    #[test]
    fn an_existing_js_file_wins_over_its_typescript_namesake()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src"))?;
        std::fs::write(temp.path().join("src/util.js"), "export const x = 1;\n")?;
        std::fs::write(temp.path().join("src/util.ts"), "export const x = 2;\n")?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { x } from \"./util.js\";\nexport const y = x;\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let relation = resolved_target(&graph, "src/app.ts").ok_or("missing import relation")?;
        assert_eq!(relation.target.as_str(), "artifact:src/util.js");

        Ok(())
    }

    /// LIT-45.1 AC3: a directory import resolves to its index, but only after
    /// every file candidate fails -- a file always beats a directory of the
    /// same name.
    #[test]
    fn directory_imports_resolve_to_index_only_when_no_file_matches()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("src/models"))?;
        std::fs::write(
            temp.path().join("src/models/index.ts"),
            "export const model = 1;\n",
        )?;
        std::fs::write(
            temp.path().join("src/app.ts"),
            "import { model } from \"./models\";\nexport const m = model;\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        assert_eq!(
            resolved_target(&graph, "src/app.ts")
                .ok_or("missing import relation")?
                .target
                .as_str(),
            "artifact:src/models/index.ts",
        );

        // With a sibling `models.ts` present, the file wins.
        std::fs::write(
            temp.path().join("src/models.ts"),
            "export const model = 2;\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        assert_eq!(
            resolved_target(&graph, "src/app.ts")
                .ok_or("missing import relation")?
                .target
                .as_str(),
            "artifact:src/models.ts",
        );

        Ok(())
    }

    /// LIT-45.1 AC2: the candidate order is the resolution rule, so it is
    /// asserted directly rather than inferred from which file happens to win.
    #[test]
    fn import_candidate_order_is_documented_and_deterministic() {
        let candidates = super::import_candidates("src/util.js", "typescript");

        assert_eq!(
            candidates[0], "src/util.js",
            "the exact path is tried first"
        );
        assert_eq!(candidates[1], "src/util.ts", "then the ESM remap");
        assert_eq!(candidates[2], "src/util.tsx");
        assert!(
            candidates.iter().position(|path| path == "src/util.js.ts")
                > candidates.iter().position(|path| path == "src/util.ts"),
            "appending to a specifier that already has an extension must not \
             outrank remapping it",
        );
        assert!(
            candidates
                .iter()
                .position(|path| path.starts_with("src/util.js/index"))
                > candidates.iter().position(|path| path == "src/util.js.ts"),
            "directory indexes are the last resort",
        );
        assert_eq!(
            candidates,
            super::import_candidates("src/util.js", "typescript"),
            "the same specifier must always yield the same order",
        );

        // A dotted directory is not an extension.
        assert_eq!(
            super::import_candidates("src/.config/thing", "typescript")[1],
            "src/.config/thing.ts",
            "a leading-dot directory must not be read as a file extension",
        );
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
        let graph = GraphBuilder.build(temp.path(), &artifacts);

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
        let graph = GraphBuilder.build(temp.path(), &artifacts);

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
        let graph = GraphBuilder.build(&root, &artifacts);

        let specialized_hybrid_count = graph
            .relations
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
            "fixture must contain HybridResolved python/rust relations from their own \
             specialized analyzers, untouched by the generic import resolver pipeline"
        );

        Ok(())
    }
}
