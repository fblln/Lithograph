//! Language-neutral intermediate representation for extraction output.

#![allow(missing_docs)] // Field names mirror the documented source analyzer contracts.

use crate::analysis::{PythonAnalysis, RustAnalysis};
use crate::domain::{Artifact, Confidence, EvidenceRef};
use serde::{Deserialize, Serialize};

/// All extracted facts for one source artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtractionIr {
    /// Artifact identity and source language.
    pub file: IrFile,
    /// Module facts declared by the file.
    pub modules: Vec<IrModule>,
    /// Typed declarations.
    pub declarations: Vec<IrDeclaration>,
    /// Candidate edges requiring graph construction/resolution.
    pub edges: Vec<IrEdgeCandidate>,
    /// Parse recovery status for callers that must distinguish no fact from
    /// partial fact extraction.
    pub has_syntax_errors: bool,
}

/// Source file identity in the normalized IR.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrFile {
    /// Repository-relative artifact id.
    pub artifact_id: String,
    /// Repository-relative path.
    pub path: String,
    /// Normalized language id.
    pub language: IrLanguage,
}

/// Supported normalized source-language identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrLanguage {
    Python,
    Rust,
}

/// One module declared by a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrModule {
    pub path: String,
    pub is_root: bool,
    pub evidence: EvidenceRef,
}

/// Strongly typed declaration kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrNodeKind {
    File,
    Module,
    Class,
    Method,
    Function,
    Type,
    Struct,
    Enum,
    Trait,
    Route,
    Config,
    Test,
    Implementation,
    MacroInvocation,
}

/// Typed ancillary fact retained when a source-language analyzer has no
/// language-neutral primary field for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrDetail {
    PythonClassBase {
        value: String,
    },
    PythonImport {
        kind: crate::analysis::PythonImportKind,
        alias: Option<String>,
        relative_level: u32,
    },
    IsAsync,
    RustModule {
        is_inline: bool,
    },
    RustUse {
        alias: Option<String>,
    },
}

/// A declaration preserved from a language analyzer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrDeclaration {
    pub kind: IrNodeKind,
    pub name: String,
    pub parent: Option<String>,
    pub parameters: Vec<String>,
    pub return_type: Option<String>,
    pub attributes: Vec<String>,
    pub documentation: Option<String>,
    pub details: Vec<IrDetail>,
    pub evidence: EvidenceRef,
}

/// Strongly typed candidate-edge categories, independent of graph storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrEdgeKind {
    Imports,
    Calls,
    TypeRefs,
    Inherits,
    Implements,
    ReadsEnv,
    RunsCommand,
    References,
    Emits,
    ListensOn,
    DataFlows,
}

/// An unresolved target is explicit rather than represented by an empty id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IrTarget {
    ResolvedLocal { name: String },
    Unresolved { value: String },
}

/// One edge candidate emitted by extraction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IrEdgeCandidate {
    pub kind: IrEdgeKind,
    pub source: String,
    pub target: IrTarget,
    pub confidence: Confidence,
    pub details: Vec<IrDetail>,
    pub evidence: EvidenceRef,
}

impl ExtractionIr {
    /// Maps all currently available Python analyzer facts without discarding
    /// imports, declarations, references, decorators, or evidence.
    pub fn from_python(artifact: &Artifact, analysis: &PythonAnalysis) -> Self {
        let mut declarations = Vec::new();
        for class in &analysis.classes {
            declarations.push(IrDeclaration {
                kind: IrNodeKind::Class,
                name: class.name.clone(),
                parent: None,
                parameters: Vec::new(),
                return_type: None,
                attributes: class.decorators.clone(),
                documentation: class.docstring.clone(),
                details: class
                    .bases
                    .iter()
                    .cloned()
                    .map(|value| IrDetail::PythonClassBase { value })
                    .collect(),
                evidence: class.evidence.clone(),
            });
            for method in &class.methods {
                declarations.push(function_declaration(
                    method,
                    IrNodeKind::Method,
                    Some(class.name.clone()),
                ));
            }
        }
        for function in &analysis.functions {
            declarations.push(function_declaration(function, IrNodeKind::Function, None));
        }
        let mut edges = Vec::new();
        for import in &analysis.imports {
            for name in &import.names {
                edges.push(IrEdgeCandidate {
                    kind: IrEdgeKind::Imports,
                    source: analysis.module_path.clone(),
                    target: IrTarget::Unresolved {
                        value: import.module.as_ref().map_or_else(
                            || name.name.clone(),
                            |module| format!("{module}.{}", name.name),
                        ),
                    },
                    confidence: Confidence::Low,
                    details: vec![IrDetail::PythonImport {
                        kind: import.kind,
                        alias: name.alias.clone(),
                        relative_level: import.relative_level,
                    }],
                    evidence: import.evidence.clone(),
                });
            }
        }
        for reference in &analysis.references {
            edges.push(IrEdgeCandidate {
                kind: python_edge_kind(reference.kind),
                source: analysis.module_path.clone(),
                target: IrTarget::Unresolved {
                    value: reference.value.clone(),
                },
                confidence: reference.confidence,
                details: Vec::new(),
                evidence: reference.evidence.clone(),
            });
        }
        Self {
            file: file(artifact, IrLanguage::Python),
            modules: vec![IrModule {
                path: analysis.module_path.clone(),
                is_root: analysis.is_package_init,
                evidence: file_evidence(artifact),
            }],
            declarations,
            edges,
            has_syntax_errors: analysis.has_syntax_errors,
        }
    }

    /// Maps all currently available Rust analyzer facts without discarding
    /// declarations, use paths, impl metadata, reference heuristics, or evidence.
    pub fn from_rust(artifact: &Artifact, analysis: &RustAnalysis) -> Self {
        let mut declarations = Vec::new();
        for item in &analysis.structs {
            declarations.push(item_declaration(item, IrNodeKind::Struct));
        }
        for item in &analysis.enums {
            declarations.push(item_declaration(item, IrNodeKind::Enum));
        }
        for item in &analysis.traits {
            declarations.push(IrDeclaration {
                kind: IrNodeKind::Trait,
                name: item.name.clone(),
                parent: None,
                parameters: Vec::new(),
                return_type: None,
                attributes: item.attributes.clone(),
                documentation: item.doc.clone(),
                details: Vec::new(),
                evidence: item.evidence.clone(),
            });
            for method in &item.methods {
                declarations.push(IrDeclaration {
                    kind: IrNodeKind::Method,
                    name: method.clone(),
                    parent: Some(item.name.clone()),
                    parameters: Vec::new(),
                    return_type: None,
                    attributes: Vec::new(),
                    documentation: None,
                    details: Vec::new(),
                    evidence: item.evidence.clone(),
                });
            }
        }
        for function in &analysis.functions {
            declarations.push(IrDeclaration {
                kind: IrNodeKind::Function,
                name: function.name.clone(),
                parent: None,
                parameters: function.parameters.clone(),
                return_type: function.return_type.clone(),
                attributes: function.attributes.clone(),
                documentation: function.doc.clone(),
                details: Vec::new(),
                evidence: function.evidence.clone(),
            });
        }
        for module in &analysis.mod_declarations {
            declarations.push(IrDeclaration {
                kind: IrNodeKind::Module,
                name: module.name.clone(),
                parent: Some(analysis.module_path.clone()),
                parameters: Vec::new(),
                return_type: None,
                attributes: Vec::new(),
                documentation: None,
                details: vec![IrDetail::RustModule {
                    is_inline: module.is_inline,
                }],
                evidence: module.evidence.clone(),
            });
        }
        for implementation in &analysis.impls {
            let implementation_name = implementation_name(implementation);
            declarations.push(IrDeclaration {
                kind: IrNodeKind::Implementation,
                name: implementation_name.clone(),
                parent: implementation.trait_name.clone(),
                parameters: Vec::new(),
                return_type: None,
                attributes: Vec::new(),
                documentation: None,
                details: Vec::new(),
                evidence: implementation.evidence.clone(),
            });
            for method in &implementation.methods {
                declarations.push(IrDeclaration {
                    kind: IrNodeKind::Method,
                    name: method.clone(),
                    parent: Some(implementation_name.clone()),
                    parameters: Vec::new(),
                    return_type: None,
                    attributes: Vec::new(),
                    documentation: None,
                    details: Vec::new(),
                    evidence: implementation.evidence.clone(),
                });
            }
        }
        for invocation in &analysis.macro_invocations {
            declarations.push(IrDeclaration {
                kind: IrNodeKind::MacroInvocation,
                name: invocation.name.clone(),
                parent: Some(analysis.module_path.clone()),
                parameters: Vec::new(),
                return_type: None,
                attributes: Vec::new(),
                documentation: None,
                details: Vec::new(),
                evidence: invocation.evidence.clone(),
            });
        }
        let mut edges = Vec::new();
        for usage in &analysis.uses {
            edges.push(IrEdgeCandidate {
                kind: IrEdgeKind::Imports,
                source: analysis.module_path.clone(),
                target: IrTarget::Unresolved {
                    value: usage.path.clone(),
                },
                confidence: Confidence::Low,
                details: vec![IrDetail::RustUse {
                    alias: usage.alias.clone(),
                }],
                evidence: usage.evidence.clone(),
            });
        }
        for reference in &analysis.references {
            edges.push(IrEdgeCandidate {
                kind: match reference.kind {
                    crate::analysis::RustReferenceKind::EnvRead => IrEdgeKind::ReadsEnv,
                    crate::analysis::RustReferenceKind::Subprocess => IrEdgeKind::RunsCommand,
                    crate::analysis::RustReferenceKind::Ffi => IrEdgeKind::References,
                    crate::analysis::RustReferenceKind::Call => IrEdgeKind::Calls,
                },
                source: analysis.module_path.clone(),
                target: IrTarget::Unresolved {
                    value: reference.value.clone(),
                },
                confidence: reference.confidence,
                details: Vec::new(),
                evidence: reference.evidence.clone(),
            });
        }
        Self {
            file: file(artifact, IrLanguage::Rust),
            modules: vec![IrModule {
                path: analysis.module_path.clone(),
                is_root: analysis.is_crate_root,
                evidence: file_evidence(artifact),
            }],
            declarations,
            edges,
            has_syntax_errors: analysis.has_syntax_errors,
        }
    }
}

fn file(artifact: &Artifact, language: IrLanguage) -> IrFile {
    IrFile {
        artifact_id: artifact.id.as_str().to_owned(),
        path: artifact.path.as_str().to_owned(),
        language,
    }
}
fn file_evidence(artifact: &Artifact) -> EvidenceRef {
    EvidenceRef::file(artifact.id.clone(), artifact.path.clone())
}
fn function_declaration(
    function: &crate::analysis::PythonFunction,
    kind: IrNodeKind,
    parent: Option<String>,
) -> IrDeclaration {
    IrDeclaration {
        kind,
        name: function.name.clone(),
        parent,
        parameters: function.parameters.clone(),
        return_type: function.return_type.clone(),
        attributes: function.decorators.clone(),
        documentation: function.docstring.clone(),
        details: function
            .is_async
            .then_some(IrDetail::IsAsync)
            .into_iter()
            .collect(),
        evidence: function.evidence.clone(),
    }
}
fn item_declaration(item: &crate::analysis::RustItem, kind: IrNodeKind) -> IrDeclaration {
    IrDeclaration {
        kind,
        name: item.name.clone(),
        parent: None,
        parameters: Vec::new(),
        return_type: None,
        attributes: item.attributes.clone(),
        documentation: item.doc.clone(),
        details: Vec::new(),
        evidence: item.evidence.clone(),
    }
}

fn implementation_name(implementation: &crate::analysis::RustImpl) -> String {
    match &implementation.trait_name {
        Some(trait_name) => format!("{trait_name} for {}", implementation.target_type),
        None => implementation.target_type.clone(),
    }
}
fn python_edge_kind(kind: crate::analysis::PythonReferenceKind) -> IrEdgeKind {
    use crate::analysis::PythonReferenceKind::*;
    match kind {
        Call => IrEdgeKind::Calls,
        EnvRead => IrEdgeKind::ReadsEnv,
        Subprocess => IrEdgeKind::RunsCommand,
        DynamicImport | Ctypes | ConfigPath | HttpCall => IrEdgeKind::References,
        Emits => IrEdgeKind::Emits,
        ListensOn => IrEdgeKind::ListensOn,
        DataFlows => IrEdgeKind::DataFlows,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::{PythonAnalyzer, RustAnalyzer};
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;

    fn fixture_artifact(path: &str) -> Result<(Artifact, String), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifact = RepositoryWalker::new(WalkOptions::default())
            .walk(&root)?
            .into_iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, path))?;
        Ok((artifact, fs::read_to_string(root.join(path))?))
    }

    #[test]
    fn python_fixture_maps_every_analyzer_fact_deterministically()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("src/python_app/service.py")?;
        let analysis = PythonAnalyzer.analyze(&artifact, &text);
        let ir = ExtractionIr::from_python(&artifact, &analysis);

        let expected_declarations = analysis.classes.len()
            + analysis.functions.len()
            + analysis
                .classes
                .iter()
                .map(|class| class.methods.len())
                .sum::<usize>();
        let expected_edges = analysis.references.len()
            + analysis
                .imports
                .iter()
                .map(|import| import.names.len())
                .sum::<usize>();
        assert_eq!(ir.declarations.len(), expected_declarations);
        assert_eq!(ir.edges.len(), expected_edges);
        assert_eq!(ir.file.language, IrLanguage::Python);
        assert_eq!(ir.modules[0].path, analysis.module_path);
        assert!(ir.edges.iter().any(|edge| {
            matches!(
                edge.details.as_slice(),
                [IrDetail::PythonImport {
                    relative_level: 0,
                    ..
                }]
            )
        }));
        assert!(
            ir.edges
                .iter()
                .any(|edge| edge.kind == IrEdgeKind::ReadsEnv)
        );
        assert_eq!(serde_json::to_string(&ir)?, serde_json::to_string(&ir)?);
        Ok(())
    }

    #[test]
    fn rust_fixture_maps_every_analyzer_fact_deterministically()
    -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("rust/src/lib.rs")?;
        let analysis = RustAnalyzer.analyze(&artifact, &text);
        let ir = ExtractionIr::from_rust(&artifact, &analysis);

        let expected_declarations = analysis.structs.len()
            + analysis.enums.len()
            + analysis.traits.len()
            + analysis.functions.len()
            + analysis.mod_declarations.len()
            + analysis.impls.len()
            + analysis.macro_invocations.len()
            + analysis
                .traits
                .iter()
                .map(|item| item.methods.len())
                .sum::<usize>()
            + analysis
                .impls
                .iter()
                .map(|item| item.methods.len())
                .sum::<usize>();
        assert_eq!(ir.declarations.len(), expected_declarations);
        assert_eq!(
            ir.edges.len(),
            analysis.uses.len() + analysis.references.len()
        );
        assert_eq!(ir.file.language, IrLanguage::Rust);
        assert!(ir.declarations.iter().any(|item| {
            item.kind == IrNodeKind::Implementation && item.name == "RouteBake for RouteBaker"
        }));
        assert!(
            ir.declarations
                .iter()
                .any(|item| item.kind == IrNodeKind::MacroInvocation && item.name == "format")
        );
        assert!(ir.edges.iter().any(|edge| {
            matches!(edge.details.as_slice(), [IrDetail::RustUse { alias: None }])
        }));
        assert_eq!(serde_json::to_string(&ir)?, serde_json::to_string(&ir)?);
        Ok(())
    }
}
