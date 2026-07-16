//! Repository analysis primitives.

pub mod cache;
pub mod dockerfile;
pub mod environment;
pub mod external_symbols;
pub mod generic_text;
pub mod ir;
pub mod markdown;
pub mod packages;
pub mod profiles;
pub mod protocols;
pub mod python;
pub mod rationale;
pub mod rust_metadata;
pub mod rust_source;
pub mod structured;
pub mod tree_sitter_adapter;
pub mod tsconfig;
pub mod typescript;

pub use cache::{ANALYSIS_CACHE_VERSION, AnalysisCache, AnalyzerKind, AnalyzerOutput};
pub use dockerfile::{
    DockerCommand, DockerCommandKind, DockerCopy, DockerEnv, DockerInstruction,
    DockerInstructionKind, DockerPort, DockerSingleValue, DockerStage, DockerfileAnalysis,
    DockerfileAnalyzer,
};
pub use environment::{EnvironmentFacts, UnresolvedEnvironmentFact};
pub use external_symbols::{
    is_python_stdlib_module, is_rust_prelude_type, normalize_python_package_name, rust_std_crate,
};
pub use generic_text::{FindingConfidence, GenericTextExtractor, TextFinding, TextFindingKind};
pub use ir::{
    ExtractionIr, IrDeclaration, IrDetail, IrEdgeCandidate, IrEdgeKind, IrFile, IrLanguage,
    IrModule, IrNodeKind, IrTarget,
};
pub use markdown::{
    CodeFence, DriftKind, LinkKind, MarkdownAnalysis, MarkdownAnalyzer, MarkdownCommand,
    MarkdownDrift, MarkdownHeading, MarkdownLink, MarkdownPathReference,
};
pub use packages::{
    ComposerAnalyzer, CsprojAnalyzer, GoModAnalyzer, GradleAnalyzer, MavenPomAnalyzer,
    NpmPackageAnalyzer, PackageDependency, PackageManifestAnalysis, PackageManifestFormat,
};
pub use profiles::{
    ActionsJob, ActionsProfile, ActionsProfileAnalyzer, ActionsStep, ActionsStepHint,
    CargoDependency, CargoDependencyKind, CargoFeature, CargoPackage, CargoProfile,
    CargoProfileAnalyzer, CargoTarget, CargoTargetKind, CargoWorkspaceMember, ComposePort,
    ComposeProfile, ComposeProfileAnalyzer, ComposeService, EnvVarFact, PyProjectAnalyzer,
    PyProjectProfile, PythonDependency, PythonProject, PythonRequirement, RequirementsAnalyzer,
    RequirementsProfile,
};
pub use protocols::{GraphQlAnalyzer, ProtoAnalyzer, ProtocolFormat, ProtocolRoute};
pub use python::{
    PythonAnalysis, PythonAnalyzer, PythonBinding, PythonClass, PythonFunction, PythonImport,
    PythonImportKind, PythonImportName, PythonMemberCall, PythonReference, PythonReferenceKind,
};
pub use rationale::{
    Rationale, RationaleKind, classify as classify_rationale, is_generated_source,
};
pub use rust_metadata::{
    RustDependency, RustDependencyKind, RustFeature, RustPackage, RustTarget,
    RustWorkspaceAnalysis, RustWorkspaceAnalyzer,
};
pub use rust_source::{
    RustAnalysis, RustAnalyzer, RustFunction, RustImpl, RustItem, RustMacroInvocation,
    RustModDeclaration, RustReference, RustReferenceKind, RustTrait, RustUse,
};
pub use structured::{
    ConfigEntity, ConfigReference, ConfigReferenceKind, StructuredAnalysis, StructuredAnalyzer,
    StructuredFormat,
};
pub use tree_sitter_adapter::{
    SyntaxIndexedLanguage, TreeSitterAdapterOutput, TreeSitterComment, TreeSitterParseStatus,
    TreeSitterParserAdapter, TreeSitterSyntaxError, TreeSitterSyntaxFact,
    parse_with_optional_adapter,
};
pub use tsconfig::{TsConfigProfile, parse_tsconfig};
pub use typescript::{
    TypeScriptAnalysis, TypeScriptAnalyzer, TypeScriptBinding, TypeScriptCall, TypeScriptClass,
    TypeScriptEnvRead, TypeScriptFunction, TypeScriptLanguage, TypeScriptMemberCall,
    TypeScriptReExport, TypeScriptReExportKind,
};
