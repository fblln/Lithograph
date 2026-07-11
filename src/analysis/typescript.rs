//! Deep TypeScript and TSX source analysis.
//!
//! The broad tree-sitter adapter remains the source of syntax-level import,
//! type-reference, and identifier-use facts. This analyzer adds the semantic
//! declaration layer that those facts alone cannot provide: named functions,
//! arrow functions, classes, and class methods.

use crate::analysis::{SyntaxIndexedLanguage, TreeSitterAdapterOutput};
use crate::domain::{
    Artifact, ArtifactId, EvidenceRef, ModelExposurePolicy, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};
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
    /// Call sites whose callee is statically named. Resolution is deferred to
    /// graph construction, where local symbols and imported artifacts exist.
    pub calls: Vec<TypeScriptCall>,
}

impl Default for TypeScriptAnalysis {
    fn default() -> Self {
        Self {
            language: TypeScriptLanguage::TypeScript,
            syntax: TreeSitterAdapterOutput::fallback("typescript", "analysis not run"),
            classes: Vec::new(),
            functions: Vec::new(),
            calls: Vec::new(),
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
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return TypeScriptAnalysis {
                language: self.language,
                syntax,
                classes: Vec::new(),
                functions: Vec::new(),
                calls: Vec::new(),
            };
        }

        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&self.language.grammar()).is_err() {
            return TypeScriptAnalysis {
                language: self.language,
                syntax,
                classes: Vec::new(),
                functions: Vec::new(),
                calls: Vec::new(),
            };
        }
        let Some(tree) = parser.parse(text, None) else {
            return TypeScriptAnalysis {
                language: self.language,
                syntax,
                classes: Vec::new(),
                functions: Vec::new(),
                calls: Vec::new(),
            };
        };

        let mut classes = Vec::new();
        let mut functions = Vec::new();
        let mut cursor = tree.root_node().walk();
        for child in tree.root_node().children(&mut cursor) {
            collect_top_level(child, artifact, text, &mut classes, &mut functions);
        }
        let mut calls = Vec::new();
        collect_calls(tree.root_node(), artifact, text, &mut calls);

        TypeScriptAnalysis {
            language: self.language,
            syntax,
            classes,
            functions,
            calls,
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
    TypeScriptClass {
        name,
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
        AnalyzerSelection, Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath,
        SupportTier, TextStatus,
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

    #[test]
    fn extracts_typed_typescript_declarations() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = TypeScriptAnalyzer::typescript().analyze(
            &artifact("src/service.ts")?,
            "export class Service {\n  run() {}\n  handler = () => {};\n}\nexport function start() {}\nexport const stop = () => {};\n",
        );

        assert_eq!(analysis.classes.len(), 1);
        assert_eq!(analysis.classes[0].name, "Service");
        assert_eq!(
            analysis.classes[0]
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
}
