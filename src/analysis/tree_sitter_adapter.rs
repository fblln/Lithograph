//! Shared tree-sitter parser adapter output for broad syntax-indexed
//! languages. Specialized analyzers can stay richer while new languages use
//! this typed baseline.

use crate::domain::SourceSpan;

/// Coarse parse status for adapter output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TreeSitterParseStatus {
    /// A tree-sitter parser produced an AST.
    Parsed,
    /// No parser output was available, so callers should keep detected or
    /// generic facts instead of failing the pipeline.
    FallbackDetected {
        /// Human-readable fallback reason.
        reason: String,
    },
}

/// One syntax fact extracted from a tree-sitter node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSitterSyntaxFact {
    /// Tree-sitter node kind.
    pub kind: String,
    /// Source text covered by the node.
    pub text: String,
    /// One-based source span for the node.
    pub span: SourceSpan,
}

/// Comment extracted from a tree-sitter syntax tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSitterComment {
    /// Comment text.
    pub text: String,
    /// One-based source span for the comment.
    pub span: SourceSpan,
}

/// Syntax error extracted from a tree-sitter syntax tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSitterSyntaxError {
    /// Error node kind.
    pub kind: String,
    /// One-based source span for the error node.
    pub span: SourceSpan,
}

/// Typed baseline output from one parser adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeSitterAdapterOutput {
    /// Stable language id.
    pub language_id: String,
    /// Parse status.
    pub status: TreeSitterParseStatus,
    /// Definition-like syntax nodes.
    pub definitions: Vec<TreeSitterSyntaxFact>,
    /// Import/include/use syntax nodes.
    pub imports: Vec<TreeSitterSyntaxFact>,
    /// Symbol/name syntax nodes.
    pub symbols: Vec<TreeSitterSyntaxFact>,
    /// Source comments.
    pub comments: Vec<TreeSitterComment>,
    /// Syntax errors and missing nodes.
    pub syntax_errors: Vec<TreeSitterSyntaxError>,
}

impl TreeSitterAdapterOutput {
    /// Builds fallback output for a detected language without parser facts.
    pub fn fallback(language_id: &str, reason: impl Into<String>) -> Self {
        Self {
            language_id: language_id.to_owned(),
            status: TreeSitterParseStatus::FallbackDetected {
                reason: reason.into(),
            },
            definitions: Vec::new(),
            imports: Vec::new(),
            symbols: Vec::new(),
            comments: Vec::new(),
            syntax_errors: Vec::new(),
        }
    }
}

/// Declarative tree-sitter adapter for a language.
#[derive(Debug, Clone)]
pub struct TreeSitterParserAdapter {
    language_id: &'static str,
    language: tree_sitter::Language,
    definition_kinds: &'static [&'static str],
    import_kinds: &'static [&'static str],
    symbol_kinds: &'static [&'static str],
    comment_kinds: &'static [&'static str],
}

impl TreeSitterParserAdapter {
    /// Adapter for Python syntax facts. This does not replace
    /// [`PythonAnalyzer`](crate::analysis::PythonAnalyzer).
    pub fn python() -> Self {
        Self {
            language_id: "python",
            language: tree_sitter_python::LANGUAGE.into(),
            definition_kinds: &["function_definition", "class_definition"],
            import_kinds: &["import_statement", "import_from_statement"],
            symbol_kinds: &["identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for Rust syntax facts. This does not replace
    /// [`RustAnalyzer`](crate::analysis::RustAnalyzer).
    pub fn rust() -> Self {
        Self {
            language_id: "rust",
            language: tree_sitter_rust::LANGUAGE.into(),
            definition_kinds: &[
                "const_item",
                "enum_item",
                "function_item",
                "impl_item",
                "mod_item",
                "static_item",
                "struct_item",
                "trait_item",
                "type_item",
            ],
            import_kinds: &["extern_crate_declaration", "use_declaration"],
            symbol_kinds: &["field_identifier", "identifier", "type_identifier"],
            comment_kinds: &["block_comment", "line_comment"],
        }
    }

    /// Stable language id for this adapter.
    pub fn language_id(&self) -> &'static str {
        self.language_id
    }

    /// Parses source text into typed baseline syntax facts. Parser failures
    /// return fallback output instead of panicking.
    pub fn parse(&self, source: &str) -> TreeSitterAdapterOutput {
        let mut parser = tree_sitter::Parser::new();
        if let Err(error) = parser.set_language(&self.language) {
            return TreeSitterAdapterOutput::fallback(
                self.language_id,
                format!("failed to set tree-sitter language: {error}"),
            );
        }
        let Some(tree) = parser.parse(source, None) else {
            return TreeSitterAdapterOutput::fallback(
                self.language_id,
                "tree-sitter parser returned no tree",
            );
        };

        let mut output = TreeSitterAdapterOutput {
            language_id: self.language_id.to_owned(),
            status: TreeSitterParseStatus::Parsed,
            definitions: Vec::new(),
            imports: Vec::new(),
            symbols: Vec::new(),
            comments: Vec::new(),
            syntax_errors: Vec::new(),
        };
        collect_node_facts(self, tree.root_node(), source, &mut output);
        output
    }
}

/// Parses with an optional adapter, falling back to detected-only output when
/// no parser adapter is available for the language.
pub fn parse_with_optional_adapter(
    language_id: &str,
    adapter: Option<&TreeSitterParserAdapter>,
    source: &str,
) -> TreeSitterAdapterOutput {
    adapter.map_or_else(
        || TreeSitterAdapterOutput::fallback(language_id, "no tree-sitter adapter available"),
        |adapter| adapter.parse(source),
    )
}

fn collect_node_facts(
    adapter: &TreeSitterParserAdapter,
    node: tree_sitter::Node<'_>,
    source: &str,
    output: &mut TreeSitterAdapterOutput,
) {
    let kind = node.kind();
    if (node.is_error() || node.is_missing())
        && let Some(span) = node_span(node)
    {
        output.syntax_errors.push(TreeSitterSyntaxError {
            kind: kind.to_owned(),
            span,
        });
    }
    if adapter.definition_kinds.contains(&kind) {
        push_fact(&mut output.definitions, node, kind, source);
    }
    if adapter.import_kinds.contains(&kind) {
        push_fact(&mut output.imports, node, kind, source);
    }
    if adapter.symbol_kinds.contains(&kind) {
        push_fact(&mut output.symbols, node, kind, source);
    }
    if adapter.comment_kinds.contains(&kind)
        && let Some(span) = node_span(node)
    {
        output.comments.push(TreeSitterComment {
            text: node_text(node, source),
            span,
        });
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_node_facts(adapter, child, source, output);
    }
}

fn push_fact(
    facts: &mut Vec<TreeSitterSyntaxFact>,
    node: tree_sitter::Node<'_>,
    kind: &str,
    source: &str,
) {
    if let Some(span) = node_span(node) {
        facts.push(TreeSitterSyntaxFact {
            kind: kind.to_owned(),
            text: node_text(node, source),
            span,
        });
    }
}

fn node_span(node: tree_sitter::Node<'_>) -> Option<SourceSpan> {
    let start = node.start_position().row as u32 + 1;
    let end = node.end_position().row as u32 + 1;
    SourceSpan::new(start, end.max(start)).ok()
}

fn node_text(node: tree_sitter::Node<'_>, source: &str) -> String {
    node.utf8_text(source.as_bytes())
        .unwrap_or("")
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::{TreeSitterParseStatus, TreeSitterParserAdapter, parse_with_optional_adapter};
    use crate::analysis::{PythonAnalyzer, RustAnalyzer};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, RepoPath, SupportTier, TextStatus,
    };

    fn artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::DeepLanguage,
            ContentHash::new("aaaaaaaa")?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn python_adapter_extracts_typed_syntax_facts() {
        let source = "# module comment\nimport os\n\nclass Greeter:\n    def hello(self):\n        return os.getcwd()\n";
        let output = TreeSitterParserAdapter::python().parse(source);

        assert_eq!(output.status, TreeSitterParseStatus::Parsed);
        assert_eq!(output.language_id, "python");
        assert!(output.definitions.iter().any(|fact| {
            fact.kind == "class_definition" && fact.text.contains("class Greeter")
        }));
        assert!(
            output.definitions.iter().any(|fact| {
                fact.kind == "function_definition" && fact.text.contains("def hello")
            })
        );
        assert!(output.imports.iter().any(|fact| fact.text == "import os"));
        assert!(output.symbols.iter().any(|fact| fact.text == "Greeter"));
        assert_eq!(output.comments[0].text, "# module comment");
        assert!(output.syntax_errors.is_empty());
    }

    #[test]
    fn rust_adapter_extracts_typed_syntax_facts() {
        let source = "// crate comment\nuse std::fmt;\nstruct Greeter;\nimpl Greeter { fn hello(&self) {} }\n";
        let output = TreeSitterParserAdapter::rust().parse(source);

        assert_eq!(output.status, TreeSitterParseStatus::Parsed);
        assert_eq!(output.language_id, "rust");
        assert!(
            output
                .imports
                .iter()
                .any(|fact| fact.text == "use std::fmt;")
        );
        assert!(
            output
                .definitions
                .iter()
                .any(|fact| { fact.kind == "struct_item" && fact.text.contains("struct Greeter") })
        );
        assert!(
            output
                .definitions
                .iter()
                .any(|fact| fact.kind == "impl_item")
        );
        assert!(output.symbols.iter().any(|fact| fact.text == "Greeter"));
        assert_eq!(output.comments[0].text, "// crate comment");
        assert!(output.syntax_errors.is_empty());
    }

    #[test]
    fn adapter_records_syntax_errors_without_failing() {
        let output = TreeSitterParserAdapter::python().parse("def broken(:\n");

        assert_eq!(output.status, TreeSitterParseStatus::Parsed);
        assert!(!output.syntax_errors.is_empty());
    }

    #[test]
    fn missing_adapter_returns_detected_fallback() {
        let output = parse_with_optional_adapter("ruby", None, "class Greeter; end\n");

        assert_eq!(output.language_id, "ruby");
        assert!(matches!(
            output.status,
            TreeSitterParseStatus::FallbackDetected { .. }
        ));
        assert!(output.definitions.is_empty());
        assert!(output.imports.is_empty());
    }

    #[test]
    fn specialized_python_and_rust_analyzers_remain_unchanged()
    -> Result<(), Box<dyn std::error::Error>> {
        let python = PythonAnalyzer.analyze(&artifact("app.py")?, "def hello():\n    return 1\n");
        let rust = RustAnalyzer.analyze(&artifact("src/lib.rs")?, "fn hello() {}\n");

        assert_eq!(python.functions[0].name, "hello");
        assert_eq!(rust.functions[0].name, "hello");

        Ok(())
    }
}
