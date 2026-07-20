//! Shared tree-sitter parser adapter output for broad syntax-indexed
//! languages. Specialized analyzers can stay richer while new languages use
//! this typed baseline.

use crate::domain::SourceSpan;
use serde::{Deserialize, Serialize};

/// Definition node kinds shared by the TypeScript and TSX grammars, which
/// only differ in JSX syntax support, not in declaration shape.
const TS_DEFINITION_KINDS: &[&str] = &[
    "class_declaration",
    "abstract_class_declaration",
    "interface_declaration",
    "function_declaration",
    "generator_function_declaration",
    "enum_declaration",
    "type_alias_declaration",
    "method_definition",
];

/// Coarse parse status for adapter output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSitterSyntaxFact {
    /// Tree-sitter node kind.
    pub kind: String,
    /// Source text covered by the node.
    pub text: String,
    /// One-based source span for the node.
    pub span: SourceSpan,
}

/// Comment extracted from a tree-sitter syntax tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSitterComment {
    /// Comment text.
    pub text: String,
    /// One-based source span for the comment.
    pub span: SourceSpan,
}

/// Syntax error extracted from a tree-sitter syntax tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeSitterSyntaxError {
    /// Error node kind.
    pub kind: String,
    /// One-based source span for the error node.
    pub span: SourceSpan,
}

/// Typed baseline output from one parser adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Adapter for C syntax facts.
    pub fn c() -> Self {
        Self {
            language_id: "c",
            language: tree_sitter_c::LANGUAGE.into(),
            definition_kinds: &[
                "function_definition",
                "struct_specifier",
                "union_specifier",
                "enum_specifier",
                "type_definition",
            ],
            import_kinds: &["preproc_include"],
            symbol_kinds: &["identifier", "type_identifier", "field_identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for C++ syntax facts.
    pub fn cpp() -> Self {
        Self {
            language_id: "cpp",
            language: tree_sitter_cpp::LANGUAGE.into(),
            definition_kinds: &[
                "function_definition",
                "class_specifier",
                "struct_specifier",
                "union_specifier",
                "enum_specifier",
                "namespace_definition",
                "template_declaration",
            ],
            import_kinds: &["preproc_include", "using_declaration"],
            symbol_kinds: &[
                "identifier",
                "type_identifier",
                "field_identifier",
                "namespace_identifier",
            ],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for C# syntax facts.
    pub fn csharp() -> Self {
        Self {
            language_id: "csharp",
            language: tree_sitter_c_sharp::LANGUAGE.into(),
            definition_kinds: &[
                "class_declaration",
                "interface_declaration",
                "struct_declaration",
                "record_declaration",
                "enum_declaration",
                "method_declaration",
                "constructor_declaration",
                "delegate_declaration",
            ],
            import_kinds: &["using_directive"],
            symbol_kinds: &["identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for Java syntax facts.
    pub fn java() -> Self {
        Self {
            language_id: "java",
            language: tree_sitter_java::LANGUAGE.into(),
            definition_kinds: &[
                "class_declaration",
                "interface_declaration",
                "enum_declaration",
                "record_declaration",
                "annotation_type_declaration",
                "method_declaration",
                "constructor_declaration",
            ],
            import_kinds: &["import_declaration"],
            symbol_kinds: &["identifier", "type_identifier", "scoped_identifier"],
            comment_kinds: &["line_comment", "block_comment"],
        }
    }

    /// Adapter for Kotlin syntax facts.
    pub fn kotlin() -> Self {
        Self {
            language_id: "kotlin",
            language: tree_sitter_kotlin_ng::LANGUAGE.into(),
            definition_kinds: &[
                "class_declaration",
                "object_declaration",
                "function_declaration",
                "property_declaration",
            ],
            import_kinds: &["import"],
            symbol_kinds: &["identifier"],
            comment_kinds: &["line_comment", "block_comment"],
        }
    }

    /// Adapter for Go syntax facts.
    pub fn go() -> Self {
        Self {
            language_id: "go",
            language: tree_sitter_go::LANGUAGE.into(),
            definition_kinds: &[
                "function_declaration",
                "method_declaration",
                "type_declaration",
                "const_declaration",
                "var_declaration",
            ],
            import_kinds: &["import_declaration"],
            symbol_kinds: &[
                "identifier",
                "type_identifier",
                "field_identifier",
                "package_identifier",
            ],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for PHP syntax facts.
    pub fn php() -> Self {
        Self {
            language_id: "php",
            language: tree_sitter_php::LANGUAGE_PHP.into(),
            definition_kinds: &[
                "function_definition",
                "class_declaration",
                "interface_declaration",
                "trait_declaration",
                "enum_declaration",
                "method_declaration",
            ],
            import_kinds: &[
                "namespace_use_declaration",
                "include_expression",
                "include_once_expression",
            ],
            symbol_kinds: &["name", "qualified_name"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for TypeScript syntax facts.
    pub fn typescript() -> Self {
        Self {
            language_id: "typescript",
            language: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            definition_kinds: TS_DEFINITION_KINDS,
            import_kinds: &["import_statement"],
            symbol_kinds: &["identifier", "type_identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for TSX syntax facts.
    pub fn tsx() -> Self {
        Self {
            language_id: "tsx",
            language: tree_sitter_typescript::LANGUAGE_TSX.into(),
            definition_kinds: TS_DEFINITION_KINDS,
            import_kinds: &["import_statement"],
            symbol_kinds: &["identifier", "type_identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for JavaScript (and JSX) syntax facts.
    pub fn javascript() -> Self {
        Self {
            language_id: "javascript",
            language: tree_sitter_javascript::LANGUAGE.into(),
            definition_kinds: &[
                "class_declaration",
                "function_declaration",
                "generator_function_declaration",
                "method_definition",
            ],
            import_kinds: &["import_statement"],
            symbol_kinds: &["identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for HTML syntax facts. HTML has no import-like construct, so
    /// `imports` is always empty. `symbol_kinds` is deliberately empty
    /// (LIT-73, same reasoning as `css()`'s LIT-23.2 fix): a tag name or
    /// presentation attribute (`table`, `cellpadding`, `bgcolor`, MSO
    /// namespace attributes like `xmlns:o`) is what the markup *is*, not a
    /// reference to something else, so routing them through the generic
    /// Usages/TypeRefs reference-extraction pass produced one spurious
    /// Unresolved node per tag/attribute name -- most visibly on
    /// Outlook-conditional email templates, which are almost entirely
    /// table-based presentation markup.
    pub fn html() -> Self {
        Self {
            language_id: "html",
            language: tree_sitter_html::LANGUAGE.into(),
            definition_kinds: &["element", "script_element", "style_element"],
            import_kinds: &[],
            symbol_kinds: &[],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for CSS syntax facts. `symbol_kinds` is deliberately empty
    /// (LIT-23.2): class/id selectors are what a rule_set *is*, not a
    /// reference to something else the way an identifier use-site in code
    /// is, so routing them through the generic Usages/TypeRefs
    /// reference-extraction pass (`process_syntax_indexed` in
    /// src/graph/builder.rs) produced one spurious `Usages` relation per
    /// selector -- a category error, not a resolvable fact.
    pub fn css() -> Self {
        Self {
            language_id: "css",
            language: tree_sitter_css::LANGUAGE.into(),
            definition_kinds: &["rule_set", "at_rule"],
            import_kinds: &["import_statement"],
            symbol_kinds: &[],
            comment_kinds: &["comment"],
        }
    }

    /// Adapter for SQL syntax facts. SQL has no import-like construct, so
    /// `imports` is always empty.
    pub fn sql() -> Self {
        Self {
            language_id: "sql",
            language: tree_sitter_sequel::LANGUAGE.into(),
            definition_kinds: &[
                "create_table",
                "create_view",
                "create_materialized_view",
                "create_function",
                "create_index",
                "create_type",
                "create_schema",
                "create_trigger",
            ],
            import_kinds: &[],
            symbol_kinds: &["identifier"],
            comment_kinds: &["comment"],
        }
    }

    /// Stable language id for this adapter.
    pub fn language_id(&self) -> &'static str {
        self.language_id
    }

    /// Parses source text into typed baseline syntax facts. Parser failures
    /// return fallback output instead of panicking.
    pub fn parse(&self, source: &str) -> TreeSitterAdapterOutput {
        self.parse_indexed(source).0
    }

    /// Parses once and returns both the typed baseline facts and the byte
    /// offsets of each top-level definition, so a single parse can serve both
    /// fact extraction and syntax-aware chunking (LIT-86.14). Parser failures
    /// return fallback output and no boundaries instead of panicking.
    pub fn parse_indexed(&self, source: &str) -> (TreeSitterAdapterOutput, Vec<usize>) {
        let mut parser = tree_sitter::Parser::new();
        if let Err(error) = parser.set_language(&self.language) {
            return (
                TreeSitterAdapterOutput::fallback(
                    self.language_id,
                    format!("failed to set tree-sitter language: {error}"),
                ),
                Vec::new(),
            );
        }
        let Some(tree) = parser.parse(source, None) else {
            return (
                TreeSitterAdapterOutput::fallback(
                    self.language_id,
                    "tree-sitter parser returned no tree",
                ),
                Vec::new(),
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
        (
            output,
            self.top_level_definition_boundaries(tree.root_node()),
        )
    }

    /// Byte offsets where each top-level definition begins, sorted and unique.
    /// These are the strong syntax cut points the chunker prefers; only direct
    /// children of the root are used so chunks split *between* declarations,
    /// not inside them.
    fn top_level_definition_boundaries(&self, root: tree_sitter::Node<'_>) -> Vec<usize> {
        let mut boundaries = Vec::new();
        let mut cursor = root.walk();
        for child in root.children(&mut cursor) {
            if self.definition_kinds.contains(&child.kind()) {
                boundaries.push(child.start_byte());
            }
        }
        boundaries.sort_unstable();
        boundaries.dedup();
        boundaries
    }
}

/// `Copy`-able identifier for a language wired through the generic
/// syntax-indexed graph path (see `graph::builder`). Unlike
/// [`LanguageRegistryEntry::id`](crate::inventory::language::LanguageRegistryEntry),
/// which is a `&'static str`, this is a plain enum so it can be used as an
/// [`AnalyzerKind`](crate::analysis::AnalyzerKind) cache-key discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyntaxIndexedLanguage {
    /// C.
    C,
    /// C++.
    Cpp,
    /// C#.
    CSharp,
    /// Java.
    Java,
    /// Kotlin.
    Kotlin,
    /// Go.
    Go,
    /// PHP.
    Php,
    /// TypeScript.
    TypeScript,
    /// TSX.
    Tsx,
    /// JavaScript (and JSX).
    JavaScript,
    /// HTML.
    Html,
    /// CSS.
    Css,
    /// SQL.
    Sql,
}

impl SyntaxIndexedLanguage {
    /// Looks up the variant matching a
    /// [`LanguageRegistryEntry::id`](crate::inventory::language::LanguageRegistryEntry::id).
    pub fn from_registry_id(id: &str) -> Option<Self> {
        Some(match id {
            "c" => Self::C,
            "cpp" => Self::Cpp,
            "c_sharp" => Self::CSharp,
            "java" => Self::Java,
            "kotlin" => Self::Kotlin,
            "go" => Self::Go,
            "php" => Self::Php,
            "typescript" => Self::TypeScript,
            "tsx" => Self::Tsx,
            "javascript" => Self::JavaScript,
            "html" => Self::Html,
            "css" => Self::Css,
            "sql" => Self::Sql,
            _ => return None,
        })
    }

    /// The [`LanguageRegistryEntry::id`](crate::inventory::language::LanguageRegistryEntry::id)
    /// matching this variant. Inverse of [`Self::from_registry_id`].
    pub fn registry_id(self) -> &'static str {
        match self {
            Self::C => "c",
            Self::Cpp => "cpp",
            Self::CSharp => "c_sharp",
            Self::Java => "java",
            Self::Kotlin => "kotlin",
            Self::Go => "go",
            Self::Php => "php",
            Self::TypeScript => "typescript",
            Self::Tsx => "tsx",
            Self::JavaScript => "javascript",
            Self::Html => "html",
            Self::Css => "css",
            Self::Sql => "sql",
        }
    }

    /// Builds the tree-sitter adapter for this language.
    pub fn adapter(self) -> TreeSitterParserAdapter {
        match self {
            Self::C => TreeSitterParserAdapter::c(),
            Self::Cpp => TreeSitterParserAdapter::cpp(),
            Self::CSharp => TreeSitterParserAdapter::csharp(),
            Self::Java => TreeSitterParserAdapter::java(),
            Self::Kotlin => TreeSitterParserAdapter::kotlin(),
            Self::Go => TreeSitterParserAdapter::go(),
            Self::Php => TreeSitterParserAdapter::php(),
            Self::TypeScript => TreeSitterParserAdapter::typescript(),
            Self::Tsx => TreeSitterParserAdapter::tsx(),
            Self::JavaScript => TreeSitterParserAdapter::javascript(),
            Self::Html => TreeSitterParserAdapter::html(),
            Self::Css => TreeSitterParserAdapter::css(),
            Self::Sql => TreeSitterParserAdapter::sql(),
        }
    }
}

/// Parses with an optional adapter, falling back to detected-only output when
/// no parser adapter is available for the language. Test-only helper.
#[cfg(test)]
pub(crate) fn parse_with_optional_adapter(
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
    // LIT-45.4: CommonJS `require("./x")` is an import spelled as a call, so
    // no node-kind list can capture it. Only a literal string argument counts;
    // `require(expr)` names nothing statically and capturing it would send a
    // fabricated reference into the resolver.
    if is_commonjs_require_with_literal(adapter, node, kind, source) {
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

/// True for a `require("literal")` call in a CommonJS-capable language.
///
/// The callee must be exactly the identifier `require` (not `foo.require`)
/// and the sole argument a plain string -- template strings and expressions
/// are dynamic imports whose target is unknowable from syntax (LIT-45.4 AC3).
fn is_commonjs_require_with_literal(
    adapter: &TreeSitterParserAdapter,
    node: tree_sitter::Node<'_>,
    kind: &str,
    source: &str,
) -> bool {
    if kind != "call_expression"
        || !matches!(adapter.language_id, "typescript" | "tsx" | "javascript")
    {
        return false;
    }
    let Some(function) = node.child_by_field_name("function") else {
        return false;
    };
    if function.kind() != "identifier" || node_text(function, source) != "require" {
        return false;
    }
    let Some(arguments) = node.child_by_field_name("arguments") else {
        return false;
    };
    let mut cursor = arguments.walk();
    let named: Vec<_> = arguments
        .children(&mut cursor)
        .filter(|child| child.is_named())
        .collect();
    matches!(named.as_slice(), [argument] if argument.kind() == "string")
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

/// Collects every comment under `root` whose node kind is in
/// `comment_kinds`, at any depth.
///
/// Shared so the analyzers that parse a language themselves (Python, Rust)
/// report comment spans by exactly the same rule as the generic adapter --
/// including its end-of-file clamp (LIT-30/31) -- instead of each growing its
/// own copy.
pub(crate) fn collect_comments(
    root: tree_sitter::Node<'_>,
    source: &str,
    comment_kinds: &[&str],
) -> Vec<TreeSitterComment> {
    let mut comments = Vec::new();
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        for child in node.children(&mut cursor) {
            if comment_kinds.contains(&child.kind())
                && let Some(span) = node_span(child)
            {
                comments.push(TreeSitterComment {
                    text: node_text(child, source),
                    span,
                });
            }
            stack.push(child);
        }
    }
    // Depth-first order is an artifact of the traversal; source order is what
    // a reader and a deterministic snapshot both expect.
    comments.sort_by(|a, b| {
        a.span
            .start_line
            .cmp(&b.span.start_line)
            .then(a.text.cmp(&b.text))
    });
    comments
}

fn node_span(node: tree_sitter::Node<'_>) -> Option<SourceSpan> {
    let start_row = node.start_position().row as u32;
    let end_position = node.end_position();
    // `end_position` is exclusive. When a node's text ends with a newline it
    // points at column 0 of the *following* row, which is one line past the
    // node's last content line -- an unclosed HTML element or a file-final
    // node then reports an end line past EOF and fails `GraphValidator`
    // (LIT-30, LIT-31). Such a node ends on the preceding row instead.
    let ends_on_previous_row = end_position.column == 0 && end_position.row as u32 > start_row;
    let end_row = if ends_on_previous_row {
        end_position.row as u32 - 1
    } else {
        end_position.row as u32
    };
    SourceSpan::new(start_row + 1, end_row.max(start_row) + 1).ok()
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

    /// LIT-45.4: `require("literal")` is an import fact; `require(expr)` and
    /// member `.require(...)` calls are not, and a non-CJS language never
    /// captures calls at all.
    #[test]
    fn commonjs_require_with_a_literal_is_an_import_fact() {
        let source = "\
const local = require('./local');
const pkg = require(\"react\");
const dynamic = require(path.join(base, 'x'));
const template = require(`./tpl-${name}`);
const notGlobal = loader.require('./other');
";
        let output = TreeSitterParserAdapter::javascript().parse(source);
        let imports: Vec<&str> = output
            .imports
            .iter()
            .map(|fact| fact.text.as_str())
            .collect();

        assert_eq!(
            imports,
            vec!["require('./local')", "require(\"react\")"],
            "only literal global require calls may become import facts",
        );

        // The same shapes in TypeScript, plus proof that a language without
        // the convention (Python has a `require` fixture-name collision risk
        // in test corpora) captures nothing.
        let ts = TreeSitterParserAdapter::typescript().parse("const a = require('./b');\n");
        assert_eq!(ts.imports.len(), 1);
        let python = TreeSitterParserAdapter::python().parse("a = require('./b')\n");
        assert!(python.imports.is_empty());
    }

    /// Highest line touched by any fact in the output, mirroring what
    /// evidence-carrying callers persist and `GraphValidator` then checks
    /// against the artifact's inventory line count.
    fn max_end_line(output: &super::TreeSitterAdapterOutput) -> u32 {
        output
            .definitions
            .iter()
            .chain(&output.imports)
            .chain(&output.symbols)
            .map(|fact| fact.span.end_line)
            .chain(output.comments.iter().map(|comment| comment.span.end_line))
            .chain(output.syntax_errors.iter().map(|error| error.span.end_line))
            .max()
            .unwrap_or(0)
    }

    /// A node whose text ends with a newline must not report a span past the
    /// artifact's last line. `tree_sitter::Node::end_position` is exclusive,
    /// so such a node points at column 0 of the *following* row; treating
    /// that row as the end line overshot EOF by one and produced
    /// `InvalidSourceSpan` findings on the pinned corpora (LIT-30, LIT-31).
    ///
    /// This template mirrors the shape of the Flask corpus file that failed:
    /// an unclosed void element (`<hr>`) followed by non-HTML template text
    /// makes tree-sitter-html extend the element node to EOF.
    #[test]
    fn spans_stay_within_the_line_count_when_a_node_ends_with_a_newline() {
        let template = "<article>\n  <p>body</p>\n</article>\n<hr>\n{% endif %}\n{% endblock %}\n";
        let output = TreeSitterParserAdapter::html().parse(template);

        assert_eq!(output.status, TreeSitterParseStatus::Parsed);
        assert_eq!(line_count(template), 6);
        assert!(
            max_end_line(&output) <= line_count(template),
            "span end {} exceeds the {}-line artifact",
            max_end_line(&output),
            line_count(template),
        );
        // The element really does run to the file's last line: the clamp must
        // land exactly on it rather than trimming evidence short.
        assert!(
            output
                .definitions
                .iter()
                .any(|fact| fact.text.starts_with("<hr>") && fact.span.end_line == 6),
            "expected the EOF-spanning element to end on line 6, got {:?}",
            output
                .definitions
                .iter()
                .map(|fact| (fact.kind.as_str(), fact.span.clone()))
                .collect::<Vec<_>>(),
        );

        // The uv corpus header (LIT-31): a trailing newline after a final
        // `#include` reported the include one line past EOF.
        let header = "// leading comment\n// second line\n\n#include <pybind11/pybind11.h>\n";
        let output = TreeSitterParserAdapter::c().parse(header);
        assert_eq!(line_count(header), 4);
        assert!(max_end_line(&output) <= line_count(header));
        assert!(
            output
                .imports
                .iter()
                .any(|fact| fact.kind == "preproc_include" && fact.span.end_line == 4),
            "expected the final include to end on line 4, got {:?}",
            output
                .imports
                .iter()
                .map(|fact| (fact.kind.as_str(), fact.span.clone()))
                .collect::<Vec<_>>(),
        );
    }

    /// LIT-73: an MSO/Outlook-conditional email template is almost entirely
    /// table-based presentation markup (`xmlns:o`, `cellpadding`, `bgcolor`,
    /// `valign`...). Its tag and attribute names must not be mined as code
    /// references -- same reasoning as `css()`'s LIT-23.2 fix -- while its
    /// elements remain real definitions and a template-engine expression
    /// embedded in text content is left alone either way.
    #[test]
    fn html_symbols_stay_empty_for_outlook_conditional_email_markup() {
        let template = "\
<html xmlns:o=\"urn:schemas-microsoft-com:office:office\">
<head><meta charset=\"utf-8\"></head>
<body>
<table role=\"presentation\" cellpadding=\"0\" cellspacing=\"0\" bgcolor=\"#ffffff\">
<tr><td valign=\"top\">{{ user.name }}</td></tr>
</table>
</body>
</html>
";
        let output = TreeSitterParserAdapter::html().parse(template);

        assert_eq!(output.status, TreeSitterParseStatus::Parsed);
        assert!(
            output.symbols.is_empty(),
            "tag/attribute names are markup structure, not code references: {:?}",
            output.symbols
        );
        assert!(
            !output.definitions.is_empty(),
            "element/script/style definitions must stay unaffected by the symbol_kinds change"
        );
    }

    /// The no-final-newline counterpart, mirroring the uv corpus C header
    /// (LIT-31): the last content line is still the end line, so the fix must
    /// not shift spans off the final line.
    #[test]
    fn spans_cover_the_final_line_when_a_file_has_no_trailing_newline() {
        let header = "#ifndef H\n#define H\nint f(void);\n#endif";
        let output = TreeSitterParserAdapter::c().parse(header);

        assert_eq!(output.status, TreeSitterParseStatus::Parsed);
        assert_eq!(line_count(header), 4);
        assert!(max_end_line(&output) <= line_count(header));
        assert!(
            output
                .symbols
                .iter()
                .any(|fact| fact.text == "f" && fact.span.start_line == 3),
            "expected the declaration on line 3 to keep its span",
        );

        let single = "int f(void);";
        let output = TreeSitterParserAdapter::c().parse(single);
        assert_eq!(max_end_line(&output), 1);
        assert_eq!(line_count(single), 1);
    }

    /// Mirrors `inventory::walk::line_count`, the count `GraphValidator`
    /// compares evidence spans against.
    fn line_count(text: &str) -> u32 {
        if text.is_empty() {
            0
        } else {
            text.lines().count().try_into().unwrap_or(u32::MAX)
        }
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

    /// One case per broad-wave language family: source, whether it declares
    /// an import-like construct, and the adapter under test. Each case
    /// asserts a parsed status, at least one definition fact, at least one
    /// comment fact, and (when applicable) at least one import fact --
    /// mirroring LIT-22.2.3 AC1/AC2 without duplicating a full grammar test
    /// suite per language.
    fn broad_wave_cases() -> Vec<(&'static str, &'static str, bool, TreeSitterParserAdapter)> {
        vec![
            (
                "c",
                "// leading comment\n#include <stdio.h>\nstruct Point { int x; int y; };\nint add(int a, int b) { return a + b; }\n",
                true,
                TreeSitterParserAdapter::c(),
            ),
            (
                "cpp",
                "// leading comment\n#include <vector>\nnamespace app { class Greeter { public: void hello(); }; }\n",
                true,
                TreeSitterParserAdapter::cpp(),
            ),
            (
                "csharp",
                "// leading comment\nusing System;\nnamespace App { class Greeter { void Hello() {} } }\n",
                true,
                TreeSitterParserAdapter::csharp(),
            ),
            (
                "java",
                "// leading comment\nimport java.util.List;\nclass Greeter { void hello() {} }\n",
                true,
                TreeSitterParserAdapter::java(),
            ),
            (
                "kotlin",
                "// leading comment\nimport kotlin.collections.List\n\nclass Greeter {\n    fun hello() {}\n}\n",
                true,
                TreeSitterParserAdapter::kotlin(),
            ),
            (
                "go",
                "// leading comment\npackage main\nimport \"fmt\"\nfunc hello() {}\n",
                true,
                TreeSitterParserAdapter::go(),
            ),
            (
                "php",
                "<?php\n// leading comment\nnamespace App;\nuse Foo\\Bar;\nclass Greeter { function hello() {} }\n",
                true,
                TreeSitterParserAdapter::php(),
            ),
            (
                "typescript",
                "// leading comment\nimport { List } from \"immutable\";\nclass Greeter { hello(): void {} }\n",
                true,
                TreeSitterParserAdapter::typescript(),
            ),
            (
                "tsx",
                "// leading comment\nimport { List } from \"immutable\";\nclass Greeter { hello(): void {} }\n",
                true,
                TreeSitterParserAdapter::tsx(),
            ),
            (
                "javascript",
                "// leading comment\nimport { List } from \"immutable\";\nclass Greeter { hello() {} }\n",
                true,
                TreeSitterParserAdapter::javascript(),
            ),
            (
                "html",
                "<!-- leading comment -->\n<html><body><div class=\"greeter\">Hello</div></body></html>\n",
                false,
                TreeSitterParserAdapter::html(),
            ),
            (
                "css",
                "/* leading comment */\n@import url(\"base.css\");\n.greeter { color: red; }\n",
                true,
                TreeSitterParserAdapter::css(),
            ),
            (
                "sql",
                "-- leading comment\nCREATE TABLE greeter (id INT PRIMARY KEY);\n",
                false,
                TreeSitterParserAdapter::sql(),
            ),
        ]
    }

    #[test]
    fn broad_wave_adapters_extract_definitions_and_comments() {
        for (label, source, _expects_import, adapter) in broad_wave_cases() {
            let output = adapter.parse(source);
            assert_eq!(
                output.status,
                TreeSitterParseStatus::Parsed,
                "{label} failed to parse"
            );
            assert!(
                !output.definitions.is_empty(),
                "{label} produced no definitions"
            );
            assert!(!output.comments.is_empty(), "{label} produced no comments");
        }
    }

    #[test]
    fn broad_wave_adapters_extract_imports_where_the_language_has_them() {
        for (label, source, expects_import, adapter) in broad_wave_cases() {
            let output = adapter.parse(source);
            assert_eq!(
                !output.imports.is_empty(),
                expects_import,
                "{label} import expectation mismatch"
            );
        }
    }

    #[test]
    fn syntax_indexed_language_round_trips_registry_ids() {
        for id in [
            "c",
            "cpp",
            "c_sharp",
            "java",
            "kotlin",
            "go",
            "php",
            "typescript",
            "tsx",
            "javascript",
            "html",
            "css",
            "sql",
        ] {
            let Some(language) = super::SyntaxIndexedLanguage::from_registry_id(id) else {
                unreachable!("missing SyntaxIndexedLanguage mapping for {id}");
            };
            assert_eq!(language.registry_id(), id);
            assert_eq!(
                language.adapter().parse("").status,
                TreeSitterParseStatus::Parsed
            );
        }
        assert!(super::SyntaxIndexedLanguage::from_registry_id("ruby").is_none());
    }
}
