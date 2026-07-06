//! Deep Python analysis: modules, imports, classes, functions, and heuristic
//! cross-artifact references.

use crate::domain::{
    Artifact, ArtifactId, Confidence, EvidenceRef, ModelExposurePolicy, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use tree_sitter::Node;

/// Deep Python analysis output for one artifact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PythonAnalysis {
    /// Dotted module path derived from the artifact path.
    pub module_path: String,
    /// True when this file is a package `__init__.py`.
    pub is_package_init: bool,
    /// `import` and `from ... import ...` statements.
    pub imports: Vec<PythonImport>,
    /// Module-level classes.
    pub classes: Vec<PythonClass>,
    /// Module-level functions.
    pub functions: Vec<PythonFunction>,
    /// Heuristic cross-artifact references (calls, env reads, subprocess,
    /// dynamic imports, ctypes, config/path literals).
    pub references: Vec<PythonReference>,
    /// True when the parse tree contains recovered syntax errors.
    pub has_syntax_errors: bool,
}

/// Import statement category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PythonImportKind {
    /// `import a.b`, optionally `as` aliased.
    Import,
    /// `from a.b import c`, optionally `as` aliased.
    ImportFrom,
}

/// One imported name, with its optional alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonImportName {
    /// Imported name as written.
    pub name: String,
    /// `as` alias, when present.
    pub alias: Option<String>,
}

/// One `import` or `from ... import ...` statement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonImport {
    /// Statement category.
    pub kind: PythonImportKind,
    /// From-clause dotted module, when present (`ImportFrom` only).
    pub module: Option<String>,
    /// Imported names.
    pub names: Vec<PythonImportName>,
    /// Leading-dot count for relative imports; 0 for absolute imports.
    pub relative_level: u32,
    /// Evidence for this statement.
    pub evidence: EvidenceRef,
}

/// Module-level or nested class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonClass {
    /// Class name.
    pub name: String,
    /// Base class expressions, as written.
    pub bases: Vec<String>,
    /// Decorator expressions, as written, without the leading `@`.
    pub decorators: Vec<String>,
    /// Docstring text, when the class body starts with a bare string.
    pub docstring: Option<String>,
    /// Methods declared directly in the class body.
    pub methods: Vec<PythonFunction>,
    /// Evidence for this class.
    pub evidence: EvidenceRef,
}

/// Module-level function or method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonFunction {
    /// Function or method name.
    pub name: String,
    /// True for `async def`.
    pub is_async: bool,
    /// Decorator expressions, as written, without the leading `@`.
    pub decorators: Vec<String>,
    /// Parameter names, in declaration order.
    pub parameters: Vec<String>,
    /// Return type annotation, as written.
    pub return_type: Option<String>,
    /// Docstring text, when the body starts with a bare string.
    pub docstring: Option<String>,
    /// Evidence for this function.
    pub evidence: EvidenceRef,
}

/// Heuristic cross-artifact reference category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PythonReferenceKind {
    /// Call to a function or class defined elsewhere in the same file.
    Call,
    /// `os.environ.get`/`os.getenv` environment variable read.
    EnvRead,
    /// `subprocess.*`/`os.system` command invocation.
    Subprocess,
    /// `importlib.import_module`/`__import__` dynamic import.
    DynamicImport,
    /// `ctypes` foreign-function usage.
    Ctypes,
    /// `open`/`Path` config or path reference.
    ConfigPath,
}

/// One heuristic reference extracted from a call expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonReference {
    /// Reference category.
    pub kind: PythonReferenceKind,
    /// Extracted literal value, or a best-effort raw expression when dynamic.
    pub value: String,
    /// Confidence in the extracted value.
    pub confidence: Confidence,
    /// Evidence for the call expression.
    pub evidence: EvidenceRef,
}

/// Tree-sitter-backed analyzer for Python source files.
///
/// Any syntax error anywhere in the file causes whole-file parsers
/// (`rustpython-parser`, `syn`) to yield nothing; see
/// `docs/dev/parser-spike-decisions.md`. Tree-sitter recovers a partial tree
/// instead, so `has_syntax_errors` on the result signals partial rather than
/// total data loss.
#[derive(Debug, Clone, Copy, Default)]
pub struct PythonAnalyzer;

impl PythonAnalyzer {
    /// Parses and analyzes a safe Python artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> PythonAnalysis {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return PythonAnalysis::default();
        }

        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .is_err()
        {
            return PythonAnalysis::default();
        }
        let Some(tree) = parser.parse(text, None) else {
            return PythonAnalysis::default();
        };

        build_python_analysis(artifact, tree.root_node(), text)
    }
}

fn build_python_analysis(artifact: &Artifact, root: Node, source: &str) -> PythonAnalysis {
    let (module_path, is_package_init) = module_path(artifact.path.as_str());
    let mut imports = Vec::new();
    let mut classes = Vec::new();
    let mut functions = Vec::new();
    let mut defined_names = HashSet::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        collect_top_level(
            child,
            artifact,
            source,
            &mut imports,
            &mut classes,
            &mut functions,
            &mut defined_names,
        );
    }

    let mut references = Vec::new();
    collect_references(root, artifact, source, &defined_names, &mut references);

    PythonAnalysis {
        module_path,
        is_package_init,
        imports,
        classes,
        functions,
        references,
        has_syntax_errors: root.has_error(),
    }
}

pub(crate) fn module_path(path: &str) -> (String, bool) {
    let trimmed = path.strip_suffix(".py").unwrap_or(path);
    if let Some(package) = trimmed.strip_suffix("/__init__") {
        return (package.replace('/', "."), true);
    }
    if trimmed == "__init__" {
        return (String::new(), true);
    }
    (trimmed.replace('/', "."), false)
}

fn collect_top_level(
    node: Node,
    artifact: &Artifact,
    source: &str,
    imports: &mut Vec<PythonImport>,
    classes: &mut Vec<PythonClass>,
    functions: &mut Vec<PythonFunction>,
    defined_names: &mut HashSet<String>,
) {
    match node.kind() {
        "import_statement" => imports.push(build_import(node, artifact, source)),
        "import_from_statement" => imports.push(build_import_from(node, artifact, source)),
        "future_import_statement" => imports.push(build_future_import(node, artifact, source)),
        "decorated_definition" => {
            let decorators = decorator_texts(node, source);
            let Some(definition) = node.child_by_field_name("definition") else {
                return;
            };
            match definition.kind() {
                "class_definition" => {
                    let class = build_class(definition, artifact, source, decorators);
                    defined_names.insert(class.name.clone());
                    classes.push(class);
                }
                "function_definition" => {
                    let function = build_function(definition, artifact, source, decorators);
                    defined_names.insert(function.name.clone());
                    functions.push(function);
                }
                _ => {}
            }
        }
        "class_definition" => {
            let class = build_class(node, artifact, source, Vec::new());
            defined_names.insert(class.name.clone());
            classes.push(class);
        }
        "function_definition" => {
            let function = build_function(node, artifact, source, Vec::new());
            defined_names.insert(function.name.clone());
            functions.push(function);
        }
        _ => {}
    }
}

fn build_class(
    node: Node,
    artifact: &Artifact,
    source: &str,
    decorators: Vec<String>,
) -> PythonClass {
    let name = field_text(node, "name", source)
        .unwrap_or_default()
        .to_owned();
    let bases = node
        .child_by_field_name("superclasses")
        .map(|args| base_names(args, source))
        .unwrap_or_default();
    let body = node.child_by_field_name("body");
    let docstring = body.and_then(|body| block_docstring(body, source));
    let methods = body
        .map(|body| {
            let mut methods = Vec::new();
            let mut cursor = body.walk();
            for child in body.children(&mut cursor) {
                collect_method(child, artifact, source, &mut methods);
            }
            methods
        })
        .unwrap_or_default();

    PythonClass {
        name,
        bases,
        decorators,
        docstring,
        methods,
        evidence: evidence(artifact, node),
    }
}

fn collect_method(
    node: Node,
    artifact: &Artifact,
    source: &str,
    methods: &mut Vec<PythonFunction>,
) {
    match node.kind() {
        "function_definition" => methods.push(build_function(node, artifact, source, Vec::new())),
        "decorated_definition" => {
            let decorators = decorator_texts(node, source);
            if let Some(definition) = node.child_by_field_name("definition")
                && definition.kind() == "function_definition"
            {
                methods.push(build_function(definition, artifact, source, decorators));
            }
        }
        _ => {}
    }
}

fn base_names(args: Node, source: &str) -> Vec<String> {
    let mut bases = Vec::new();
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if matches!(child.kind(), "identifier" | "attribute") {
            bases.push(node_text(child, source).to_owned());
        }
    }
    bases
}

fn build_function(
    node: Node,
    artifact: &Artifact,
    source: &str,
    decorators: Vec<String>,
) -> PythonFunction {
    let name = field_text(node, "name", source)
        .unwrap_or_default()
        .to_owned();
    let is_async = node.child(0).is_some_and(|child| child.kind() == "async");
    let parameters = node
        .child_by_field_name("parameters")
        .map(|params| parameter_names(params, source))
        .unwrap_or_default();
    let return_type = node
        .child_by_field_name("return_type")
        .map(|node| node_text(node, source).to_owned());
    let docstring = node
        .child_by_field_name("body")
        .and_then(|body| block_docstring(body, source));

    PythonFunction {
        name,
        is_async,
        decorators,
        parameters,
        return_type,
        docstring,
        evidence: evidence(artifact, node),
    }
}

fn parameter_names(params: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = params.walk();
    for child in params.children(&mut cursor) {
        match child.kind() {
            "identifier" => names.push(node_text(child, source).to_owned()),
            "typed_parameter"
            | "default_parameter"
            | "typed_default_parameter"
            | "list_splat_pattern"
            | "dictionary_splat_pattern" => {
                let name = child
                    .child_by_field_name("name")
                    .or_else(|| first_child_of_kind(child, "identifier"));
                if let Some(name) = name {
                    names.push(node_text(name, source).to_owned());
                }
            }
            _ => {}
        }
    }
    names
}

fn first_child_of_kind<'a>(node: Node<'a>, kind: &str) -> Option<Node<'a>> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == kind)
}

fn block_docstring(block: Node, source: &str) -> Option<String> {
    let mut cursor = block.walk();
    let first = block.children(&mut cursor).next()?;
    if first.kind() != "expression_statement" {
        return None;
    }
    let string_node = first.named_child(0)?;
    if string_node.kind() != "string" {
        return None;
    }
    string_literal_value(string_node, source)
}

fn decorator_texts(node: Node, source: &str) -> Vec<String> {
    let mut decorators = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            decorators.push(
                node_text(child, source)
                    .trim_start_matches('@')
                    .trim()
                    .to_owned(),
            );
        }
    }
    decorators
}

fn build_import(node: Node, artifact: &Artifact, source: &str) -> PythonImport {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_import_name(child, source, &mut names);
    }

    PythonImport {
        kind: PythonImportKind::Import,
        module: None,
        names,
        relative_level: 0,
        evidence: evidence(artifact, node),
    }
}

fn build_import_from(node: Node, artifact: &Artifact, source: &str) -> PythonImport {
    let module_name_node = node.child_by_field_name("module_name");
    let mut module = None;
    let mut relative_level = 0u32;

    if let Some(module_node) = module_name_node {
        match module_node.kind() {
            "relative_import" => {
                let mut cursor = module_node.walk();
                for child in module_node.children(&mut cursor) {
                    match child.kind() {
                        "import_prefix" => {
                            relative_level = node_text(child, source)
                                .chars()
                                .filter(|dot| *dot == '.')
                                .count() as u32;
                        }
                        "dotted_name" => module = Some(node_text(child, source).to_owned()),
                        _ => {}
                    }
                }
            }
            "dotted_name" => module = Some(node_text(module_node, source).to_owned()),
            _ => {}
        }
    }

    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if module_name_node.is_some_and(|module_node| module_node.id() == child.id()) {
            continue;
        }
        collect_import_name(child, source, &mut names);
    }

    PythonImport {
        kind: PythonImportKind::ImportFrom,
        module,
        names,
        relative_level,
        evidence: evidence(artifact, node),
    }
}

fn build_future_import(node: Node, artifact: &Artifact, source: &str) -> PythonImport {
    let mut names = Vec::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_import_name(child, source, &mut names);
    }

    PythonImport {
        kind: PythonImportKind::ImportFrom,
        module: Some("__future__".to_owned()),
        names,
        relative_level: 0,
        evidence: evidence(artifact, node),
    }
}

fn collect_import_name(node: Node, source: &str, names: &mut Vec<PythonImportName>) {
    match node.kind() {
        "dotted_name" => names.push(PythonImportName {
            name: node_text(node, source).to_owned(),
            alias: None,
        }),
        "aliased_import" => {
            let name = node
                .child_by_field_name("name")
                .map(|name| node_text(name, source).to_owned())
                .unwrap_or_default();
            let alias = node
                .child_by_field_name("alias")
                .map(|alias| node_text(alias, source).to_owned());
            names.push(PythonImportName { name, alias });
        }
        _ => {}
    }
}

fn collect_references(
    node: Node,
    artifact: &Artifact,
    source: &str,
    defined_names: &HashSet<String>,
    references: &mut Vec<PythonReference>,
) {
    if node.kind() == "call"
        && let Some(reference) = classify_call(node, artifact, source, defined_names)
    {
        references.push(reference);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_references(child, artifact, source, defined_names, references);
    }
}

// ponytail: callee classification matches literal dotted call text
// ("os.environ.get", "subprocess.run", ...) rather than resolving import
// aliases, so `import subprocess as sp; sp.run(...)` is not recognized.
// Upgrade to alias-aware resolution if that pattern shows up in real repos.
fn classify_call(
    node: Node,
    artifact: &Artifact,
    source: &str,
    defined_names: &HashSet<String>,
) -> Option<PythonReference> {
    let function = node.child_by_field_name("function")?;
    let callee = node_text(function, source);
    let first_arg = first_positional_argument(node);

    let (kind, value, confidence) = if defined_names.contains(callee) {
        (
            PythonReferenceKind::Call,
            callee.to_owned(),
            Confidence::High,
        )
    } else if matches!(callee, "os.environ.get" | "os.getenv") {
        literal_or_dynamic(first_arg, source, PythonReferenceKind::EnvRead)
    } else if matches!(
        callee,
        "subprocess.run"
            | "subprocess.call"
            | "subprocess.Popen"
            | "subprocess.check_call"
            | "subprocess.check_output"
            | "os.system"
    ) {
        literal_or_dynamic(first_arg, source, PythonReferenceKind::Subprocess)
    } else if matches!(callee, "importlib.import_module" | "__import__") {
        let value = first_arg
            .and_then(|arg| string_literal_value(arg, source))
            .unwrap_or_else(|| callee.to_owned());
        (PythonReferenceKind::DynamicImport, value, Confidence::Low)
    } else if callee.starts_with("ctypes.") {
        (
            PythonReferenceKind::Ctypes,
            callee.to_owned(),
            Confidence::Low,
        )
    } else if matches!(callee, "open" | "Path") {
        literal_or_dynamic(first_arg, source, PythonReferenceKind::ConfigPath)
    } else {
        return None;
    };

    Some(PythonReference {
        kind,
        value,
        confidence,
        evidence: evidence(artifact, node),
    })
}

fn first_positional_argument(call: Node) -> Option<Node> {
    let arguments = call.child_by_field_name("arguments")?;
    let mut cursor = arguments.walk();
    arguments.children(&mut cursor).find(|child| {
        !matches!(child.kind(), "(" | ")" | ",") && child.kind() != "keyword_argument"
    })
}

fn literal_or_dynamic(
    arg: Option<Node>,
    source: &str,
    kind: PythonReferenceKind,
) -> (PythonReferenceKind, String, Confidence) {
    let literal = arg
        .filter(|node| node.kind() == "string")
        .and_then(|node| string_literal_value(node, source));
    match literal {
        Some(value) => (kind, value, Confidence::High),
        None => {
            let raw = arg
                .map(|node| node_text(node, source).to_owned())
                .unwrap_or_else(|| "<dynamic>".to_owned());
            (kind, raw, Confidence::Low)
        }
    }
}

fn string_literal_value(node: Node, source: &str) -> Option<String> {
    let mut value = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_content" {
            value.push_str(node_text(child, source));
        }
    }
    Some(value)
}

fn field_text<'a>(node: Node<'a>, field: &str, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name(field)
        .map(|child| node_text(child, source))
}

fn node_text<'a>(node: Node<'a>, source: &'a str) -> &'a str {
    node.utf8_text(source.as_bytes()).unwrap_or("")
}

fn evidence(artifact: &Artifact, node: Node) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    let start = node.start_position().row as u32 + 1;
    let end = node.end_position().row as u32 + 1;
    match SourceSpan::new(start, end) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

#[cfg(test)]
mod tests {
    use super::{PythonAnalyzer, PythonImportKind, PythonReferenceKind};
    use crate::domain::{
        Artifact, ArtifactCategory, Confidence, ContentHash, ModelExposurePolicy, RepoPath,
        SupportTier, TextStatus,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn fixture_artifact(path: &str) -> Result<(Artifact, String), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let not_found = std::io::ErrorKind::NotFound;
        let artifact = artifacts
            .into_iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or(std::io::Error::new(not_found, path.to_owned()))?;
        let text = fs::read_to_string(root.join(path))?;
        Ok((artifact, text))
    }

    fn python_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::DeepLanguage,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_detected_format("python")
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn python_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("src/python_app/service.py")?;
        let analysis = PythonAnalyzer.analyze(&artifact, &text);

        assert_eq!(analysis.module_path, "src.python_app.service");
        assert!(!analysis.is_package_init);
        assert!(!analysis.has_syntax_errors);

        assert_eq!(
            snapshot_imports(&analysis),
            "\
ImportFrom:__future__:0:annotations
Import:-:0:json
Import:-:0:os
Import:-:0:subprocess
ImportFrom:pathlib:0:Path"
        );

        let class = &analysis.classes[0];
        assert_eq!(class.name, "RouteService");
        assert_eq!(
            class.docstring.as_deref(),
            Some("Loads route metadata and delegates expensive work to the Rust worker.")
        );
        assert_eq!(
            class
                .methods
                .iter()
                .map(|m| m.name.as_str())
                .collect::<Vec<_>>(),
            vec!["__init__", "load_settings", "bake_route"]
        );
        let init = &class.methods[0];
        assert_eq!(init.parameters, vec!["self", "config_path"]);
        assert_eq!(init.return_type.as_deref(), Some("None"));

        assert_eq!(analysis.functions[0].name, "run_worker");
        assert_eq!(analysis.functions[0].return_type.as_deref(), Some("str"));

        assert!(analysis.references.iter().any(|reference| {
            reference.kind == PythonReferenceKind::EnvRead
                && reference.value == "RIDGELINE_WORKER"
                && reference.confidence == Confidence::High
        }));
        assert!(analysis.references.iter().any(|reference| {
            reference.kind == PythonReferenceKind::Call
                && reference.value == "run_worker"
                && reference.confidence == Confidence::High
        }));

        Ok(())
    }

    #[test]
    fn python_fixture_package_init_has_relative_import() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("src/python_app/__init__.py")?;
        let analysis = PythonAnalyzer.analyze(&artifact, &text);

        assert_eq!(analysis.module_path, "src.python_app");
        assert!(analysis.is_package_init);
        let import = &analysis.imports[0];
        assert_eq!(import.kind, PythonImportKind::ImportFrom);
        assert_eq!(import.module.as_deref(), Some("service"));
        assert_eq!(import.relative_level, 1);
        assert_eq!(
            import
                .names
                .iter()
                .map(|name| name.name.as_str())
                .collect::<Vec<_>>(),
            vec!["RouteService", "run_worker"]
        );

        Ok(())
    }

    #[test]
    fn python_analyzer_extracts_decorators_async_inheritance_and_dynamic_refs()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = python_artifact("app/worker.py")?;
        let text = "\
import importlib
import json as j
from . import sibling
from ..pkg import other as o

@decorator_one
@decorator_two(arg=1)
class Base:
    pass

class Worker(Base, metaclass=Meta):
    @staticmethod
    async def run(self, path):
        importlib.import_module(dynamic_name)
        return await other()
";
        let analysis = PythonAnalyzer.analyze(&artifact, text);

        let base = analysis
            .classes
            .iter()
            .find(|class| class.name == "Base")
            .ok_or("Base class")?;
        assert_eq!(
            base.decorators,
            vec!["decorator_one", "decorator_two(arg=1)"]
        );

        let worker = analysis
            .classes
            .iter()
            .find(|class| class.name == "Worker")
            .ok_or("Worker class")?;
        assert_eq!(worker.bases, vec!["Base"]);
        let run = &worker.methods[0];
        assert!(run.is_async);
        assert_eq!(run.decorators, vec!["staticmethod"]);

        let dynamic_import = analysis
            .references
            .iter()
            .find(|reference| reference.kind == PythonReferenceKind::DynamicImport)
            .ok_or("dynamic import reference")?;
        assert_eq!(dynamic_import.confidence, Confidence::Low);

        let relative = &analysis.imports[3];
        assert_eq!(relative.relative_level, 2);
        assert_eq!(relative.module.as_deref(), Some("pkg"));

        Ok(())
    }

    #[test]
    fn python_analyzer_respects_policy_and_records_syntax_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let never = python_artifact("app/secret.py")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let binary = python_artifact("app/binary.py")?.with_text_status(TextStatus::Binary, None);
        let artifact = python_artifact("app/broken.py")?;
        let broken = "def broken(:\n    pass\n";

        assert_eq!(
            PythonAnalyzer.analyze(&never, "import os"),
            super::PythonAnalysis::default()
        );
        assert_eq!(
            PythonAnalyzer.analyze(&binary, "import os"),
            super::PythonAnalysis::default()
        );
        assert!(PythonAnalyzer.analyze(&artifact, broken).has_syntax_errors);

        Ok(())
    }

    fn snapshot_imports(analysis: &super::PythonAnalysis) -> String {
        analysis
            .imports
            .iter()
            .map(|import| {
                format!(
                    "{:?}:{}:{}:{}",
                    import.kind,
                    import.module.as_deref().unwrap_or("-"),
                    import.relative_level,
                    import
                        .names
                        .iter()
                        .map(|name| name.name.as_str())
                        .collect::<Vec<_>>()
                        .join("+")
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}
