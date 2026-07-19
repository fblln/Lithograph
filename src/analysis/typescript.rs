//! Deep TypeScript and TSX source analysis.
//!
//! The broad tree-sitter adapter remains the source of syntax-level import,
//! type-reference, and identifier-use facts. This analyzer adds the semantic
//! declaration layer that those facts alone cannot provide: named functions,
//! arrow functions, classes, and class methods.

use crate::analysis::{SyntaxIndexedLanguage, TreeSitterAdapterOutput};
use crate::domain::{
    Artifact, ArtifactId, Confidence, EvidenceRef, ModelExposurePolicy, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use tree_sitter::Node;

/// Deep analysis output for one TypeScript or TSX artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptAnalysis {
    /// TypeScript grammar selected for the artifact.
    pub language: TypeScriptLanguage,
    /// Existing tree-sitter syntax facts retained during the deep-analysis
    /// transition. In particular, callers still receive imports, type refs,
    /// and identifier uses from this field.
    pub syntax: TreeSitterAdapterOutput,
    /// Named classes declared in the artifact.
    pub classes: Vec<TypeScriptClass>,
    /// Named top-level functions and arrow-function bindings.
    pub functions: Vec<TypeScriptFunction>,
    /// LIT-81: module-level *value* `const`/`let`/`var` declarations that are
    /// not arrow functions -- `export const $ItemCreate = {...}`, a TanStack
    /// `const LayoutRoute = createRoute(...)`. They are real module exports the
    /// analyzer previously dropped (only arrow-const bindings became symbols),
    /// so a same-file or barrel reference to one had nothing to resolve to.
    pub value_bindings: Vec<TypeScriptFunction>,
    /// Call sites whose callee is statically named. Resolution is deferred to
    /// graph construction, where local symbols and imported artifacts exist.
    pub calls: Vec<TypeScriptCall>,
    /// Environment reads such as `process.env.NAME`.
    pub env_reads: Vec<TypeScriptEnvRead>,
    /// LIT-57: member calls paired with the receiver name and enclosing class.
    /// `calls` keeps only a member call's property segment, which cannot be
    /// resolved without knowing what the receiver is; these facts carry the
    /// receiver evidence the cross-file type-propagation pass needs.
    pub member_calls: Vec<TypeScriptMemberCall>,
    /// LIT-57: `const name = new Ctor(...)` bindings, which type `name`.
    pub bindings: Vec<TypeScriptBinding>,
    /// LIT-45.3: `export ... from './x'` re-exports, which let a symbol
    /// imported from a barrel be traced to the file that declares it.
    pub re_exports: Vec<TypeScriptReExport>,
    /// LIT-80: names bound *inside a function body* -- block-scoped `const`/
    /// `let`/`var` declarators (including destructuring) and parameters. These
    /// are the file's function-local variables; a bare use of one is a local
    /// reference resolved by lexical scoping, not a cross-file symbol, so the
    /// graph builder suppresses the `Unresolved` use-site node it would
    /// otherwise mint. Deliberately excludes module-top-level bindings, which
    /// may be exported and are left to normal resolution (LIT-75).
    pub local_value_bindings: BTreeSet<String>,
}

impl Default for TypeScriptAnalysis {
    fn default() -> Self {
        Self {
            language: TypeScriptLanguage::TypeScript,
            syntax: TreeSitterAdapterOutput::fallback("typescript", "analysis not run"),
            classes: Vec::new(),
            functions: Vec::new(),
            value_bindings: Vec::new(),
            calls: Vec::new(),
            env_reads: Vec::new(),
            member_calls: Vec::new(),
            bindings: Vec::new(),
            re_exports: Vec::new(),
            local_value_bindings: BTreeSet::new(),
        }
    }
}

/// The two TypeScript grammar variants supported by the deep analyzer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeScriptLanguage {
    /// Standard TypeScript (`.ts`, `.mts`, `.cts`).
    TypeScript,
    /// TypeScript with JSX (`.tsx`).
    Tsx,
}

impl TypeScriptLanguage {
    /// Returns the matching registry language identifier.
    pub fn registry_id(self) -> &'static str {
        match self {
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
        }
    }

    fn syntax_language(self) -> SyntaxIndexedLanguage {
        match self {
            Self::TypeScript => SyntaxIndexedLanguage::TypeScript,
            Self::Tsx => SyntaxIndexedLanguage::Tsx,
        }
    }

    fn grammar(self) -> tree_sitter::Language {
        match self {
            Self::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Self::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }
}

/// A TypeScript class declaration and its direct methods.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptClass {
    /// Class name.
    pub name: String,
    /// Direct superclass expressions from the class's `extends` clause.
    /// TypeScript permits one runtime superclass; this remains a vector so
    /// recovered parse trees cannot silently discard repeated clauses.
    pub bases: Vec<String>,
    /// Directly declared methods, including arrow-function class fields.
    pub methods: Vec<TypeScriptFunction>,
    /// Source evidence for the class declaration.
    pub evidence: EvidenceRef,
}

/// A named TypeScript function, arrow-function binding, or method.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptFunction {
    /// Name declared by the function or binding.
    pub name: String,
    /// Source evidence for the declaration.
    pub evidence: EvidenceRef,
}

/// One TypeScript call with a statically readable callee name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptCall {
    /// Bare function name or the property segment of a member call.
    pub name: String,
    /// Source evidence for the call expression.
    pub evidence: EvidenceRef,
}

/// One TypeScript environment read. Dynamic property expressions retain their
/// source expression but have no fabricated variable name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptEnvRead {
    /// Literal environment name, when statically readable.
    pub name: Option<String>,
    /// Full property expression as written.
    pub expression: String,
    /// Confidence in the extracted name.
    pub confidence: Confidence,
    /// Source evidence for the member expression.
    pub evidence: EvidenceRef,
}

/// LIT-57: one `receiver.method(...)` call whose receiver is a bare name.
///
/// Only bare-identifier and `this` receivers are extracted. Chained receivers
/// (`a.b.method()`, `f().method()`) carry no single name a binding
/// environment could type, so they are left out rather than guessed at.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptMemberCall {
    /// Receiver identifier as written, e.g. `this` or `provider`.
    pub receiver: String,
    /// Called method name.
    pub method: String,
    /// Enclosing class name, when the call sits inside a class body. This is
    /// what types a `this` receiver.
    pub enclosing_class: Option<String>,
    /// Evidence for the call expression.
    pub evidence: EvidenceRef,
}

/// LIT-57: one `const name = new Ctor(...)` binding, which types `name`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptBinding {
    /// Bound name.
    pub name: String,
    /// Constructed type name as written.
    pub constructor: String,
    /// Enclosing class name, when bound inside a class body.
    pub enclosing_class: Option<String>,
    /// True when bound at module level. Module-level bindings are visible to
    /// importing files; function-local ones are not.
    pub is_module_level: bool,
    /// Evidence for the declarator.
    pub evidence: EvidenceRef,
}

/// LIT-45.3: how a barrel re-exports names from another module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeScriptReExportKind {
    /// `export * from './a'` -- every name the target exports, unrenamed.
    Star,
    /// `export { A } from './b'`, or `export { A as B } from './b'`, where
    /// `A` is the name in the target and `B` the name consumers import.
    Named {
        /// Name as the target module declares it.
        exported: String,
        /// Name as this barrel publishes it.
        local: String,
    },
}

/// LIT-45.3: one `export ... from './x'` statement.
///
/// `export * as ns from './x'` is deliberately not modelled: it binds a
/// namespace object rather than republishing names, so `ns.foo()` is a module
/// member access and not a symbol a consumer can import by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeScriptReExport {
    /// Module specifier as written, e.g. `./core/ApiError`.
    pub specifier: String,
    /// What is being republished.
    pub kind: TypeScriptReExportKind,
    /// Evidence for the export statement.
    pub evidence: EvidenceRef,
}

/// LIT-45.3: collects `export ... from` statements. Only re-exports have a
/// `source`; a plain `export class Foo {}` declares locally and is already
/// covered by the declaration passes.
fn collect_re_exports(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    re_exports: &mut Vec<TypeScriptReExport>,
) {
    if node.kind() == "export_statement"
        && let Some(specifier) = node
            .child_by_field_name("source")
            .and_then(|value| string_literal_text(value, source))
    {
        match node.child_by_field_name("declaration") {
            // `export * as ns from './x'` parses with a namespace_export
            // child; see the type comment for why it is skipped.
            _ if has_child_of_kind(node, "namespace_export") => {}
            _ => {
                let clause = node
                    .children(&mut node.walk())
                    .find(|child| child.kind() == "export_clause");
                match clause {
                    Some(clause) => {
                        let mut cursor = clause.walk();
                        for specifier_node in clause
                            .children(&mut cursor)
                            .filter(|child| child.kind() == "export_specifier")
                        {
                            let Some(exported) = field_text(specifier_node, "name", source) else {
                                continue;
                            };
                            // `alias` is present only for `A as B`.
                            let local = field_text(specifier_node, "alias", source)
                                .unwrap_or(exported)
                                .to_owned();
                            re_exports.push(TypeScriptReExport {
                                specifier: specifier.clone(),
                                kind: TypeScriptReExportKind::Named {
                                    exported: exported.to_owned(),
                                    local,
                                },
                                evidence: evidence(artifact, node),
                            });
                        }
                    }
                    // No clause but a source: `export * from './x'`.
                    None => re_exports.push(TypeScriptReExport {
                        specifier,
                        kind: TypeScriptReExportKind::Star,
                        evidence: evidence(artifact, node),
                    }),
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_re_exports(child, artifact, source, re_exports);
    }
}

fn has_child_of_kind(node: Node<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.children(&mut cursor).any(|child| child.kind() == kind)
}

/// LIT-80: collects names bound *inside a function body* -- `const`/`let`/`var`
/// declarators and parameters, following destructuring. `in_function` starts
/// false at module scope and latches true under the first function/arrow/method
/// node, so a module-level `const config = ...` is never collected (it may be
/// exported and is left to normal resolution) while a block-local
/// `const addItem = ...` is. Type annotations are `type_identifier` nodes and a
/// default value lives on the `right` of an assignment pattern, so neither is
/// mistaken for a binding.
fn collect_local_value_bindings(
    node: Node<'_>,
    source: &str,
    in_function: bool,
    out: &mut BTreeSet<String>,
) {
    let in_function = in_function
        || matches!(
            node.kind(),
            "function_declaration"
                | "generator_function_declaration"
                | "function_expression"
                | "arrow_function"
                | "method_definition"
        );
    if in_function {
        match node.kind() {
            "variable_declarator" => {
                if let Some(name) = node.child_by_field_name("name") {
                    collect_binding_identifiers(name, source, out);
                }
            }
            "required_parameter" | "optional_parameter" => {
                if let Some(pattern) = node.child_by_field_name("pattern") {
                    collect_binding_identifiers(pattern, source, out);
                }
            }
            // A single-parameter arrow (`item => ...`) names the parameter
            // directly rather than through a `formal_parameters` list.
            "arrow_function" => {
                if let Some(parameter) = node.child_by_field_name("parameter") {
                    collect_binding_identifiers(parameter, source, out);
                }
            }
            // `catch (e)` binds a function-local exception name.
            "catch_clause" => {
                if let Some(parameter) = node.child_by_field_name("parameter") {
                    collect_binding_identifiers(parameter, source, out);
                }
            }
            _ => {}
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_local_value_bindings(child, source, in_function, out);
    }
}

/// Collects the identifiers a binding pattern introduces, descending through
/// object/array destructuring. For `{ a = fallback }` only the binding side is
/// taken, never the `fallback` default expression, so a reference used as a
/// default is not misread as a local binding.
fn collect_binding_identifiers(node: Node<'_>, source: &str, out: &mut BTreeSet<String>) {
    match node.kind() {
        "identifier" | "shorthand_property_identifier_pattern" => {
            out.insert(node_text(node, source).to_owned());
        }
        // `left = default` in a pattern: only the left is a binding.
        "assignment_pattern" | "object_assignment_pattern" => {
            if let Some(left) = node.child_by_field_name("left") {
                collect_binding_identifiers(left, source, out);
            }
        }
        _ => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_binding_identifiers(child, source, out);
            }
        }
    }
}

/// Text inside a string node, without its quotes.
fn string_literal_text(node: Node<'_>, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .find(|child| child.kind() == "string_fragment")
        .map(|fragment| node_text(fragment, source).to_owned())
}

/// LIT-57: walks the tree tracking the enclosing class (which types `this`)
/// and whether a binding is module-level (which decides if importers see it).
#[allow(clippy::too_many_arguments)]
fn collect_scoped_facts(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    enclosing_class: Option<&str>,
    is_module_level: bool,
    member_calls: &mut Vec<TypeScriptMemberCall>,
    bindings: &mut Vec<TypeScriptBinding>,
) {
    match node.kind() {
        "call_expression" => {
            if let Some(member_call) = classify_member_call(node, artifact, source, enclosing_class)
            {
                member_calls.push(member_call);
            }
        }
        "variable_declarator" => {
            if let Some(binding) =
                classify_binding(node, artifact, source, enclosing_class, is_module_level)
            {
                bindings.push(binding);
            }
        }
        // LIT-45.5: `function f(svc: ItemsService)` (and the constructor
        // parameter-property form) types `svc` exactly as `new ItemsService()`
        // would. Only a bare `type_identifier` annotation counts: unions,
        // `ns.Baz`, generics, and predefined types are distinct node kinds and
        // fall through rather than being unwrapped.
        "required_parameter" | "optional_parameter" => {
            if let Some(annotation) = node
                .child_by_field_name("type")
                .filter(|annotation| annotation.named_child_count() == 1)
                .and_then(|annotation| annotation.named_child(0))
                .filter(|inner| inner.kind() == "type_identifier")
                && let Some(name) = node
                    .child_by_field_name("pattern")
                    .filter(|pattern| pattern.kind() == "identifier")
            {
                bindings.push(TypeScriptBinding {
                    name: node_text(name, source).to_owned(),
                    constructor: node_text(annotation, source).to_owned(),
                    enclosing_class: enclosing_class.map(str::to_owned),
                    is_module_level: false,
                    evidence: evidence(artifact, node),
                });
            }
        }
        _ => {}
    }

    let (child_class, child_module_level) = match node.kind() {
        "class_declaration" | "abstract_class_declaration" | "class" => {
            (field_text(node, "name", source).or(enclosing_class), false)
        }
        "function_declaration"
        | "generator_function_declaration"
        | "function_expression"
        | "arrow_function"
        | "method_definition" => (enclosing_class, false),
        _ => (enclosing_class, is_module_level),
    };

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_scoped_facts(
            child,
            artifact,
            source,
            child_class,
            child_module_level,
            member_calls,
            bindings,
        );
    }
}

fn classify_member_call(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    enclosing_class: Option<&str>,
) -> Option<TypeScriptMemberCall> {
    let function = node.child_by_field_name("function")?;
    if !matches!(function.kind(), "member_expression" | "optional_chain") {
        return None;
    }
    let object = function.child_by_field_name("object")?;
    if !matches!(object.kind(), "identifier" | "this") {
        return None;
    }
    let method = field_text(function, "property", source)?;

    Some(TypeScriptMemberCall {
        receiver: node_text(object, source).to_owned(),
        method: method.to_owned(),
        enclosing_class: enclosing_class.map(str::to_owned),
        evidence: evidence(artifact, node),
    })
}

/// Extracts `name = new Ctor(...)`, the only declarator shape that types a
/// name without inference. `const a = b` and `const a = f()` are left out:
/// a plain call's return type is not readable from syntax.
fn classify_binding(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    enclosing_class: Option<&str>,
    is_module_level: bool,
) -> Option<TypeScriptBinding> {
    let name = node.child_by_field_name("name")?;
    if name.kind() != "identifier" {
        return None;
    }
    let value = node.child_by_field_name("value")?;
    if value.kind() != "new_expression" {
        return None;
    }
    let constructor = value.child_by_field_name("constructor")?;
    if constructor.kind() != "identifier" {
        return None;
    }

    Some(TypeScriptBinding {
        name: node_text(name, source).to_owned(),
        constructor: node_text(constructor, source).to_owned(),
        enclosing_class: enclosing_class.map(str::to_owned),
        is_module_level,
        evidence: evidence(artifact, node),
    })
}

/// Tree-sitter-backed analyzer for TypeScript and TSX artifacts.
#[derive(Debug, Clone, Copy)]
pub struct TypeScriptAnalyzer {
    language: TypeScriptLanguage,
}

impl TypeScriptAnalyzer {
    /// Creates an analyzer for standard TypeScript.
    pub fn typescript() -> Self {
        Self {
            language: TypeScriptLanguage::TypeScript,
        }
    }

    /// Creates an analyzer for TSX.
    pub fn tsx() -> Self {
        Self {
            language: TypeScriptLanguage::Tsx,
        }
    }

    /// Parses a safe TypeScript/TSX artifact and extracts typed declarations.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> TypeScriptAnalysis {
        let syntax = self.syntax_language().adapter().parse(text);
        // Every bail-out below keeps the syntax facts and yields no deep
        // facts, so they share one shape rather than restating each field.
        let syntax_only = |syntax| TypeScriptAnalysis {
            language: self.language,
            syntax,
            ..TypeScriptAnalysis::default()
        };
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return syntax_only(syntax);
        }

        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&self.language.grammar()).is_err() {
            return syntax_only(syntax);
        }
        let Some(tree) = parser.parse(text, None) else {
            return syntax_only(syntax);
        };

        let mut classes = Vec::new();
        let mut functions = Vec::new();
        let mut value_bindings = Vec::new();
        let mut cursor = tree.root_node().walk();
        for child in tree.root_node().children(&mut cursor) {
            collect_top_level(child, artifact, text, &mut classes, &mut functions);
            collect_top_level_value_bindings(child, artifact, text, &mut value_bindings);
        }
        let mut calls = Vec::new();
        collect_calls(tree.root_node(), artifact, text, &mut calls);
        let mut env_reads = Vec::new();
        collect_env_reads(tree.root_node(), artifact, text, &mut env_reads);
        let mut member_calls = Vec::new();
        let mut bindings = Vec::new();
        collect_scoped_facts(
            tree.root_node(),
            artifact,
            text,
            None,
            true,
            &mut member_calls,
            &mut bindings,
        );
        let mut re_exports = Vec::new();
        collect_re_exports(tree.root_node(), artifact, text, &mut re_exports);
        let mut local_value_bindings = BTreeSet::new();
        collect_local_value_bindings(tree.root_node(), text, false, &mut local_value_bindings);

        TypeScriptAnalysis {
            language: self.language,
            syntax,
            classes,
            functions,
            value_bindings,
            calls,
            env_reads,
            member_calls,
            bindings,
            re_exports,
            local_value_bindings,
        }
    }

    fn syntax_language(self) -> SyntaxIndexedLanguage {
        self.language.syntax_language()
    }
}

fn collect_calls(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    calls: &mut Vec<TypeScriptCall>,
) {
    if node.kind() == "call_expression"
        && let Some(function) = node.child_by_field_name("function")
        && let Some(name) = callee_name(function, source)
    {
        calls.push(TypeScriptCall {
            name: name.to_owned(),
            evidence: evidence(artifact, node),
        });
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_calls(child, artifact, source, calls);
    }
}

fn collect_env_reads(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    reads: &mut Vec<TypeScriptEnvRead>,
) {
    if matches!(node.kind(), "member_expression" | "subscript_expression") {
        let expression = node_text(node, source).to_owned();
        if expression.starts_with("process.env.") || expression.starts_with("process.env[") {
            let name = expression
                .strip_prefix("process.env.")
                .map(str::trim)
                .or_else(|| {
                    expression
                        .strip_prefix("process.env[")
                        .and_then(|value| value.strip_suffix(']'))
                        .map(str::trim)
                        .filter(|value| {
                            value
                                .chars()
                                .next()
                                .is_some_and(|character| matches!(character, '\"' | '\'' | '`'))
                        })
                })
                .map(|value| value.trim_matches(['\"', '\'', '`']))
                .filter(|value| {
                    !value.is_empty()
                        && value
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric() || character == '_')
                })
                .map(str::to_owned);
            reads.push(TypeScriptEnvRead {
                confidence: if name.is_some() {
                    Confidence::High
                } else {
                    Confidence::Low
                },
                name,
                expression,
                evidence: evidence(artifact, node),
            });
        }
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_env_reads(child, artifact, source, reads);
    }
}

fn node_text<'a>(node: Node<'_>, source: &'a str) -> &'a str {
    source.get(node.byte_range()).unwrap_or_default()
}

fn callee_name<'a>(function: Node<'_>, source: &'a str) -> Option<&'a str> {
    match function.kind() {
        "identifier" => source.get(function.byte_range()),
        "member_expression" | "optional_chain" => function
            .child_by_field_name("property")
            .and_then(|property| source.get(property.byte_range())),
        _ => None,
    }
}

fn collect_top_level(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    classes: &mut Vec<TypeScriptClass>,
    functions: &mut Vec<TypeScriptFunction>,
) {
    match node.kind() {
        "class_declaration" | "abstract_class_declaration" => {
            classes.push(build_class(node, artifact, source));
        }
        "function_declaration" | "generator_function_declaration" => {
            if let Some(function) = build_named_function(node, artifact, source) {
                functions.push(function);
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            collect_arrow_bindings(node, artifact, source, functions);
        }
        // Export and ambient wrappers own a declaration as a direct child;
        // unwrap only that layer so nested implementation details never turn
        // into top-level symbols accidentally.
        "export_statement" | "ambient_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_top_level(child, artifact, source, classes, functions);
            }
        }
        _ => {}
    }
}

fn build_class(node: Node<'_>, artifact: &Artifact, source: &str) -> TypeScriptClass {
    let name = field_text(node, "name", source)
        .unwrap_or_default()
        .to_owned();
    let methods = node
        .child_by_field_name("body")
        .map(|body| collect_class_methods(body, artifact, source))
        .unwrap_or_default();
    let bases = node
        .children(&mut node.walk())
        .find(|child| child.kind() == "class_heritage")
        .into_iter()
        .flat_map(|heritage| {
            let mut cursor = heritage.walk();
            heritage
                .children(&mut cursor)
                .filter(|child| child.kind() == "extends_clause")
                .filter_map(|clause| clause.child_by_field_name("value"))
                .filter(|value| matches!(value.kind(), "identifier" | "member_expression"))
                .map(|value| node_text(value, source).to_owned())
                .collect::<Vec<_>>()
        })
        .collect();
    TypeScriptClass {
        name,
        bases,
        methods,
        evidence: evidence(artifact, node),
    }
}

fn collect_class_methods(
    body: Node<'_>,
    artifact: &Artifact,
    source: &str,
) -> Vec<TypeScriptFunction> {
    let mut methods = Vec::new();
    let mut cursor = body.walk();
    for child in body.children(&mut cursor) {
        match child.kind() {
            "method_definition" | "abstract_method_signature" => {
                if let Some(method) = build_named_function(child, artifact, source) {
                    methods.push(method);
                }
            }
            // `handler = () => {}` is a method-like callable field on the
            // class and must resolve like a method rather than a free
            // function in the follow-on call-resolution pass.
            "public_field_definition" => {
                if child
                    .child_by_field_name("value")
                    .is_some_and(|value| value.kind() == "arrow_function")
                    && let Some(method) = build_named_function(child, artifact, source)
                {
                    methods.push(method);
                }
            }
            _ => {}
        }
    }
    methods
}

/// LIT-81: collects module-level *value* bindings -- a top-level `const`/`let`/
/// `var` declarator with an identifier name whose initializer is not an arrow
/// function (those are already collected as callable `functions`). Mirrors
/// `collect_top_level`'s unwrapping of `export`/`ambient` wrappers so an
/// `export const $X = {...}` is reached, and stays at module scope (never
/// descends into function bodies, whose locals are handled by LIT-80).
fn collect_top_level_value_bindings(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
    value_bindings: &mut Vec<TypeScriptFunction>,
) {
    match node.kind() {
        "lexical_declaration" | "variable_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() != "variable_declarator" {
                    continue;
                }
                let is_arrow = child
                    .child_by_field_name("value")
                    .is_some_and(|value| value.kind() == "arrow_function");
                if is_arrow {
                    continue;
                }
                if let Some(name) = child.child_by_field_name("name")
                    && name.kind() == "identifier"
                {
                    value_bindings.push(TypeScriptFunction {
                        name: node_text(name, source).to_owned(),
                        evidence: evidence(artifact, child),
                    });
                }
            }
        }
        "export_statement" | "ambient_declaration" => {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                collect_top_level_value_bindings(child, artifact, source, value_bindings);
            }
        }
        _ => {}
    }
}

fn collect_arrow_bindings(
    declaration: Node<'_>,
    artifact: &Artifact,
    source: &str,
    functions: &mut Vec<TypeScriptFunction>,
) {
    let mut cursor = declaration.walk();
    for child in declaration.children(&mut cursor) {
        if child.kind() != "variable_declarator" {
            continue;
        }
        if child
            .child_by_field_name("value")
            .is_some_and(|value| value.kind() == "arrow_function")
            && let Some(function) = build_named_function(child, artifact, source)
        {
            functions.push(function);
        }
    }
}

fn build_named_function(
    node: Node<'_>,
    artifact: &Artifact,
    source: &str,
) -> Option<TypeScriptFunction> {
    let name = field_text(node, "name", source)?;
    (!name.is_empty()).then(|| TypeScriptFunction {
        name: name.to_owned(),
        evidence: evidence(artifact, node),
    })
}

fn field_text<'a>(node: Node<'_>, field: &str, source: &'a str) -> Option<&'a str> {
    node.child_by_field_name(field)
        .and_then(|child| source.get(child.byte_range()))
}

fn evidence(artifact: &Artifact, node: Node<'_>) -> EvidenceRef {
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
    use super::TypeScriptAnalyzer;
    use crate::domain::{
        AnalyzerSelection, Artifact, ArtifactCategory, Confidence, ContentHash,
        ModelExposurePolicy, RepoPath, SupportTier, TextStatus,
    };

    fn artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::DeepLanguage,
            ContentHash::new("00")?,
            0,
        )
        .with_detected_format(if path.ends_with(".tsx") {
            "tsx"
        } else {
            "typescript"
        })
        .with_text_status(TextStatus::Text, Some(1))
        .with_model_policy(ModelExposurePolicy::Allowed)
        .with_analyzer(AnalyzerSelection::Specialized("typescript".to_owned())))
    }

    /// LIT-45.3: every `export ... from` shape a barrel uses, including the
    /// two that must NOT become re-export facts.
    #[test]
    fn re_exports_capture_named_star_and_aliased_forms() -> Result<(), Box<dyn std::error::Error>> {
        use super::TypeScriptReExportKind::{Named, Star};

        let analysis = TypeScriptAnalyzer::typescript().analyze(
            &artifact("src/client/index.ts")?,
            "\
export { ApiError } from './core/ApiError';
export * from './models';
export { C as D } from './c';
export type { OpenAPIConfig } from './core/OpenAPI';
export * as ns from './d';
export class Local {}
",
        );

        let facts: Vec<_> = analysis
            .re_exports
            .iter()
            .map(|re_export| (re_export.specifier.as_str(), re_export.kind.clone()))
            .collect();

        assert_eq!(
            facts,
            vec![
                (
                    "./core/ApiError",
                    Named {
                        exported: "ApiError".to_owned(),
                        local: "ApiError".to_owned(),
                    },
                ),
                ("./models", Star),
                (
                    "./c",
                    Named {
                        exported: "C".to_owned(),
                        local: "D".to_owned(),
                    },
                ),
                // A type-only re-export republishes a name the same way.
                (
                    "./core/OpenAPI",
                    Named {
                        exported: "OpenAPIConfig".to_owned(),
                        local: "OpenAPIConfig".to_owned(),
                    },
                ),
            ],
            "`export * as ns` binds a namespace object, and `export class Local` \
             declares locally -- neither republishes an importable name",
        );

        Ok(())
    }

    /// LIT-57: the two facts a cross-file propagation pass types receivers
    /// from. The scope fields are what make them useful, so they are asserted
    /// directly rather than inferred from a resolved edge downstream.
    #[test]
    fn member_calls_and_bindings_carry_their_scope() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = TypeScriptAnalyzer::typescript().analyze(
            &artifact("src/app.ts")?,
            "\
const provider = new Provider();

class Encoder {
  dump() {
    this.dumps();
  }

  helper() {
    const local = new Helper();
    local.run();
    a.b.chained();
  }
}

function free() {
  other.method();
}

function annotated(svc: ItemsService, plain, u: Foo | Bar, n: number, ns: a.Baz) {
  svc.deleteItem();
}
",
        );

        let member_calls: Vec<_> = analysis
            .member_calls
            .iter()
            .map(|call| {
                (
                    call.receiver.as_str(),
                    call.method.as_str(),
                    call.enclosing_class.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            member_calls,
            vec![
                // `this` is typed by the class it sits in.
                ("this", "dumps", Some("Encoder")),
                ("local", "run", Some("Encoder")),
                // Outside any class there is no `this` type to carry.
                ("other", "method", None),
                // The annotated parameter's call, typed by the binding below.
                ("svc", "deleteItem", None),
            ],
            "`a.b.chained()` must be absent: a chained receiver has no single name to type",
        );

        let bindings: Vec<_> = analysis
            .bindings
            .iter()
            .map(|binding| {
                (
                    binding.name.as_str(),
                    binding.constructor.as_str(),
                    binding.is_module_level,
                )
            })
            .collect();
        assert_eq!(
            bindings,
            vec![
                // Module level: importers can see this one.
                ("provider", "Provider", true),
                // Method-local: they cannot.
                ("local", "Helper", false),
                // LIT-45.5: only the bare type_identifier annotation counts;
                // `plain`, the union, `number`, and `a.Baz` are all skipped.
                ("svc", "ItemsService", false),
            ],
        );

        Ok(())
    }

    #[test]
    fn extracts_typed_typescript_declarations() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = TypeScriptAnalyzer::typescript().analyze(
            &artifact("src/service.ts")?,
            "class Base {}\nexport class Service extends Base {\n  run() {}\n  handler = () => {};\n}\nexport function start() {}\nexport const stop = () => {};\n",
        );

        assert_eq!(analysis.classes.len(), 2);
        assert_eq!(analysis.classes[1].name, "Service");
        assert_eq!(analysis.classes[1].bases, ["Base"]);
        assert_eq!(
            analysis.classes[1]
                .methods
                .iter()
                .map(|method| method.name.as_str())
                .collect::<Vec<_>>(),
            ["run", "handler"]
        );
        assert_eq!(
            analysis
                .functions
                .iter()
                .map(|function| function.name.as_str())
                .collect::<Vec<_>>(),
            ["start", "stop"]
        );
        assert_eq!(analysis.syntax.language_id, "typescript");
        assert!(analysis.calls.is_empty());
        Ok(())
    }

    #[test]
    fn extracts_typed_tsx_declarations_and_keeps_syntax_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let analysis = TypeScriptAnalyzer::tsx().analyze(
            &artifact("src/App.tsx")?,
            "import type { Props } from \"./types\";\nclass View { render(): void {} }\nconst App = (_props: Props) => <main />;\n",
        );

        assert_eq!(analysis.classes[0].name, "View");
        assert_eq!(analysis.classes[0].methods[0].name, "render");
        assert_eq!(analysis.functions[0].name, "App");
        assert_eq!(analysis.syntax.imports.len(), 1);
        assert!(
            analysis
                .syntax
                .symbols
                .iter()
                .any(|fact| fact.text == "Props")
        );
        Ok(())
    }

    #[test]
    fn records_bare_and_member_call_names() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = TypeScriptAnalyzer::typescript().analyze(
            &artifact("src/calls.ts")?,
            "function run() {}\nclass Service { start() {} }\nrun();\nnew Service().start();\nunknown();\n",
        );

        assert_eq!(
            analysis
                .calls
                .iter()
                .map(|call| call.name.as_str())
                .collect::<Vec<_>>(),
            ["run", "start", "unknown"]
        );
        Ok(())
    }

    #[test]
    fn extracts_literal_and_dynamic_process_env_reads() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = TypeScriptAnalyzer::typescript().analyze(
            &artifact("src/config.ts")?,
            "const host = process.env.API_HOST; const key = process.env[envName];\n",
        );
        assert_eq!(analysis.env_reads.len(), 2);
        assert_eq!(analysis.env_reads[0].name.as_deref(), Some("API_HOST"));
        assert_eq!(analysis.env_reads[0].confidence, Confidence::High);
        assert!(analysis.env_reads[1].name.is_none());
        assert_eq!(analysis.env_reads[1].confidence, Confidence::Low);
        Ok(())
    }
}
