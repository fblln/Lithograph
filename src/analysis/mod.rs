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
pub(crate) use dockerfile::{
    DockerCommand, DockerCommandKind, DockerCopy, DockerEnv, DockerInstruction,
    DockerInstructionKind, DockerPort, DockerSingleValue, DockerStage, DockerfileAnalysis,
    DockerfileAnalyzer,
};
pub(crate) use environment::{EnvironmentFacts, UnresolvedEnvironmentFact};
pub(crate) use external_symbols::{
    is_javascript_builtin, is_python_builtin, is_python_stdlib_module,
    normalize_python_package_name, rust_std_crate,
};
pub(crate) use generic_text::{
    FindingConfidence, GenericTextExtractor, TextFinding, TextFindingKind,
};
pub(crate) use markdown::{
    CodeFence, DriftKind, LinkKind, MarkdownAnalysis, MarkdownAnalyzer, MarkdownCommand,
    MarkdownDrift, MarkdownHeading, MarkdownLink, MarkdownPathReference,
};
pub(crate) use packages::{
    ComposerAnalyzer, CsprojAnalyzer, GoModAnalyzer, GradleAnalyzer, MavenPomAnalyzer,
    NpmPackageAnalyzer, PackageDependency, PackageManifestAnalysis, PackageManifestFormat,
};
pub(crate) use profiles::{
    ActionsJob, ActionsProfile, ActionsProfileAnalyzer, ActionsStep, ActionsStepHint,
    CargoDependency, CargoDependencyKind, CargoFeature, CargoPackage, CargoProfile,
    CargoProfileAnalyzer, CargoTarget, CargoTargetKind, CargoWorkspaceMember, ComposePort,
    ComposeProfile, ComposeProfileAnalyzer, ComposeService, EnvVarFact, PyProjectAnalyzer,
    PyProjectProfile, PythonDependency, PythonProject, PythonRequirement, RequirementsAnalyzer,
    RequirementsProfile,
};
pub(crate) use protocols::{GraphQlAnalyzer, ProtoAnalyzer, ProtocolFormat, ProtocolRoute};
pub(crate) use python::{
    PythonAnalysis, PythonAnalyzer, PythonBinding, PythonClass, PythonFunction, PythonImport,
    PythonImportKind, PythonImportName, PythonMemberCall, PythonReference, PythonReferenceKind,
};
pub(crate) use rationale::{
    Rationale, RationaleKind, classify as classify_rationale, is_generated_source,
};
pub(crate) use rust_metadata::{
    RustDependency, RustDependencyKind, RustFeature, RustPackage, RustTarget,
    RustWorkspaceAnalysis, RustWorkspaceAnalyzer,
};
pub(crate) use rust_source::{
    RustAnalysis, RustAnalyzer, RustFunction, RustImpl, RustItem, RustMacroInvocation,
    RustModDeclaration, RustReference, RustReferenceKind, RustTrait, RustUse,
};
pub(crate) use structured::{
    ConfigEntity, ConfigReference, ConfigReferenceKind, StructuredAnalysis, StructuredAnalyzer,
    StructuredFormat,
};
pub(crate) use tree_sitter_adapter::{
    SyntaxIndexedLanguage, TreeSitterAdapterOutput, TreeSitterComment, TreeSitterParseStatus,
    TreeSitterParserAdapter, TreeSitterSyntaxError, TreeSitterSyntaxFact,
    parse_with_optional_adapter,
};
pub(crate) use tsconfig::{TsConfigProfile, parse_tsconfig};
pub(crate) use typescript::{
    TypeScriptAnalysis, TypeScriptAnalyzer, TypeScriptBinding, TypeScriptCall, TypeScriptClass,
    TypeScriptEnvRead, TypeScriptFunction, TypeScriptLanguage, TypeScriptMemberCall,
    TypeScriptReExport, TypeScriptReExportKind,
};
