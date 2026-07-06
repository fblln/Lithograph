//! Repository analysis primitives.

pub mod dockerfile;
pub mod generic_text;
pub mod markdown;
pub mod profiles;
pub mod python;
pub mod rust_metadata;
pub mod rust_source;
pub mod structured;

pub use dockerfile::{
    DockerCommand, DockerCommandKind, DockerCopy, DockerEnv, DockerInstruction,
    DockerInstructionKind, DockerPort, DockerStage, DockerfileAnalysis, DockerfileAnalyzer,
};
pub use generic_text::{FindingConfidence, GenericTextExtractor, TextFinding, TextFindingKind};
pub use markdown::{
    CodeFence, DriftKind, LinkKind, MarkdownAnalysis, MarkdownAnalyzer, MarkdownCommand,
    MarkdownDrift, MarkdownHeading, MarkdownLink, MarkdownPathReference,
};
pub use profiles::{
    ActionsJob, ActionsProfile, ActionsProfileAnalyzer, ActionsStep, ActionsStepHint,
    CargoDependency, CargoDependencyKind, CargoFeature, CargoPackage, CargoProfile,
    CargoProfileAnalyzer, CargoTarget, CargoTargetKind, CargoWorkspaceMember, ComposePort,
    ComposeProfile, ComposeProfileAnalyzer, ComposeService, EnvVarFact, PyProjectAnalyzer,
    PyProjectProfile, PythonDependency, PythonProject, PythonRequirement, RequirementsAnalyzer,
    RequirementsProfile,
};
pub use python::{
    PythonAnalysis, PythonAnalyzer, PythonClass, PythonFunction, PythonImport, PythonImportKind,
    PythonImportName, PythonReference, PythonReferenceKind,
};
pub use rust_metadata::{
    RustDependency, RustDependencyKind, RustFeature, RustPackage, RustTarget,
    RustWorkspaceAnalysis, RustWorkspaceAnalyzer,
};
pub use rust_source::{
    RustAnalysis, RustAnalyzer, RustFunction, RustImpl, RustItem, RustMacroInvocation,
    RustModDeclaration, RustTrait, RustUse,
};
pub use structured::{
    ConfigEntity, ConfigReference, ConfigReferenceKind, StructuredAnalysis, StructuredAnalyzer,
    StructuredFormat,
};
