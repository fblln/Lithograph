//! Repository analysis primitives.

pub(crate) mod cache;
pub(crate) mod dockerfile;
pub(crate) mod environment;
pub(crate) mod external_symbols;
pub(crate) mod generic_text;
pub(crate) mod markdown;
pub(crate) mod packages;
pub(crate) mod profiles;
pub(crate) mod protocols;
pub(crate) mod python;
pub(crate) mod rationale;
pub(crate) mod rust_metadata;
pub(crate) mod rust_source;
pub(crate) mod structured;
pub(crate) mod tree_sitter_adapter;
pub(crate) mod tsconfig;
pub(crate) mod typescript;

pub(crate) use cache::{ANALYSIS_CACHE_VERSION, AnalysisCache, AnalyzerKind, AnalyzerOutput};
pub(crate) use dockerfile::{DockerCommandKind, DockerfileAnalysis, DockerfileAnalyzer};
pub(crate) use environment::EnvironmentFacts;
pub(crate) use external_symbols::{
    is_javascript_builtin, is_python_builtin, is_python_stdlib_module,
    normalize_python_package_name, rust_std_crate,
};
#[cfg(test)]
pub(crate) use generic_text::FindingConfidence;
pub(crate) use generic_text::{GenericTextExtractor, TextFinding, TextFindingKind};
pub(crate) use markdown::{DriftKind, LinkKind, MarkdownAnalysis, MarkdownAnalyzer, MarkdownDrift};
pub(crate) use packages::{PackageManifestAnalysis, PackageManifestFormat};
pub(crate) use profiles::{
    ActionsProfile, ActionsProfileAnalyzer, ActionsStepHint, CargoProfile, CargoProfileAnalyzer,
    ComposeProfile, ComposeProfileAnalyzer, PyProjectAnalyzer, PyProjectProfile,
    RequirementsAnalyzer, RequirementsProfile,
};
pub(crate) use protocols::{ProtocolFormat, ProtocolRoute};
pub(crate) use python::{
    PythonAnalysis, PythonAnalyzer, PythonFunction, PythonImport, PythonImportKind,
    PythonReference, PythonReferenceKind,
};
pub(crate) use rationale::{RationaleKind, classify as classify_rationale, is_generated_source};
pub(crate) use rust_metadata::{RustWorkspaceAnalysis, RustWorkspaceAnalyzer};
pub(crate) use rust_source::{RustAnalysis, RustAnalyzer, RustReference, RustReferenceKind};
pub(crate) use structured::{
    ConfigReferenceKind, StructuredAnalysis, StructuredAnalyzer, StructuredFormat,
};
pub(crate) use tree_sitter_adapter::{
    SyntaxIndexedLanguage, TreeSitterAdapterOutput, TreeSitterComment,
};
pub(crate) use tsconfig::{TsConfigProfile, parse_tsconfig};
pub(crate) use typescript::{
    TypeScriptAnalysis, TypeScriptAnalyzer, TypeScriptLanguage, TypeScriptReExportKind,
};
