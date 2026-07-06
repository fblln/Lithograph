//! Deep Rust source analysis: modules, `use` declarations, structs, enums,
//! traits, functions, impls, and macro invocations.

use crate::domain::{
    Artifact, ArtifactId, Confidence, EvidenceRef, ModelExposurePolicy, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tree_sitter::Node;

/// Deep Rust analysis output for one `.rs` artifact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RustAnalysis {
    /// `::`-joined module path derived from the artifact path.
    pub module_path: String,
    /// True when this file is a crate root (`lib.rs`/`main.rs`).
    pub is_crate_root: bool,
    /// `use` declarations, flattened from grouped/aliased/wildcard forms.
    pub uses: Vec<RustUse>,
    /// `mod` declarations.
    pub mod_declarations: Vec<RustModDeclaration>,
    /// Top-level structs.
    pub structs: Vec<RustItem>,
    /// Top-level enums.
    pub enums: Vec<RustItem>,
    /// Top-level traits.
    pub traits: Vec<RustTrait>,
    /// Top-level functions.
    pub functions: Vec<RustFunction>,
    /// `impl` and trait `impl` blocks.
    pub impls: Vec<RustImpl>,
    /// Macro invocations anywhere in the file.
    pub macro_invocations: Vec<RustMacroInvocation>,
    /// Heuristic cross-artifact references (env reads, subprocess spawns).
    pub references: Vec<RustReference>,
    /// True when the parse tree contains recovered syntax errors.
    pub has_syntax_errors: bool,
}

/// One flattened `use` path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustUse {
    /// `::`-joined imported path, or `path::*` for wildcard imports.
    pub path: String,
    /// `as` alias, when present.
    pub alias: Option<String>,
    /// Evidence for the `use` declaration.
    pub evidence: EvidenceRef,
}

/// One `mod` declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustModDeclaration {
    /// Module name.
    pub name: String,
    /// True for `mod foo { ... }`; false for `mod foo;` (external file).
    pub is_inline: bool,
    /// Evidence for the `mod` declaration.
    pub evidence: EvidenceRef,
}

/// Struct or enum declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustItem {
    /// Item name.
    pub name: String,
    /// Attribute expressions, as written, without the `#[` `]` wrapper.
    pub attributes: Vec<String>,
    /// Doc comment text, from consecutive preceding `///` lines.
    pub doc: Option<String>,
    /// Evidence for this item.
    pub evidence: EvidenceRef,
}

/// Trait declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustTrait {
    /// Trait name.
    pub name: String,
    /// Attribute expressions, as written, without the `#[` `]` wrapper.
    pub attributes: Vec<String>,
    /// Doc comment text, from consecutive preceding `///` lines.
    pub doc: Option<String>,
    /// Method names declared or defaulted in the trait body.
    pub methods: Vec<String>,
    /// Evidence for this trait.
    pub evidence: EvidenceRef,
}

/// Top-level function declaration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustFunction {
    /// Function name.
    pub name: String,
    /// Parameter names, in declaration order (`self` included when present).
    pub parameters: Vec<String>,
    /// Return type, as written.
    pub return_type: Option<String>,
    /// Attribute expressions, as written, without the `#[` `]` wrapper.
    pub attributes: Vec<String>,
    /// Doc comment text, from consecutive preceding `///` lines.
    pub doc: Option<String>,
    /// Evidence for this function.
    pub evidence: EvidenceRef,
}

/// `impl` or trait `impl` block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustImpl {
    /// Implementing type, as written.
    pub target_type: String,
    /// Implemented trait, when this is a trait impl.
    pub trait_name: Option<String>,
    /// Method names declared in the impl body.
    pub methods: Vec<String>,
    /// Evidence for this impl block.
    pub evidence: EvidenceRef,
}

/// One macro invocation, e.g. `format!(...)`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustMacroInvocation {
    /// Macro name, without the trailing `!`.
    pub name: String,
    /// Evidence for the invocation.
    pub evidence: EvidenceRef,
}

/// Heuristic cross-artifact reference category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RustReferenceKind {
    /// `std::env::var`/`std::env::var_os` environment variable read.
    EnvRead,
    /// `std::process::Command::new` command invocation.
    Subprocess,
}

/// One heuristic reference extracted from a call expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RustReference {
    /// Reference category.
    pub kind: RustReferenceKind,
    /// Extracted literal value, or a best-effort raw expression when dynamic.
    pub value: String,
    /// Confidence in the extracted value.
    pub confidence: Confidence,
    /// Evidence for the call expression.
    pub evidence: EvidenceRef,
}

/// Tree-sitter-backed analyzer for Rust source files.
///
/// Any syntax error anywhere in the file causes whole-file parsers (`syn`) to
/// yield nothing; see `docs/dev/parser-spike-decisions.md`. Tree-sitter
/// recovers a partial tree instead, so `has_syntax_errors` on the result
/// signals partial rather than total data loss.
#[derive(Debug, Clone, Copy, Default)]
pub struct RustAnalyzer;

impl RustAnalyzer {
    /// Parses and analyzes a safe Rust artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> RustAnalysis {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return RustAnalysis::default();
        }

        let mut parser = tree_sitter::Parser::new();
        if parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .is_err()
        {
            return RustAnalysis::default();
        }
        let Some(tree) = parser.parse(text, None) else {
            return RustAnalysis::default();
        };

        build_rust_analysis(artifact, tree.root_node(), text)
    }
}

fn build_rust_analysis(artifact: &Artifact, root: Node, source: &str) -> RustAnalysis {
    let module_path = module_path(artifact.path.as_str());
    let is_crate_root = matches!(basename(artifact.path.as_str()), "lib.rs" | "main.rs");

    let mut uses = Vec::new();
    let mut mod_declarations = Vec::new();
    let mut structs = Vec::new();
    let mut enums = Vec::new();
    let mut traits = Vec::new();
    let mut functions = Vec::new();
    let mut impls = Vec::new();
    let mut pending_attributes: Vec<String> = Vec::new();
    let mut pending_doc: Vec<String> = Vec::new();

    let mut cursor = root.walk();
    for child in root.children(&mut cursor) {
        match child.kind() {
            "attribute_item" => pending_attributes.push(attribute_text(child, source)),
            "line_comment" => {
                if let Some(doc) = outer_doc_text(child, source) {
                    pending_doc.push(doc);
                }
            }
            "use_declaration" => {
                collect_uses(child, artifact, source, &mut uses);
                pending_attributes.clear();
                pending_doc.clear();
            }
            "mod_item" => {
                mod_declarations.push(build_mod(child, artifact, source));
                pending_attributes.clear();
                pending_doc.clear();
            }
            "struct_item" => structs.push(build_simple_item(
                child,
                artifact,
                source,
                take_attributes(&mut pending_attributes),
                take_doc(&mut pending_doc),
            )),
            "enum_item" => enums.push(build_simple_item(
                child,
                artifact,
                source,
                take_attributes(&mut pending_attributes),
                take_doc(&mut pending_doc),
            )),
            "trait_item" => traits.push(build_trait(
                child,
                artifact,
                source,
                take_attributes(&mut pending_attributes),
                take_doc(&mut pending_doc),
            )),
            "function_item" => functions.push(build_rust_function(
                child,
                artifact,
                source,
                take_attributes(&mut pending_attributes),
                take_doc(&mut pending_doc),
            )),
            "impl_item" => {
                impls.push(build_impl(child, artifact, source));
                pending_attributes.clear();
                pending_doc.clear();
            }
            _ => {}
        }
    }

    let mut macro_invocations = Vec::new();
    collect_macro_invocations(root, artifact, source, &mut macro_invocations);

    let aliases = build_use_alias_map(&uses);
    let mut references = Vec::new();
    collect_rust_references(root, artifact, source, &aliases, &mut references);

    RustAnalysis {
        module_path,
        is_crate_root,
        uses,
        mod_declarations,
        structs,
        enums,
        traits,
        functions,
        impls,
        macro_invocations,
        references,
        has_syntax_errors: root.has_error(),
    }
}

fn take_attributes(pending: &mut Vec<String>) -> Vec<String> {
    std::mem::take(pending)
}

fn take_doc(pending: &mut Vec<String>) -> Option<String> {
    if pending.is_empty() {
        return None;
    }
    Some(std::mem::take(pending).join("\n"))
}

// ponytail: module path is a plain slash-to-`::` mapping of the artifact
// path with the crate-root filename stripped; it does not know where the
// crate root actually lives (that requires Cargo.toml/cargo_metadata
// context from RustWorkspaceAnalyzer). Upgrade by combining the two once
// the graph builder correlates files with their owning package.
pub(crate) fn module_path(path: &str) -> String {
    let trimmed = path.strip_suffix(".rs").unwrap_or(path);
    let trimmed = trimmed
        .strip_suffix("/lib")
        .or_else(|| trimmed.strip_suffix("/main"))
        .or_else(|| trimmed.strip_suffix("/mod"))
        .unwrap_or(trimmed);
    let trimmed = match trimmed {
        "lib" | "main" | "mod" => "",
        other => other,
    };
    trimmed.replace('/', "::")
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn attribute_text(node: Node, source: &str) -> String {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "attribute")
        .map(|attribute| node_text(attribute, source).to_owned())
        .unwrap_or_else(|| node_text(node, source).to_owned())
}

fn outer_doc_text(node: Node, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    let is_outer = node
        .children(&mut cursor)
        .any(|child| child.kind() == "outer_doc_comment_marker");
    if !is_outer {
        return None;
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "doc_comment")
        .map(|doc| node_text(doc, source).trim().to_owned())
}

fn build_mod(node: Node, artifact: &Artifact, source: &str) -> RustModDeclaration {
    RustModDeclaration {
        name: field_text(node, "name", source)
            .unwrap_or_default()
            .to_owned(),
        is_inline: node.child_by_field_name("body").is_some(),
        evidence: evidence(artifact, node),
    }
}

fn build_simple_item(
    node: Node,
    artifact: &Artifact,
    source: &str,
    attributes: Vec<String>,
    doc: Option<String>,
) -> RustItem {
    RustItem {
        name: field_text(node, "name", source)
            .unwrap_or_default()
            .to_owned(),
        attributes,
        doc,
        evidence: evidence(artifact, node),
    }
}

fn build_trait(
    node: Node,
    artifact: &Artifact,
    source: &str,
    attributes: Vec<String>,
    doc: Option<String>,
) -> RustTrait {
    let methods = node
        .child_by_field_name("body")
        .map(|body| item_names(body, source, &["function_item", "function_signature_item"]))
        .unwrap_or_default();

    RustTrait {
        name: field_text(node, "name", source)
            .unwrap_or_default()
            .to_owned(),
        attributes,
        doc,
        methods,
        evidence: evidence(artifact, node),
    }
}

fn build_rust_function(
    node: Node,
    artifact: &Artifact,
    source: &str,
    attributes: Vec<String>,
    doc: Option<String>,
) -> RustFunction {
    let parameters = node
        .child_by_field_name("parameters")
        .map(|params| rust_parameter_names(params, source))
        .unwrap_or_default();
    let return_type = node
        .child_by_field_name("return_type")
        .map(|node| node_text(node, source).to_owned());

    RustFunction {
        name: field_text(node, "name", source)
            .unwrap_or_default()
            .to_owned(),
        parameters,
        return_type,
        attributes,
        doc,
        evidence: evidence(artifact, node),
    }
}

fn rust_parameter_names(params: Node, source: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut cursor = params.walk();
    for child in params.children(&mut cursor) {
        match child.kind() {
            "self_parameter" => names.push("self".to_owned()),
            "parameter" => {
                if let Some(pattern) = child.child_by_field_name("pattern") {
                    names.push(node_text(pattern, source).to_owned());
                }
            }
            _ => {}
        }
    }
    names
}

fn build_impl(node: Node, artifact: &Artifact, source: &str) -> RustImpl {
    let target_type = field_text(node, "type", source)
        .unwrap_or_default()
        .to_owned();
    let trait_name = field_text(node, "trait", source).map(str::to_owned);
    let methods = node
        .child_by_field_name("body")
        .map(|body| item_names(body, source, &["function_item"]))
        .unwrap_or_default();

    RustImpl {
        target_type,
        trait_name,
        methods,
        evidence: evidence(artifact, node),
    }
}

fn item_names(body: Node, source: &str, kinds: &[&str]) -> Vec<String> {
    let mut cursor = body.walk();
    body.children(&mut cursor)
        .filter(|child| kinds.contains(&child.kind()))
        .filter_map(|child| field_text(child, "name", source).map(str::to_owned))
        .collect()
}

fn collect_uses(node: Node, artifact: &Artifact, source: &str, uses: &mut Vec<RustUse>) {
    let Some(argument) = node.child_by_field_name("argument") else {
        return;
    };
    let mut paths = Vec::new();
    collect_use_paths(argument, "", source, &mut paths);
    for (path, alias) in paths {
        uses.push(RustUse {
            path,
            alias,
            evidence: evidence(artifact, node),
        });
    }
}

fn collect_use_paths(
    node: Node,
    prefix: &str,
    source: &str,
    out: &mut Vec<(String, Option<String>)>,
) {
    match node.kind() {
        "use_as_clause" => {
            let path = node
                .child_by_field_name("path")
                .map(|path| node_text(path, source).to_owned())
                .unwrap_or_default();
            let alias = node
                .child_by_field_name("alias")
                .map(|alias| node_text(alias, source).to_owned());
            out.push((join_path(prefix, &path), alias));
        }
        "scoped_use_list" => {
            let path = node
                .child_by_field_name("path")
                .map(|path| node_text(path, source).to_owned())
                .unwrap_or_default();
            let new_prefix = join_path(prefix, &path);
            if let Some(list) = node.child_by_field_name("list") {
                collect_use_paths(list, &new_prefix, source, out);
            }
        }
        "use_list" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if !matches!(child.kind(), "{" | "}" | ",") {
                    collect_use_paths(child, prefix, source, out);
                }
            }
        }
        "use_wildcard" => {
            let mut cursor = node.walk();
            let path = node
                .children(&mut cursor)
                .find(|child| !matches!(child.kind(), "::" | "*"))
                .map(|child| node_text(child, source).to_owned())
                .unwrap_or_default();
            out.push((format!("{}::*", join_path(prefix, &path)), None));
        }
        _ => out.push((join_path(prefix, node_text(node, source)), None)),
    }
}

fn join_path(prefix: &str, suffix: &str) -> String {
    if prefix.is_empty() {
        suffix.to_owned()
    } else {
        format!("{prefix}::{suffix}")
    }
}

fn collect_macro_invocations(
    node: Node,
    artifact: &Artifact,
    source: &str,
    out: &mut Vec<RustMacroInvocation>,
) {
    if node.kind() == "macro_invocation"
        && let Some(macro_name) = field_text(node, "macro", source)
    {
        out.push(RustMacroInvocation {
            name: macro_name.to_owned(),
            evidence: evidence(artifact, node),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_macro_invocations(child, artifact, source, out);
    }
}

/// Maps a locally-bound `use` name to the canonical path it stands for, so
/// `use std::env; env::var(...)` lets a later call resolve the same as an
/// unaliased `std::env::var(...)` call. The bound name is the `as` alias
/// when present, otherwise the path's last segment (wildcard imports bind no
/// single name and are skipped).
fn build_use_alias_map(uses: &[RustUse]) -> HashMap<String, String> {
    let mut aliases = HashMap::new();
    for use_ in uses {
        let bound_name = use_.alias.clone().or_else(|| {
            let last = use_.path.rsplit("::").next()?;
            (last != "*").then(|| last.to_owned())
        });
        if let Some(bound_name) = bound_name {
            aliases.insert(bound_name, use_.path.clone());
        }
    }
    aliases
}

/// Rewrites the leading bound identifier of a dotted callee text through the
/// alias map, e.g. `"env::var"` with `env -> std::env` becomes
/// `"std::env::var"`. Leaves unaliased callees untouched.
fn canonicalize_rust_callee(callee: &str, aliases: &HashMap<String, String>) -> String {
    match callee.split_once("::") {
        Some((head, rest)) => match aliases.get(head) {
            Some(canonical) => format!("{canonical}::{rest}"),
            None => callee.to_owned(),
        },
        None => aliases
            .get(callee)
            .cloned()
            .unwrap_or_else(|| callee.to_owned()),
    }
}

fn collect_rust_references(
    node: Node,
    artifact: &Artifact,
    source: &str,
    aliases: &HashMap<String, String>,
    references: &mut Vec<RustReference>,
) {
    if node.kind() == "call_expression"
        && let Some(reference) = classify_rust_call(node, artifact, source, aliases)
    {
        references.push(reference);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_rust_references(child, artifact, source, aliases, references);
    }
}

fn classify_rust_call(
    node: Node,
    artifact: &Artifact,
    source: &str,
    aliases: &HashMap<String, String>,
) -> Option<RustReference> {
    let function = node.child_by_field_name("function")?;
    let callee = node_text(function, source);
    let canonical = canonicalize_rust_callee(callee, aliases);
    let first_arg = node
        .child_by_field_name("arguments")
        .and_then(|arguments| arguments.named_child(0));

    let (kind, value, confidence) =
        if matches!(canonical.as_str(), "std::env::var" | "std::env::var_os") {
            rust_literal_or_dynamic(first_arg, source, RustReferenceKind::EnvRead)
        } else if canonical == "std::process::Command::new" {
            rust_literal_or_dynamic(first_arg, source, RustReferenceKind::Subprocess)
        } else {
            return None;
        };

    Some(RustReference {
        kind,
        value,
        confidence,
        evidence: evidence(artifact, node),
    })
}

fn rust_literal_or_dynamic(
    arg: Option<Node>,
    source: &str,
    kind: RustReferenceKind,
) -> (RustReferenceKind, String, Confidence) {
    let literal = arg
        .filter(|node| node.kind() == "string_literal")
        .map(|node| rust_string_literal_value(node, source));
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

fn rust_string_literal_value(node: Node, source: &str) -> String {
    let mut value = String::new();
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string_content" {
            value.push_str(node_text(child, source));
        }
    }
    value
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
    use super::RustAnalyzer;
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
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

    fn rust_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::DeepLanguage,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_detected_format("rust")
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn rust_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("rust/src/lib.rs")?;
        let analysis = RustAnalyzer.analyze(&artifact, &text);

        assert!(analysis.is_crate_root);
        assert!(!analysis.has_syntax_errors);
        assert_eq!(analysis.uses[0].path, "std::env");

        let trait_item = &analysis.traits[0];
        assert_eq!(trait_item.name, "RouteBake");
        assert_eq!(trait_item.methods, vec!["bake"]);

        let struct_item = &analysis.structs[0];
        assert_eq!(struct_item.name, "RouteBaker");

        assert_eq!(analysis.impls.len(), 2);
        let inherent_impl = analysis
            .impls
            .iter()
            .find(|imp| imp.trait_name.is_none())
            .ok_or("inherent impl")?;
        assert_eq!(inherent_impl.target_type, "RouteBaker");
        assert_eq!(inherent_impl.methods, vec!["from_env"]);
        let trait_impl = analysis
            .impls
            .iter()
            .find(|imp| imp.trait_name.is_some())
            .ok_or("trait impl")?;
        assert_eq!(trait_impl.trait_name.as_deref(), Some("RouteBake"));
        assert_eq!(trait_impl.target_type, "RouteBaker");

        assert_eq!(analysis.functions[0].name, "bake_route");
        assert!(
            analysis
                .macro_invocations
                .iter()
                .any(|invocation| invocation.name == "format")
        );

        // `use std::env;` + `env::var(...)` resolves through the use-alias
        // map the same as a fully-qualified `std::env::var(...)` call would.
        assert!(analysis.references.iter().any(|reference| {
            reference.kind == super::RustReferenceKind::EnvRead
                && reference.value == "RIDGELINE_CACHE_DIR"
                && reference.confidence == crate::domain::Confidence::High
        }));

        Ok(())
    }

    #[test]
    fn rust_analyzer_extracts_env_and_subprocess_references()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::RustReferenceKind;
        use crate::domain::Confidence;

        let artifact = rust_artifact("app/worker.rs")?;
        let text = "\
use std::process::Command;

fn var(_key: &str) -> Option<String> {
    None
}

fn run(dynamic_key: &str) {
    std::env::var_os(\"PATH\");
    std::env::var(dynamic_key);
    Command::new(\"git\").arg(\"status\");
    var(\"not_a_real_env_read\");
}
";
        let analysis = RustAnalyzer.analyze(&artifact, text);

        assert!(analysis.references.iter().any(|reference| {
            reference.kind == RustReferenceKind::EnvRead
                && reference.value == "PATH"
                && reference.confidence == Confidence::High
        }));
        assert!(
            analysis.references.iter().any(|reference| {
                reference.kind == RustReferenceKind::EnvRead
                    && reference.confidence == Confidence::Low
            }),
            "expected a low-confidence EnvRead for the non-literal std::env::var argument"
        );
        assert!(analysis.references.iter().any(|reference| {
            reference.kind == RustReferenceKind::Subprocess
                && reference.value == "git"
                && reference.confidence == Confidence::High
        }));

        // A locally-defined `var(...)` function -- unrelated to
        // `std::env::var` and never imported from it -- must not be
        // misclassified as an env read just because its name matches.
        let env_read_values: Vec<&str> = analysis
            .references
            .iter()
            .filter(|reference| reference.kind == RustReferenceKind::EnvRead)
            .map(|reference| reference.value.as_str())
            .collect();
        assert!(!env_read_values.contains(&"not_a_real_env_read"));

        Ok(())
    }

    #[test]
    fn rust_analyzer_extracts_mods_generics_wildcards_and_docs()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = rust_artifact("src/routes/mod.rs")?;
        let text = "\
use std::collections::{HashMap, HashSet as Set};
use crate::prelude::*;

mod handlers;
mod inline {
    pub fn helper() {}
}

/// Route table.
#[derive(Debug)]
pub enum Route {
    Get,
    Post,
}
";
        let analysis = RustAnalyzer.analyze(&artifact, text);

        assert_eq!(analysis.module_path, "src::routes");
        assert!(
            analysis
                .uses
                .iter()
                .any(|use_| use_.path == "std::collections::HashMap")
        );
        assert!(
            analysis
                .uses
                .iter()
                .any(|use_| use_.path == "std::collections::HashSet"
                    && use_.alias.as_deref() == Some("Set"))
        );
        assert!(
            analysis
                .uses
                .iter()
                .any(|use_| use_.path == "crate::prelude::*")
        );

        assert_eq!(analysis.mod_declarations[0].name, "handlers");
        assert!(!analysis.mod_declarations[0].is_inline);
        assert!(analysis.mod_declarations[1].is_inline);

        let route = &analysis.enums[0];
        assert_eq!(route.name, "Route");
        assert_eq!(route.attributes, vec!["derive(Debug)"]);
        assert_eq!(route.doc.as_deref(), Some("Route table."));

        Ok(())
    }

    #[test]
    fn rust_analyzer_respects_policy_and_records_syntax_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let never = rust_artifact("src/secret.rs")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let binary = rust_artifact("src/binary.rs")?.with_text_status(TextStatus::Binary, None);
        let artifact = rust_artifact("src/broken.rs")?;
        let broken = "fn broken(: {\n";

        assert_eq!(
            RustAnalyzer.analyze(&never, "fn f() {}"),
            super::RustAnalysis::default()
        );
        assert_eq!(
            RustAnalyzer.analyze(&binary, "fn f() {}"),
            super::RustAnalysis::default()
        );
        assert!(RustAnalyzer.analyze(&artifact, broken).has_syntax_errors);

        Ok(())
    }
}
