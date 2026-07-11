//! Registry-backed language and format support declarations.

use crate::domain::{AnalyzerSelection, ArtifactCategory, SupportTier};
use std::sync::LazyLock;

/// Bump when registry entries, extension mappings, tiers, or analyzer routing
/// change in a way that should invalidate graph planning inputs.
pub const LANGUAGE_REGISTRY_VERSION: u32 = 4;

/// Agent-facing index support level represented by the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RegistryIndexTier {
    /// The language or format is detected and can participate in inventory.
    Detected,
    /// Deterministic syntax or structured facts are extracted.
    SyntaxIndexed,
    /// Syntax facts are eligible for cross-file/package/import refinement.
    HybridResolved,
}

/// Stable registry entry for one language or structured format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageRegistryEntry {
    /// Stable registry id. For codebase-memory parity entries this follows
    /// the vendored grammar directory id where practical.
    pub id: &'static str,
    /// Stable format name used in inventory, graph, and research output.
    pub name: &'static str,
    /// Coarse artifact category for extension-based classification.
    pub category: ArtifactCategory,
    /// Current inventory analyzer tier.
    pub support_tier: SupportTier,
    /// Current agent-facing index tier.
    pub current_tier: RegistryIndexTier,
    /// Target Phase 1 tier from the architecture plan.
    pub target_tier: RegistryIndexTier,
    /// Analyzer selected by this registry entry.
    pub analyzer: AnalyzerSelectionTemplate,
    /// Resolver strategy label stored in graph provenance.
    pub resolver_strategy: &'static str,
    /// File extensions without a leading dot.
    pub extensions: &'static [&'static str],
}

impl LanguageRegistryEntry {
    /// Builds the analyzer selection for this entry.
    pub fn analyzer_selection(&self) -> AnalyzerSelection {
        match self.analyzer {
            AnalyzerSelectionTemplate::Specialized => {
                AnalyzerSelection::Specialized(self.name.to_owned())
            }
            AnalyzerSelectionTemplate::Structured => {
                AnalyzerSelection::Structured(self.name.to_owned())
            }
            AnalyzerSelectionTemplate::SyntaxIndexed => {
                AnalyzerSelection::SyntaxIndexed(self.id.to_owned())
            }
            AnalyzerSelectionTemplate::GenericText => AnalyzerSelection::GenericText,
            AnalyzerSelectionTemplate::Opaque => AnalyzerSelection::Opaque,
        }
    }

    /// Reports the parser binding selected by the production registry.
    pub fn parser_availability(&self) -> ParserAvailability {
        match self.analyzer {
            AnalyzerSelectionTemplate::SyntaxIndexed
                if crate::analysis::SyntaxIndexedLanguage::from_registry_id(self.id).is_some() =>
            {
                ParserAvailability::TreeSitter
            }
            AnalyzerSelectionTemplate::SyntaxIndexed => ParserAvailability::Missing,
            AnalyzerSelectionTemplate::GenericText | AnalyzerSelectionTemplate::Opaque => {
                ParserAvailability::Missing
            }
            AnalyzerSelectionTemplate::Specialized | AnalyzerSelectionTemplate::Structured => {
                ParserAvailability::Dedicated
            }
        }
    }

    /// Reports capabilities without exposing builder-specific dispatch rules.
    pub fn extraction_capabilities(&self) -> ExtractionCapabilities {
        let available = self.parser_availability() != ParserAvailability::Missing;
        ExtractionCapabilities {
            declarations: available,
            imports: available,
            symbols: available,
            syntax: available,
        }
    }

    /// Safe fallback for parser-missing entries.
    pub fn fallback(&self) -> ExtractionFallback {
        match self.analyzer {
            AnalyzerSelectionTemplate::Opaque => ExtractionFallback::ArtifactOnly,
            _ => ExtractionFallback::GenericText,
        }
    }
}

/// Serializable-free analyzer template used by static registry entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzerSelectionTemplate {
    /// Specialized language analyzer.
    Specialized,
    /// Structured format analyzer.
    Structured,
    /// Generic tree-sitter syntax-indexed analyzer.
    SyntaxIndexed,
    /// Generic text analyzer.
    GenericText,
    /// Opaque metadata-only analyzer.
    Opaque,
}

/// Whether the registry entry has a parser available in this build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParserAvailability {
    /// A dedicated production analyzer owns this language or format.
    Dedicated,
    /// A generic tree-sitter adapter is wired for the registry id.
    TreeSitter,
    /// The entry is known for inventory purposes but has no parser binding.
    Missing,
}

/// Facts a production extractor can safely emit for a registry entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExtractionCapabilities {
    /// Declaration-like syntax facts are available.
    pub declarations: bool,
    /// Import/include/use facts are available.
    pub imports: bool,
    /// Identifier/symbol facts are available.
    pub symbols: bool,
    /// Comments and syntax diagnostics are available.
    pub syntax: bool,
}

/// Safe behavior when the selected parser cannot produce facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractionFallback {
    /// Preserve the artifact node and use generic text findings when safe.
    GenericText,
    /// Preserve only the structured artifact node.
    ArtifactOnly,
}

/// Registry covering Lithograph's current support plus codebase-memory's
/// visible vendored grammar inventory.
pub static LANGUAGE_REGISTRY: LazyLock<Vec<LanguageRegistryEntry>> = LazyLock::new(|| {
    let mut entries = CODEBASE_MEMORY_REGISTRY.to_vec();
    for id in CODEBASE_MEMORY_GRAMMAR_IDS {
        if entries.iter().any(|entry| entry.id == *id) {
            continue;
        }
        entries.push(syntax_target(id, id, ArtifactCategory::SourceCode, &[]));
    }
    entries
});

const CODEBASE_MEMORY_REGISTRY: &[LanguageRegistryEntry] = &[
    hybrid_current("python", &["py"]),
    hybrid_current("rust", &["rs"]),
    deep_syntax_indexed_hybrid_target("typescript", &["ts", "mts", "cts"]),
    deep_syntax_indexed_hybrid_target("tsx", &["tsx"]),
    syntax_indexed_hybrid_target("javascript", &["js", "jsx", "mjs", "cjs"]),
    syntax_indexed_hybrid_target("go", &["go"]),
    syntax_indexed_hybrid_target("java", &["java"]),
    syntax_indexed_hybrid_target("kotlin", &["kt", "kts"]),
    syntax_indexed_hybrid_target_with_id("c_sharp", "csharp", &["cs"]),
    syntax_indexed_hybrid_target("php", &["php"]),
    syntax_indexed_hybrid_target("c", &["c"]),
    syntax_indexed_hybrid_target_with_id(
        "cpp",
        "cpp",
        &[
            "cc", "ccm", "cpp", "cppm", "cxx", "h", "hh", "hpp", "hxx", "ixx",
        ],
    ),
    syntax_current(
        "markdown",
        ArtifactCategory::Documentation,
        &["md", "markdown", "mdx"],
    ),
    syntax_current("yaml", ArtifactCategory::Configuration, &["yaml", "yml"]),
    syntax_current("json", ArtifactCategory::Configuration, &["json"]),
    syntax_current("toml", ArtifactCategory::Configuration, &["toml"]),
    syntax_current(
        "dockerfile",
        ArtifactCategory::ContainerDefinition,
        &["dockerfile"],
    ),
    syntax_current_with_id(
        "docker-compose",
        "docker-compose",
        ArtifactCategory::ContainerDefinition,
        &[],
    ),
    syntax_current_with_id(
        "github-actions",
        "github-actions",
        ArtifactCategory::ContinuousIntegration,
        &[],
    ),
    syntax_indexed_current("sql", "sql", ArtifactCategory::DatabaseSchema, &["sql"]),
    syntax_indexed_current("html", "html", ArtifactCategory::Template, &["html", "htm"]),
    syntax_indexed_current("css", "css", ArtifactCategory::StaticAsset, &["css"]),
    syntax_target("scss", "scss", ArtifactCategory::StaticAsset, &["scss"]),
    syntax_target("bash", "bash", ArtifactCategory::Script, &["bash", "sh"]),
    syntax_target(
        "clojure",
        "clojure",
        ArtifactCategory::SourceCode,
        &["clj", "cljc", "cljs"],
    ),
    syntax_target(
        "cmake",
        "cmake",
        ArtifactCategory::BuildDefinition,
        &["cmake"],
    ),
    syntax_target(
        "cobol",
        "cobol",
        ArtifactCategory::SourceCode,
        &["cbl", "cob"],
    ),
    syntax_target(
        "commonlisp",
        "commonlisp",
        ArtifactCategory::SourceCode,
        &["cl", "lisp", "lsp"],
    ),
    syntax_target("cuda", "cuda", ArtifactCategory::SourceCode, &["cu", "cuh"]),
    syntax_target("dart", "dart", ArtifactCategory::SourceCode, &["dart"]),
    syntax_target(
        "dotenv",
        "dotenv",
        ArtifactCategory::Configuration,
        &["env"],
    ),
    syntax_target("elisp", "emacslisp", ArtifactCategory::SourceCode, &["el"]),
    syntax_target(
        "elixir",
        "elixir",
        ArtifactCategory::SourceCode,
        &["ex", "exs"],
    ),
    syntax_target("elm", "elm", ArtifactCategory::SourceCode, &["elm"]),
    syntax_target("erlang", "erlang", ArtifactCategory::SourceCode, &["erl"]),
    syntax_target(
        "fsharp",
        "fsharp",
        ArtifactCategory::SourceCode,
        &["fs", "fsi", "fsx"],
    ),
    syntax_target(
        "form",
        "form",
        ArtifactCategory::SourceCode,
        &["frm", "prc"],
    ),
    syntax_target(
        "fortran",
        "fortran",
        ArtifactCategory::SourceCode,
        &["f03", "f08", "f90", "f95"],
    ),
    syntax_target(
        "glsl",
        "glsl",
        ArtifactCategory::SourceCode,
        &["frag", "glsl", "vert"],
    ),
    protocol_current(
        "graphql",
        "graphql",
        ArtifactCategory::Configuration,
        &["gql", "graphql"],
    ),
    syntax_target(
        "groovy",
        "groovy",
        ArtifactCategory::SourceCode,
        &["gradle", "groovy"],
    ),
    syntax_target("haskell", "haskell", ArtifactCategory::SourceCode, &["hs"]),
    syntax_target(
        "hcl",
        "hcl",
        ArtifactCategory::Configuration,
        &["hcl", "tf"],
    ),
    syntax_target(
        "ini",
        "ini",
        ArtifactCategory::Configuration,
        &["cfg", "conf", "ini"],
    ),
    syntax_target("julia", "julia", ArtifactCategory::SourceCode, &["jl"]),
    syntax_target("lean", "lean", ArtifactCategory::SourceCode, &["lean"]),
    syntax_target("lua", "lua", ArtifactCategory::SourceCode, &["lua"]),
    syntax_target(
        "magma",
        "magma",
        ArtifactCategory::SourceCode,
        &["mag", "magma"],
    ),
    syntax_target(
        "make",
        "makefile",
        ArtifactCategory::BuildDefinition,
        &["mk"],
    ),
    syntax_target(
        "matlab",
        "matlab",
        ArtifactCategory::SourceCode,
        &["m", "matlab", "mlx"],
    ),
    syntax_target(
        "meson",
        "meson",
        ArtifactCategory::BuildDefinition,
        &["meson"],
    ),
    syntax_target("nix", "nix", ArtifactCategory::Configuration, &["nix"]),
    syntax_target(
        "ocaml",
        "ocaml",
        ArtifactCategory::SourceCode,
        &["ml", "mli"],
    ),
    syntax_target("perl", "perl", ArtifactCategory::SourceCode, &["pl", "pm"]),
    protocol_current(
        "protobuf",
        "protobuf",
        ArtifactCategory::SourceCode,
        &["proto"],
    ),
    syntax_target("r", "r", ArtifactCategory::SourceCode, &["R", "r"]),
    syntax_target(
        "ruby",
        "ruby",
        ArtifactCategory::SourceCode,
        &["gemspec", "rake", "rb"],
    ),
    syntax_target(
        "scala",
        "scala",
        ArtifactCategory::SourceCode,
        &["sc", "scala"],
    ),
    syntax_target("svelte", "svelte", ArtifactCategory::Template, &["svelte"]),
    syntax_target("swift", "swift", ArtifactCategory::SourceCode, &["swift"]),
    syntax_target("verilog", "verilog", ArtifactCategory::SourceCode, &["v"]),
    syntax_target(
        "vim",
        "vimscript",
        ArtifactCategory::SourceCode,
        &["vim", "vimrc"],
    ),
    syntax_target("vue", "vue", ArtifactCategory::Template, &["vue"]),
    syntax_target(
        "wolfram",
        "wolfram",
        ArtifactCategory::SourceCode,
        &["wl", "wls"],
    ),
    syntax_target(
        "xml",
        "xml",
        ArtifactCategory::Configuration,
        &["xml", "xsd", "xsl", "svg"],
    ),
    syntax_target("ada", "ada", ArtifactCategory::SourceCode, &["adb", "ads"]),
    syntax_target("agda", "agda", ArtifactCategory::SourceCode, &["agda"]),
    syntax_target(
        "apex",
        "apex",
        ArtifactCategory::SourceCode,
        &["cls", "trigger"],
    ),
    syntax_target("astro", "astro", ArtifactCategory::Template, &["astro"]),
    syntax_target("awk", "awk", ArtifactCategory::Script, &["awk"]),
    syntax_target(
        "beancount",
        "beancount",
        ArtifactCategory::Configuration,
        &["beancount"],
    ),
    syntax_target(
        "bibtex",
        "bibtex",
        ArtifactCategory::Configuration,
        &["bib"],
    ),
    syntax_target(
        "bicep",
        "bicep",
        ArtifactCategory::Configuration,
        &["bicep"],
    ),
    syntax_target(
        "bitbake",
        "bitbake",
        ArtifactCategory::BuildDefinition,
        &["bb", "bbappend", "bbclass", "inc"],
    ),
    syntax_target("blade", "blade", ArtifactCategory::Template, &["blade.php"]),
    syntax_target("cairo", "cairo", ArtifactCategory::SourceCode, &["cairo"]),
    syntax_target("capnp", "capnp", ArtifactCategory::SourceCode, &["capnp"]),
    syntax_target(
        "cfscript",
        "cfscript",
        ArtifactCategory::SourceCode,
        &["cfc"],
    ),
    syntax_target("cfml", "cfml", ArtifactCategory::Template, &["cfm"]),
    syntax_target("crystal", "crystal", ArtifactCategory::SourceCode, &["cr"]),
    syntax_target("csv", "csv", ArtifactCategory::Configuration, &["csv"]),
    syntax_target("d", "d", ArtifactCategory::SourceCode, &["d"]),
    syntax_target(
        "devicetree",
        "devicetree",
        ArtifactCategory::Configuration,
        &["dts", "dtsi", "overlay"],
    ),
    syntax_target(
        "diff",
        "diff",
        ArtifactCategory::Documentation,
        &["diff", "patch"],
    ),
    syntax_target("fennel", "fennel", ArtifactCategory::SourceCode, &["fnl"]),
    syntax_target("fish", "fish", ArtifactCategory::Script, &["fish"]),
    syntax_target("func", "func", ArtifactCategory::SourceCode, &["fc"]),
    syntax_target(
        "gdscript",
        "gdscript",
        ArtifactCategory::SourceCode,
        &["gd"],
    ),
    syntax_target("gleam", "gleam", ArtifactCategory::SourceCode, &["gleam"]),
    syntax_target(
        "gn",
        "gn",
        ArtifactCategory::BuildDefinition,
        &["gn", "gni"],
    ),
    syntax_target(
        "gotemplate",
        "gotemplate",
        ArtifactCategory::Template,
        &["gotmpl", "tpl", "tmpl"],
    ),
    syntax_target("hare", "hare", ArtifactCategory::SourceCode, &["ha"]),
    syntax_target(
        "hlsl",
        "hlsl",
        ArtifactCategory::SourceCode,
        &["fx", "hlsl", "hlsli"],
    ),
    syntax_target(
        "hyprlang",
        "hyprlang",
        ArtifactCategory::Configuration,
        &["hl"],
    ),
    syntax_target("ispc", "ispc", ArtifactCategory::SourceCode, &["ispc"]),
    syntax_target("janet", "janet", ArtifactCategory::SourceCode, &["janet"]),
    syntax_target(
        "jinja2",
        "jinja2",
        ArtifactCategory::Template,
        &["j2", "jinja", "jinja2"],
    ),
    syntax_target(
        "json5",
        "json5",
        ArtifactCategory::Configuration,
        &["json5"],
    ),
    syntax_target(
        "jsonnet",
        "jsonnet",
        ArtifactCategory::Configuration,
        &["jsonnet", "libsonnet"],
    ),
    syntax_target("kdl", "kdl", ArtifactCategory::Configuration, &["kdl"]),
    syntax_target(
        "linkerscript",
        "linkerscript",
        ArtifactCategory::BuildDefinition,
        &["ld", "lds"],
    ),
    syntax_target("liquid", "liquid", ArtifactCategory::Template, &["liquid"]),
    syntax_target("llvm", "llvm", ArtifactCategory::SourceCode, &["ll"]),
    syntax_target("luau", "luau", ArtifactCategory::SourceCode, &["luau"]),
    syntax_target(
        "mermaid",
        "mermaid",
        ArtifactCategory::Documentation,
        &["mermaid", "mmd"],
    ),
    syntax_target("move", "move", ArtifactCategory::SourceCode, &["move"]),
    syntax_target("nasm", "nasm", ArtifactCategory::SourceCode, &["nasm"]),
    syntax_target(
        "nickel",
        "nickel",
        ArtifactCategory::Configuration,
        &["ncl"],
    ),
    syntax_target("odin", "odin", ArtifactCategory::SourceCode, &["odin"]),
    syntax_target(
        "pascal",
        "pascal",
        ArtifactCategory::SourceCode,
        &["dpr", "lpr", "pas"],
    ),
    syntax_target("pine", "pine", ArtifactCategory::SourceCode, &["pine"]),
    syntax_target("pkl", "pkl", ArtifactCategory::Configuration, &["pkl"]),
    syntax_target("po", "po", ArtifactCategory::Documentation, &["po", "pot"]),
    syntax_target("pony", "pony", ArtifactCategory::SourceCode, &["pony"]),
    syntax_target(
        "powershell",
        "powershell",
        ArtifactCategory::Script,
        &["ps1", "psd1", "psm1"],
    ),
    syntax_target(
        "prisma",
        "prisma",
        ArtifactCategory::Configuration,
        &["prisma"],
    ),
    syntax_target(
        "properties",
        "properties",
        ArtifactCategory::Configuration,
        &["properties"],
    ),
    syntax_target("puppet", "puppet", ArtifactCategory::Configuration, &["pp"]),
    syntax_target(
        "purescript",
        "purescript",
        ArtifactCategory::SourceCode,
        &["purs"],
    ),
    syntax_target("qml", "qml", ArtifactCategory::SourceCode, &["qml"]),
    syntax_target("racket", "racket", ArtifactCategory::SourceCode, &["rkt"]),
    syntax_target("regex", "regex", ArtifactCategory::Configuration, &["re"]),
    syntax_target(
        "rescript",
        "rescript",
        ArtifactCategory::SourceCode,
        &["res", "resi"],
    ),
    syntax_target("ron", "ron", ArtifactCategory::Configuration, &["ron"]),
    syntax_target("rst", "rst", ArtifactCategory::Documentation, &["rst"]),
    syntax_target(
        "assembly",
        "assembly",
        ArtifactCategory::SourceCode,
        &["s", "S"],
    ),
    syntax_target(
        "scheme",
        "scheme",
        ArtifactCategory::SourceCode,
        &["scm", "ss"],
    ),
    syntax_target("slang", "slang", ArtifactCategory::SourceCode, &["slang"]),
    syntax_target("smali", "smali", ArtifactCategory::SourceCode, &["smali"]),
    syntax_target(
        "smithy",
        "smithy",
        ArtifactCategory::Configuration,
        &["smithy"],
    ),
    syntax_target(
        "solidity",
        "solidity",
        ArtifactCategory::SourceCode,
        &["sol"],
    ),
    syntax_target("soql", "soql", ArtifactCategory::SourceCode, &["soql"]),
    syntax_target("sosl", "sosl", ArtifactCategory::SourceCode, &["sosl"]),
    syntax_target(
        "starlark",
        "starlark",
        ArtifactCategory::BuildDefinition,
        &["bzl", "star"],
    ),
    syntax_target(
        "squirrel",
        "squirrel",
        ArtifactCategory::SourceCode,
        &["nut"],
    ),
    syntax_target("sway", "sway", ArtifactCategory::SourceCode, &["sw"]),
    syntax_target(
        "systemverilog",
        "systemverilog",
        ArtifactCategory::SourceCode,
        &["sv"],
    ),
    syntax_target(
        "tablegen",
        "tablegen",
        ArtifactCategory::SourceCode,
        &["td"],
    ),
    syntax_target("tcl", "tcl", ArtifactCategory::SourceCode, &["tcl"]),
    syntax_target("teal", "teal", ArtifactCategory::SourceCode, &["tl"]),
    syntax_target("templ", "templ", ArtifactCategory::Template, &["templ"]),
    syntax_target(
        "thrift",
        "thrift",
        ArtifactCategory::SourceCode,
        &["thrift"],
    ),
    syntax_target("tlaplus", "tlaplus", ArtifactCategory::SourceCode, &["tla"]),
    syntax_target("typst", "typst", ArtifactCategory::Documentation, &["typ"]),
    syntax_target(
        "vhdl",
        "vhdl",
        ArtifactCategory::SourceCode,
        &["vhd", "vhdl"],
    ),
    syntax_target("wgsl", "wgsl", ArtifactCategory::SourceCode, &["wgsl"]),
    syntax_target("wit", "wit", ArtifactCategory::SourceCode, &["wit"]),
    syntax_target("zig", "zig", ArtifactCategory::SourceCode, &["zig"]),
    syntax_target("zsh", "zsh", ArtifactCategory::Script, &["zsh"]),
];

const CODEBASE_MEMORY_GRAMMAR_IDS: &[&str] = &[
    "ada",
    "agda",
    "apex",
    "assembly",
    "astro",
    "awk",
    "bash",
    "beancount",
    "bibtex",
    "bicep",
    "bitbake",
    "blade",
    "c",
    "c_sharp",
    "cairo",
    "capnp",
    "cfml",
    "cfscript",
    "clojure",
    "cmake",
    "cobol",
    "commonlisp",
    "cpp",
    "crystal",
    "css",
    "csv",
    "cuda",
    "d",
    "dart",
    "devicetree",
    "diff",
    "dockerfile",
    "dotenv",
    "elisp",
    "elixir",
    "elm",
    "erlang",
    "fennel",
    "fish",
    "form",
    "fortran",
    "fsharp",
    "func",
    "gdscript",
    "gitattributes",
    "gitignore",
    "gleam",
    "glsl",
    "gn",
    "go",
    "gomod",
    "gotemplate",
    "graphql",
    "groovy",
    "hare",
    "haskell",
    "hcl",
    "hlsl",
    "html",
    "hyprlang",
    "ini",
    "ispc",
    "janet",
    "java",
    "javascript",
    "jinja2",
    "jsdoc",
    "json",
    "json5",
    "jsonnet",
    "julia",
    "just",
    "kconfig",
    "kdl",
    "kotlin",
    "lean",
    "linkerscript",
    "liquid",
    "llvm",
    "lua",
    "luau",
    "magma",
    "make",
    "markdown",
    "matlab",
    "mermaid",
    "meson",
    "mojo",
    "move",
    "nasm",
    "nickel",
    "nix",
    "objc",
    "objectscript_routine",
    "objectscript_udl",
    "ocaml",
    "odin",
    "pascal",
    "perl",
    "php",
    "pine",
    "pkl",
    "po",
    "pony",
    "powershell",
    "prisma",
    "properties",
    "protobuf",
    "puppet",
    "purescript",
    "python",
    "qml",
    "r",
    "racket",
    "regex",
    "requirements",
    "rescript",
    "ron",
    "rst",
    "ruby",
    "rust",
    "scala",
    "scheme",
    "scss",
    "slang",
    "smali",
    "smithy",
    "solidity",
    "soql",
    "sosl",
    "sql",
    "squirrel",
    "sshconfig",
    "starlark",
    "svelte",
    "sway",
    "swift",
    "systemverilog",
    "tablegen",
    "tcl",
    "teal",
    "templ",
    "thrift",
    "tlaplus",
    "toml",
    "tsx",
    "typescript",
    "typst",
    "verilog",
    "vhdl",
    "vim",
    "vue",
    "wgsl",
    "wit",
    "wolfram",
    "xml",
    "yaml",
    "zig",
    "zsh",
];

const fn hybrid_current(
    name: &'static str,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    hybrid_current_with_id(name, name, extensions)
}

const fn hybrid_current_with_id(
    id: &'static str,
    name: &'static str,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id,
        name,
        category: ArtifactCategory::SourceCode,
        support_tier: SupportTier::DeepLanguage,
        current_tier: RegistryIndexTier::HybridResolved,
        target_tier: RegistryIndexTier::HybridResolved,
        analyzer: AnalyzerSelectionTemplate::Specialized,
        resolver_strategy: "specialized-hybrid",
        extensions,
    }
}

/// A source language with a wired [`TreeSitterParserAdapter`](crate::analysis::TreeSitterParserAdapter)
/// (`current_tier: SyntaxIndexed`) still awaiting cross-file hybrid
/// resolution (`target_tier: HybridResolved`, see LIT-22.3).
const fn syntax_indexed_hybrid_target(
    name: &'static str,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    syntax_indexed_hybrid_target_with_id(name, name, extensions)
}

const fn syntax_indexed_hybrid_target_with_id(
    id: &'static str,
    name: &'static str,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id,
        name,
        category: ArtifactCategory::SourceCode,
        support_tier: SupportTier::StructuredFormat,
        current_tier: RegistryIndexTier::SyntaxIndexed,
        target_tier: RegistryIndexTier::HybridResolved,
        analyzer: AnalyzerSelectionTemplate::SyntaxIndexed,
        resolver_strategy: "syntax-indexed-treesitter",
        extensions,
    }
}

/// A language with a specialized declaration analyzer but whose cross-file
/// resolution remains a syntax-indexed target until its resolver ships.
const fn deep_syntax_indexed_hybrid_target(
    name: &'static str,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id: name,
        name,
        category: ArtifactCategory::SourceCode,
        support_tier: SupportTier::DeepLanguage,
        current_tier: RegistryIndexTier::SyntaxIndexed,
        target_tier: RegistryIndexTier::HybridResolved,
        analyzer: AnalyzerSelectionTemplate::Specialized,
        resolver_strategy: "typescript-deep-syntax",
        extensions,
    }
}

/// A structured/query format with a wired
/// [`TreeSitterParserAdapter`](crate::analysis::TreeSitterParserAdapter) and
/// no further hybrid-resolution target (`current_tier` ==  `target_tier` ==
/// `SyntaxIndexed`), e.g. HTML, CSS, and SQL.
const fn syntax_indexed_current(
    id: &'static str,
    name: &'static str,
    category: ArtifactCategory,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id,
        name,
        category,
        support_tier: SupportTier::StructuredFormat,
        current_tier: RegistryIndexTier::SyntaxIndexed,
        target_tier: RegistryIndexTier::SyntaxIndexed,
        analyzer: AnalyzerSelectionTemplate::SyntaxIndexed,
        resolver_strategy: "syntax-indexed-treesitter",
        extensions,
    }
}

const fn syntax_current(
    name: &'static str,
    category: ArtifactCategory,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    syntax_current_with_id(name, name, category, extensions)
}

const fn syntax_current_with_id(
    id: &'static str,
    name: &'static str,
    category: ArtifactCategory,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id,
        name,
        category,
        support_tier: SupportTier::StructuredFormat,
        current_tier: RegistryIndexTier::SyntaxIndexed,
        target_tier: RegistryIndexTier::SyntaxIndexed,
        analyzer: AnalyzerSelectionTemplate::Structured,
        resolver_strategy: "structured-syntax",
        extensions,
    }
}

/// A protocol schema format with a wired `ProtoAnalyzer`/`GraphQlAnalyzer`
/// (LIT-22.3.4): `current_tier` == `target_tier` == `SyntaxIndexed`, like
/// `syntax_current`, but routed through `AnalyzerSelectionTemplate::Specialized`
/// since these formats have a dedicated analyzer rather than going through
/// the generic `StructuredAnalyzer`.
const fn protocol_current(
    id: &'static str,
    name: &'static str,
    category: ArtifactCategory,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id,
        name,
        category,
        support_tier: SupportTier::StructuredFormat,
        current_tier: RegistryIndexTier::SyntaxIndexed,
        target_tier: RegistryIndexTier::SyntaxIndexed,
        analyzer: AnalyzerSelectionTemplate::Specialized,
        resolver_strategy: "protocol-schema",
        extensions,
    }
}

const fn syntax_target(
    id: &'static str,
    name: &'static str,
    category: ArtifactCategory,
    extensions: &'static [&'static str],
) -> LanguageRegistryEntry {
    LanguageRegistryEntry {
        id,
        name,
        category,
        support_tier: SupportTier::GenericText,
        current_tier: RegistryIndexTier::Detected,
        target_tier: RegistryIndexTier::SyntaxIndexed,
        analyzer: AnalyzerSelectionTemplate::GenericText,
        resolver_strategy: "generic-text-fallback",
        extensions,
    }
}

/// Looks up a registry entry by stable language/format id.
pub fn by_id(id: &str) -> Option<&'static LanguageRegistryEntry> {
    LANGUAGE_REGISTRY.iter().find(|entry| entry.id == id)
}

/// Looks up a registry entry by stable language/format name or id.
pub fn by_name(name: &str) -> Option<&'static LanguageRegistryEntry> {
    LANGUAGE_REGISTRY
        .iter()
        .find(|entry| entry.name == name || entry.id == name)
}

/// Looks up a registry entry by file extension without a leading dot.
pub fn by_extension(extension: &str) -> Option<&'static LanguageRegistryEntry> {
    LANGUAGE_REGISTRY
        .iter()
        .find(|entry| entry.extensions.contains(&extension))
}

#[cfg(test)]
mod tests {
    use super::{
        CODEBASE_MEMORY_GRAMMAR_IDS, ExtractionFallback, LANGUAGE_REGISTRY, ParserAvailability,
        RegistryIndexTier, by_extension, by_id, by_name,
    };
    use crate::domain::{AnalyzerSelection, ArtifactCategory, SupportTier};
    use std::collections::{BTreeMap, BTreeSet};

    #[test]
    fn registry_covers_codebase_memory_grammar_inventory() -> Result<(), Box<dyn std::error::Error>>
    {
        assert!(LANGUAGE_REGISTRY.len() >= CODEBASE_MEMORY_GRAMMAR_IDS.len());
        for id in CODEBASE_MEMORY_GRAMMAR_IDS {
            by_id(id).ok_or_else(|| {
                std::io::Error::other(format!("missing codebase-memory grammar id {id}"))
            })?;
        }

        Ok(())
    }

    #[test]
    fn registry_covers_phase_one_language_extensions() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            ("py", "python", RegistryIndexTier::HybridResolved),
            ("rs", "rust", RegistryIndexTier::HybridResolved),
            ("ts", "typescript", RegistryIndexTier::SyntaxIndexed),
            ("tsx", "tsx", RegistryIndexTier::SyntaxIndexed),
            ("js", "javascript", RegistryIndexTier::SyntaxIndexed),
            ("jsx", "javascript", RegistryIndexTier::SyntaxIndexed),
            ("go", "go", RegistryIndexTier::SyntaxIndexed),
            ("java", "java", RegistryIndexTier::SyntaxIndexed),
            ("kt", "kotlin", RegistryIndexTier::SyntaxIndexed),
            ("cs", "csharp", RegistryIndexTier::SyntaxIndexed),
            ("php", "php", RegistryIndexTier::SyntaxIndexed),
        ];

        for (extension, name, current_tier) in cases {
            let entry = by_extension(extension).ok_or_else(|| {
                std::io::Error::other(format!("missing registry extension {extension}"))
            })?;
            assert_eq!(entry.name, name);
            assert_eq!(entry.category, ArtifactCategory::SourceCode);
            assert_eq!(entry.current_tier, current_tier);
            assert_eq!(entry.target_tier, RegistryIndexTier::HybridResolved);
        }

        Ok(())
    }

    #[test]
    fn registry_separates_current_and_target_support() -> Result<(), Box<dyn std::error::Error>> {
        let rust =
            by_name("rust").ok_or_else(|| std::io::Error::other("missing rust registry entry"))?;
        assert_eq!(rust.id, "rust");
        assert_eq!(rust.support_tier, SupportTier::DeepLanguage);
        assert_eq!(
            rust.analyzer_selection(),
            AnalyzerSelection::Specialized("rust".to_owned())
        );

        let go = by_name("go").ok_or_else(|| std::io::Error::other("missing go registry entry"))?;
        assert_eq!(go.support_tier, SupportTier::StructuredFormat);
        assert_eq!(go.current_tier, RegistryIndexTier::SyntaxIndexed);
        assert_eq!(go.target_tier, RegistryIndexTier::HybridResolved);
        assert_eq!(
            go.analyzer_selection(),
            AnalyzerSelection::SyntaxIndexed("go".to_owned())
        );

        let sql =
            by_name("sql").ok_or_else(|| std::io::Error::other("missing sql registry entry"))?;
        assert_eq!(sql.current_tier, RegistryIndexTier::SyntaxIndexed);
        assert_eq!(sql.target_tier, RegistryIndexTier::SyntaxIndexed);

        Ok(())
    }

    #[test]
    fn registry_reports_parser_capabilities_and_safe_fallbacks()
    -> Result<(), Box<dyn std::error::Error>> {
        let rust = by_name("rust").ok_or("missing rust")?;
        assert_eq!(rust.parser_availability(), ParserAvailability::Dedicated);
        assert!(rust.extraction_capabilities().declarations);

        let go = by_name("go").ok_or("missing go")?;
        assert_eq!(go.parser_availability(), ParserAvailability::TreeSitter);
        assert!(go.extraction_capabilities().imports);

        let prisma = by_name("prisma").ok_or("missing prisma")?;
        assert_eq!(prisma.parser_availability(), ParserAvailability::Missing);
        assert_eq!(prisma.fallback(), ExtractionFallback::GenericText);
        assert!(!prisma.extraction_capabilities().syntax);
        Ok(())
    }

    #[test]
    fn registry_exposes_stable_id_and_codebase_memory_aliases()
    -> Result<(), Box<dyn std::error::Error>> {
        let csharp = by_id("c_sharp")
            .ok_or_else(|| std::io::Error::other("missing c_sharp registry entry"))?;
        assert_eq!(csharp.name, "csharp");
        assert_eq!(by_name("csharp"), Some(csharp));
        assert_eq!(by_extension("cs"), Some(csharp));
        assert_eq!(csharp.target_tier, RegistryIndexTier::HybridResolved);

        let cpp =
            by_id("cpp").ok_or_else(|| std::io::Error::other("missing cpp registry entry"))?;
        assert_eq!(cpp.name, "cpp");
        assert_eq!(by_extension("hpp"), Some(cpp));
        assert_eq!(cpp.target_tier, RegistryIndexTier::HybridResolved);

        let prisma = by_id("prisma")
            .ok_or_else(|| std::io::Error::other("missing prisma registry entry"))?;
        assert_eq!(prisma.current_tier, RegistryIndexTier::Detected);
        assert_eq!(prisma.target_tier, RegistryIndexTier::SyntaxIndexed);

        Ok(())
    }

    #[test]
    fn every_current_tier_above_detected_has_a_runnable_analyzer()
    -> Result<(), Box<dyn std::error::Error>> {
        // LIT-22.2.3 AC3: a registry entry may only claim `SyntaxIndexed` or
        // `HybridResolved` for `current_tier` when some analyzer actually
        // backs that claim -- a specialized analyzer (python/rust), a
        // structured-format analyzer, or a wired tree-sitter adapter. Every
        // other entry (the long tail of grammar ids with no adapter yet)
        // must stay at `Detected`, never overclaim.
        for entry in LANGUAGE_REGISTRY.iter() {
            if entry.current_tier == RegistryIndexTier::Detected {
                continue;
            }
            let backed = matches!(
                entry.analyzer,
                super::AnalyzerSelectionTemplate::Specialized
                    | super::AnalyzerSelectionTemplate::Structured
                    | super::AnalyzerSelectionTemplate::SyntaxIndexed
            );
            if !backed {
                return Err(std::io::Error::other(format!(
                    "registry entry {} claims current_tier {:?} without a backing analyzer",
                    entry.id, entry.current_tier
                ))
                .into());
            }
        }

        Ok(())
    }

    #[test]
    fn registry_has_no_duplicate_ids_names_or_extensions() -> Result<(), Box<dyn std::error::Error>>
    {
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut extensions: BTreeMap<&str, &str> = BTreeMap::new();

        for entry in LANGUAGE_REGISTRY.iter() {
            if !ids.insert(entry.id) {
                return Err(
                    std::io::Error::other(format!("duplicate registry id {}", entry.id)).into(),
                );
            }
            if !names.insert(entry.name) {
                return Err(std::io::Error::other(format!(
                    "duplicate registry name {}",
                    entry.name
                ))
                .into());
            }
            for extension in entry.extensions {
                if let Some(existing) = extensions.insert(extension, entry.id) {
                    return Err(std::io::Error::other(format!(
                        "extension {extension} is owned by both {existing} and {}",
                        entry.id
                    ))
                    .into());
                }
            }
        }

        Ok(())
    }
}
