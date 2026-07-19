//! Environment/configuration fact extraction over existing analyzer outputs.
//!
//! Language analyzers remain responsible for parsing their own syntax. This
//! module only normalizes their already-typed facts into the shared resolver
//! contract and provides small line-oriented parsers for `.env` and property
//! files that have no tree-sitter semantic analyzer yet.

use crate::analysis::{
    ActionsProfile, ComposeProfile, ConfigReferenceKind, DockerfileAnalysis, PythonAnalysis,
    PythonReferenceKind, RustAnalysis, RustReferenceKind, StructuredAnalysis, TypeScriptAnalysis,
};
use crate::domain::{Artifact, ArtifactId, Confidence, EvidenceRef, SourceSpan};
use crate::resolve::{ConfigFact, EnvFact, FactRole, FactSourceKind};
use serde::{Deserialize, Serialize};

/// Normalized output from one or more environment/configuration analyzers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct EnvironmentFacts {
    /// Named environment-variable facts.
    pub env: Vec<EnvFact>,
    /// Named configuration-key facts.
    pub config: Vec<ConfigFact>,
    /// Dynamic expressions retained without inventing a variable name.
    pub unresolved: Vec<UnresolvedEnvironmentFact>,
}

impl EnvironmentFacts {
    /// Adds facts from another analyzer output.
    pub(crate) fn extend(&mut self, other: Self) {
        self.env.extend(other.env);
        self.config.extend(other.config);
        self.unresolved.extend(other.unresolved);
    }

    /// Sorts all fact collections into a stable order for cache/golden output.
    pub(crate) fn sort_deterministically(&mut self) {
        self.env.sort_by(|a, b| {
            (
                a.name.canonical.as_str(),
                a.role,
                a.source,
                evidence_key(&a.evidence),
            )
                .cmp(&(
                    b.name.canonical.as_str(),
                    b.role,
                    b.source,
                    evidence_key(&b.evidence),
                ))
        });
        self.config.sort_by(|a, b| {
            (
                a.key.canonical.as_str(),
                a.role,
                a.source,
                evidence_key(&a.evidence),
            )
                .cmp(&(
                    b.key.canonical.as_str(),
                    b.role,
                    b.source,
                    evidence_key(&b.evidence),
                ))
        });
        self.unresolved.sort_by(|a, b| {
            (
                a.expression.as_str(),
                a.role,
                a.source,
                evidence_key(&a.evidence),
            )
                .cmp(&(
                    b.expression.as_str(),
                    b.role,
                    b.source,
                    evidence_key(&b.evidence),
                ))
        });
    }

    /// Converts Python environment-read references into shared facts.
    pub(crate) fn from_python(analysis: &PythonAnalysis) -> Self {
        let mut facts = Self::default();
        for reference in &analysis.references {
            if reference.kind == PythonReferenceKind::EnvRead {
                push_env(
                    &mut facts,
                    &reference.value,
                    FactContext::new(
                        FactRole::Read,
                        FactSourceKind::SourceCode,
                        None,
                        reference.evidence.clone(),
                        reference.confidence,
                    ),
                );
            }
        }
        facts.sort_deterministically();
        facts
    }

    /// Converts Rust environment-read references into shared facts.
    pub(crate) fn from_rust(analysis: &RustAnalysis) -> Self {
        let mut facts = Self::default();
        for reference in &analysis.references {
            if reference.kind == RustReferenceKind::EnvRead {
                push_env(
                    &mut facts,
                    &reference.value,
                    FactContext::new(
                        FactRole::Read,
                        FactSourceKind::SourceCode,
                        None,
                        reference.evidence.clone(),
                        reference.confidence,
                    ),
                );
            }
        }
        facts.sort_deterministically();
        facts
    }

    /// Converts TypeScript/TSX environment reads into shared facts.
    pub(crate) fn from_typescript(analysis: &TypeScriptAnalysis) -> Self {
        let mut facts = Self::default();
        for reference in &analysis.env_reads {
            match &reference.name {
                Some(name) => push_env(
                    &mut facts,
                    name,
                    FactContext::new(
                        FactRole::Read,
                        FactSourceKind::SourceCode,
                        None,
                        reference.evidence.clone(),
                        reference.confidence,
                    ),
                ),
                None => facts.unresolved.push(UnresolvedEnvironmentFact {
                    expression: reference.expression.clone(),
                    role: FactRole::Read,
                    source: FactSourceKind::SourceCode,
                    owner: None,
                    evidence: reference.evidence.clone(),
                    confidence: reference.confidence,
                }),
            }
        }
        facts.sort_deterministically();
        facts
    }

    /// Converts Dockerfile `ENV` and `ARG` assignments into definitions.
    pub(crate) fn from_dockerfile(analysis: &DockerfileAnalysis) -> Self {
        let mut facts = Self::default();
        for assignment in analysis.env.iter().chain(&analysis.args) {
            push_env_value(
                &mut facts,
                &assignment.key,
                FactContext::new(
                    FactRole::Define,
                    FactSourceKind::Dockerfile,
                    None,
                    assignment.evidence.clone(),
                    Confidence::High,
                ),
                assignment.value.clone(),
            );
        }
        facts.sort_deterministically();
        facts
    }

    /// Converts Compose service environment blocks into definitions.
    pub(crate) fn from_compose(analysis: &ComposeProfile) -> Self {
        let mut facts = Self::default();
        for service in &analysis.services {
            for assignment in &service.environment {
                push_env_value(
                    &mut facts,
                    &assignment.key,
                    FactContext::new(
                        FactRole::Define,
                        FactSourceKind::Compose,
                        Some(service.name.clone()),
                        assignment.evidence.clone(),
                        Confidence::High,
                    ),
                    assignment.value.clone(),
                );
            }
        }
        facts.sort_deterministically();
        facts
    }

    /// Converts CI workflow step environments into definitions.
    pub(crate) fn from_actions(analysis: &ActionsProfile) -> Self {
        let mut facts = Self::default();
        for job in &analysis.jobs {
            for step in &job.steps {
                for assignment in &step.env {
                    push_env_value(
                        &mut facts,
                        &assignment.key,
                        FactContext::new(
                            FactRole::Define,
                            FactSourceKind::CiWorkflow,
                            Some(job.id.clone()),
                            assignment.evidence.clone(),
                            Confidence::High,
                        ),
                        assignment.value.clone(),
                    );
                }
            }
        }
        facts.sort_deterministically();
        facts
    }

    /// Converts structured config paths and environment references into facts.
    pub(crate) fn from_structured(analysis: &StructuredAnalysis) -> Self {
        let mut facts = Self::default();
        for entity in &analysis.entities {
            let Some(key) = config_path_key(&entity.config_path) else {
                continue;
            };
            push_config_value(
                &mut facts,
                key,
                FactContext::new(
                    FactRole::Define,
                    FactSourceKind::StructuredConfig,
                    None,
                    entity.evidence.clone(),
                    Confidence::High,
                ),
                entity.scalar_summary.clone(),
            );
        }
        for reference in &analysis.references {
            if reference.kind == ConfigReferenceKind::EnvironmentVariable {
                push_env(
                    &mut facts,
                    &reference.value,
                    FactContext::new(
                        FactRole::Reference,
                        FactSourceKind::StructuredConfig,
                        None,
                        reference.evidence.clone(),
                        Confidence::High,
                    ),
                );
            }
        }
        facts.sort_deterministically();
        facts
    }

    /// Parses `.env` assignments and Java/Spring-style property files.
    pub(crate) fn parse_assignments(artifact: &Artifact, text: &str) -> Self {
        let file_name = artifact
            .path
            .as_str()
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let is_dotenv = file_name == ".env" || file_name.starts_with(".env.");
        let source = if is_dotenv {
            FactSourceKind::Dotenv
        } else {
            FactSourceKind::Properties
        };
        let mut facts = Self::default();
        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            let Some((key, raw_value)) = line.split_once('=').or_else(|| line.split_once(':'))
            else {
                continue;
            };
            let key = key.trim();
            if key.is_empty() || key.chars().any(char::is_whitespace) {
                continue;
            }
            let value = unquote(raw_value.trim());
            let evidence = line_evidence(artifact, index as u32 + 1);
            if is_dotenv {
                push_env_value(
                    &mut facts,
                    key,
                    FactContext::new(
                        FactRole::Define,
                        source,
                        None,
                        evidence.clone(),
                        Confidence::High,
                    ),
                    Some(value.to_owned()),
                );
            } else {
                push_config_value(
                    &mut facts,
                    key,
                    FactContext::new(
                        FactRole::Define,
                        source,
                        None,
                        evidence.clone(),
                        Confidence::High,
                    ),
                    Some(value.to_owned()),
                );
            }
            for placeholder in placeholders(value) {
                push_env(
                    &mut facts,
                    placeholder,
                    FactContext::new(
                        FactRole::Reference,
                        source,
                        None,
                        evidence.clone(),
                        Confidence::High,
                    ),
                );
            }
        }
        facts.sort_deterministically();
        facts
    }
}

/// A dynamic environment expression kept for review without a fake name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct UnresolvedEnvironmentFact {
    /// Expression as written by the source.
    pub expression: String,
    /// Fact role.
    pub role: FactRole,
    /// Source family.
    pub source: FactSourceKind,
    /// Stable owner, when known.
    pub owner: Option<String>,
    /// Source evidence.
    pub evidence: EvidenceRef,
    /// Deterministic confidence bucket.
    pub confidence: Confidence,
}

#[derive(Debug, Clone)]
struct FactContext {
    role: FactRole,
    source: FactSourceKind,
    owner: Option<String>,
    evidence: EvidenceRef,
    confidence: Confidence,
}

impl FactContext {
    fn new(
        role: FactRole,
        source: FactSourceKind,
        owner: Option<String>,
        evidence: EvidenceRef,
        confidence: Confidence,
    ) -> Self {
        Self {
            role,
            source,
            owner,
            evidence,
            confidence,
        }
    }
}

fn push_env(facts: &mut EnvironmentFacts, name: &str, context: FactContext) {
    if context.confidence == Confidence::Low {
        facts.unresolved.push(UnresolvedEnvironmentFact {
            expression: name.to_owned(),
            role: context.role,
            source: context.source,
            owner: context.owner,
            evidence: context.evidence,
            confidence: context.confidence,
        });
        return;
    }
    match EnvFact::new(
        name,
        context.role,
        context.source,
        context.owner.clone(),
        None,
        context.evidence.clone(),
        context.confidence,
    ) {
        Ok(fact) => facts.env.push(fact),
        Err(_) => facts.unresolved.push(UnresolvedEnvironmentFact {
            expression: name.to_owned(),
            role: context.role,
            source: context.source,
            owner: context.owner,
            evidence: context.evidence,
            confidence: context.confidence,
        }),
    }
}

fn push_env_value(
    facts: &mut EnvironmentFacts,
    name: &str,
    context: FactContext,
    value: Option<String>,
) {
    match EnvFact::new(
        name,
        context.role,
        context.source,
        context.owner.clone(),
        value,
        context.evidence.clone(),
        context.confidence,
    ) {
        Ok(fact) => facts.env.push(fact),
        Err(_) => facts.unresolved.push(UnresolvedEnvironmentFact {
            expression: name.to_owned(),
            role: context.role,
            source: context.source,
            owner: context.owner,
            evidence: context.evidence,
            confidence: context.confidence,
        }),
    }
}

fn push_config_value(
    facts: &mut EnvironmentFacts,
    key: &str,
    context: FactContext,
    value: Option<String>,
) {
    if let Ok(fact) = ConfigFact::new(
        key,
        context.role,
        context.source,
        context.owner,
        value,
        context.evidence,
        context.confidence,
    ) {
        facts.config.push(fact);
    }
}

fn config_path_key(path: &str) -> Option<&str> {
    let key = path.strip_prefix("$.").unwrap_or(path);
    (!key.is_empty() && key != "$").then_some(key)
}

fn evidence_key(evidence: &EvidenceRef) -> (&str, u32, u32) {
    let span = evidence.span.as_ref();
    (
        evidence.path.as_str(),
        span.map_or(0, |value| value.start_line),
        span.map_or(0, |value| value.end_line),
    )
}

fn line_evidence(artifact: &Artifact, line: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    SourceSpan::new(line, line).map_or(base.clone(), |span| base.with_span(span))
}

fn unquote(value: &str) -> &str {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .or_else(|| {
            value
                .strip_prefix('\'')
                .and_then(|value| value.strip_suffix('\''))
        })
        .unwrap_or(value)
}

fn placeholders(value: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            break;
        };
        let name = after[..end]
            .split_once(':')
            .map_or(after[..end].trim(), |(name, _)| name.trim());
        if !name.is_empty() {
            found.push(name);
        }
        rest = &after[end + 1..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::PythonReferenceKind;
    use crate::analysis::{PythonAnalysis, PythonReference};
    use crate::domain::{
        ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier, TextStatus,
    };

    fn artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::Configuration,
            SupportTier::GenericText,
            ContentHash::new("00")?,
            0,
        )
        .with_text_status(TextStatus::Text, Some(1))
        .with_model_policy(ModelExposurePolicy::Allowed))
    }

    #[test]
    fn parses_dotenv_and_properties_without_persisting_secrets()
    -> Result<(), Box<dyn std::error::Error>> {
        let dotenv = artifact(".env")?;
        let facts = EnvironmentFacts::parse_assignments(
            &dotenv,
            "API_URL=https://example.test\nDB_PASSWORD=hidden\nCACHE_URL=${API_URL}\n",
        );
        assert_eq!(facts.env.len(), 4);
        assert!(
            facts
                .env
                .iter()
                .find(|fact| fact.name.original() == "DB_PASSWORD")
                .and_then(|fact| fact.value.as_ref())
                .is_some_and(|value| value.redacted)
        );
        assert!(
            facts
                .env
                .iter()
                .any(|fact| fact.role == FactRole::Reference && fact.name.original() == "API_URL")
        );

        let properties = artifact("application.properties")?;
        let facts = EnvironmentFacts::parse_assignments(
            &properties,
            "server.port=8080\nspring.datasource.url=${DB_URL:jdbc:test}\n",
        );
        assert_eq!(facts.config.len(), 2);
        assert!(
            facts
                .env
                .iter()
                .any(|fact| fact.name.original() == "DB_URL")
        );
        Ok(())
    }

    #[test]
    fn dynamic_python_environment_reads_are_retained_as_unresolved()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = RepoPath::new("src/main.py")?;
        let analysis = PythonAnalysis {
            references: vec![PythonReference {
                kind: PythonReferenceKind::EnvRead,
                value: "config_key".to_owned(),
                confidence: Confidence::Low,
                evidence: EvidenceRef::file(ArtifactId::from_path(&path), path),
            }],
            ..PythonAnalysis::default()
        };
        let facts = EnvironmentFacts::from_python(&analysis);
        assert!(facts.env.is_empty());
        assert_eq!(facts.unresolved.len(), 1);
        Ok(())
    }
}
