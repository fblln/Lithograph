//! Shared environment/configuration facts and deterministic name handling.
//!
//! This module deliberately stops before graph resolution. It gives language
//! and configuration analyzers one lossless, secret-safe contract that later
//! resolver passes can index without re-parsing source text.

use crate::domain::{Confidence, EvidenceRef};
use crate::graph::{
    ConfigNodeKind, Graph, GraphNode, GraphNodeId, Relation, RelationKind, RelationProvenance,
    RelationResolution,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Display, Formatter};

/// Version of the normalized environment/configuration fact contract.
pub(crate) const ENVIRONMENT_FACT_VERSION: u32 = 1;

/// Error returned when an identifier contains no usable alphanumeric token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct NameNormalizationError;

impl Display for NameNormalizationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("identifier must contain at least one alphanumeric token")
    }
}

impl std::error::Error for NameNormalizationError {}

/// A source identifier with its original spelling and canonical tokens.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct NormalizedName {
    /// Original spelling retained for evidence and display.
    pub original: String,
    /// Lowercase tokens split at separators, case transitions, acronym edges,
    /// and alpha/numeric boundaries.
    pub tokens: Vec<String>,
    /// Stable dot-separated canonical form.
    pub canonical: String,
}

impl NormalizedName {
    /// Normalizes an identifier without discarding its original spelling.
    pub(crate) fn new(value: impl Into<String>) -> Result<Self, NameNormalizationError> {
        let original = value.into();
        let tokens = split_identifier(&original);
        if tokens.is_empty() {
            return Err(NameNormalizationError);
        }
        let canonical = tokens.join(".");
        Ok(Self {
            original,
            tokens,
            canonical,
        })
    }

    /// Original spelling as written by the source or configuration file.
    pub(crate) fn original(&self) -> &str {
        &self.original
    }

    /// Generates deterministic spelling aliases used by exact/framework
    /// resolution. The original spelling is always retained as an alias.
    pub(crate) fn aliases(&self) -> Vec<NameAlias> {
        let mut aliases = BTreeMap::<String, NameAliasKind>::new();
        add_alias(&mut aliases, &self.original, NameAliasKind::Original);
        add_alias(&mut aliases, &self.canonical, NameAliasKind::CanonicalDot);
        add_alias(&mut aliases, &self.tokens.join("-"), NameAliasKind::Kebab);
        add_alias(
            &mut aliases,
            &camel_case(&self.tokens, false),
            NameAliasKind::Camel,
        );
        add_alias(
            &mut aliases,
            &camel_case(&self.tokens, true),
            NameAliasKind::Pascal,
        );
        add_alias(
            &mut aliases,
            &self
                .tokens
                .iter()
                .map(|token| token.to_ascii_uppercase())
                .collect::<Vec<_>>()
                .join("_"),
            NameAliasKind::EnvironmentUpper,
        );
        aliases
            .into_iter()
            .map(|(value, kind)| NameAlias { value, kind })
            .collect()
    }
}

/// How a normalized alias was generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum NameAliasKind {
    /// The exact original spelling.
    Original,
    /// Lowercase dot-separated canonical form.
    CanonicalDot,
    /// Lowercase kebab-case form.
    Kebab,
    /// Lower camelCase form.
    Camel,
    /// Upper camel/PascalCase form.
    Pascal,
    /// Upper snake-case environment form.
    EnvironmentUpper,
}

/// One deterministic spelling alias and its provenance.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct NameAlias {
    /// Alias text.
    pub value: String,
    /// Alias generation strategy.
    pub kind: NameAliasKind,
}

/// Role played by an environment/configuration fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum FactRole {
    /// A source symbol or artifact reads an environment variable.
    Read,
    /// A deployment/configuration source defines or supplies an environment variable.
    Define,
    /// A source references a configuration key.
    Reference,
}

/// Source family responsible for a fact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum FactSourceKind {
    /// Python, Rust, TypeScript, or another source-language analyzer.
    SourceCode,
    /// `.env` assignment.
    Dotenv,
    /// Java/Spring or generic `.properties` assignment/reference.
    Properties,
    /// YAML, JSON, or TOML structured configuration.
    StructuredConfig,
    /// Dockerfile `ENV` or `ARG`.
    Dockerfile,
    /// Docker Compose environment block.
    Compose,
    /// GitHub Actions or another CI workflow environment block.
    CiWorkflow,
    /// Kubernetes resource configuration.
    Kubernetes,
    /// Helm values or templates.
    Helm,
    /// Framework-specific convention such as Spring relaxed binding.
    Framework,
}

/// A literal value that is safe to retain in analysis output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SafeFactValue {
    /// Non-secret value, omitted when redaction is required.
    pub value: Option<String>,
    /// True when a value was intentionally withheld.
    pub redacted: bool,
}

impl SafeFactValue {
    /// Retains non-sensitive values and redacts likely secrets by key/value.
    pub(crate) fn from_named_value(name: &NormalizedName, value: impl Into<String>) -> Self {
        let value = value.into();
        let redacted = is_secret_like(name) || contains_private_material(&value);
        Self {
            value: (!redacted).then_some(value),
            redacted,
        }
    }
}

/// One environment variable observation or definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvFact {
    /// Environment variable name and normalized aliases.
    pub name: NormalizedName,
    /// Whether this is a read or a definition.
    pub role: FactRole,
    /// Analyzer/source family that produced the fact.
    pub source: FactSourceKind,
    /// Stable graph owner, when a source symbol has been identified.
    pub owner: Option<String>,
    /// Optional assignment/default value, stored only in secret-safe form.
    pub value: Option<SafeFactValue>,
    /// Source evidence for this fact.
    pub evidence: EvidenceRef,
    /// Deterministic confidence bucket.
    pub confidence: Confidence,
}

impl EnvFact {
    /// Creates a validated environment fact.
    pub(crate) fn new(
        name: impl Into<String>,
        role: FactRole,
        source: FactSourceKind,
        owner: Option<String>,
        value: Option<String>,
        evidence: EvidenceRef,
        confidence: Confidence,
    ) -> Result<Self, NameNormalizationError> {
        let name = NormalizedName::new(name)?;
        let value = value.map(|value| SafeFactValue::from_named_value(&name, value));
        Ok(Self {
            name,
            role,
            source,
            owner,
            value,
            evidence,
            confidence,
        })
    }
}

/// One configuration-key observation or definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConfigFact {
    /// Configuration key and normalized aliases.
    pub key: NormalizedName,
    /// Whether this is a definition or a source reference.
    pub role: FactRole,
    /// Analyzer/source family that produced the fact.
    pub source: FactSourceKind,
    /// Stable graph owner, when a source symbol has been identified.
    pub owner: Option<String>,
    /// Optional configured/default value, stored only in secret-safe form.
    pub value: Option<SafeFactValue>,
    /// Source evidence for this fact.
    pub evidence: EvidenceRef,
    /// Deterministic confidence bucket.
    pub confidence: Confidence,
}

impl ConfigFact {
    /// Creates a validated configuration fact.
    pub(crate) fn new(
        key: impl Into<String>,
        role: FactRole,
        source: FactSourceKind,
        owner: Option<String>,
        value: Option<String>,
        evidence: EvidenceRef,
        confidence: Confidence,
    ) -> Result<Self, NameNormalizationError> {
        let key = NormalizedName::new(key)?;
        let value = value.map(|value| SafeFactValue::from_named_value(&key, value));
        Ok(Self {
            key,
            role,
            source,
            owner,
            value,
            evidence,
            confidence,
        })
    }
}

/// Returns true for conservative key names whose values should not be stored.
pub(crate) fn is_secret_like(name: &NormalizedName) -> bool {
    const SECRET_TOKENS: &[&str] = &[
        "access_token",
        "credential",
        "credentials",
        "jwt",
        "password",
        "passwd",
        "private",
        "refresh_token",
        "secret",
        "signing",
        "token",
    ];
    name.tokens.iter().any(|token| {
        SECRET_TOKENS.contains(&token.as_str())
            || token.ends_with("secret")
            || token.ends_with("token")
            || token.ends_with("password")
    })
}

/// Summary of deterministic environment-to-config linking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct EnvironmentResolveReport {
    /// Authoritative links added to the graph.
    pub linked: usize,
    /// Environment variables with multiple distinct config candidates.
    pub ambiguous: usize,
}

/// Deterministic, evidence-backed explanation of environment graph facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvironmentExplanation {
    /// Variables included after applying the optional name filter.
    pub variables: Vec<EnvironmentVariableExplanation>,
}

/// Explanation for one environment variable node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvironmentVariableExplanation {
    /// Stable environment node identifier.
    pub id: GraphNodeId,
    /// Original environment variable spelling.
    pub name: String,
    /// Canonical normalized spelling.
    pub canonical: String,
    /// Authoritative environment-to-config links.
    pub resolved: Vec<EnvironmentResolvedLink>,
    /// Source nodes that read this variable.
    pub code_users: Vec<EnvironmentCodeUser>,
    /// Source nodes that define this variable.
    pub definitions: Vec<EnvironmentCodeUser>,
    /// Config keys considered but not linked authoritatively.
    pub candidates: Vec<EnvironmentCandidate>,
    /// Why no authoritative config link exists, when applicable.
    pub unresolved_reason: Option<String>,
}

/// One authoritative environment-to-config relation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvironmentResolvedLink {
    /// Relation identifier.
    pub relation_id: String,
    /// Canonical config key node identifier.
    pub config_key_id: GraphNodeId,
    /// Config key name.
    pub config_key: String,
    /// Relation confidence.
    pub confidence: Confidence,
    /// Evidence supporting the link.
    pub evidence: Vec<EvidenceRef>,
}

/// One environment read or definition source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvironmentCodeUser {
    /// Source graph node identifier.
    pub source: GraphNodeId,
    /// Relation confidence.
    pub confidence: Confidence,
    /// Evidence supporting the source relation.
    pub evidence: Vec<EvidenceRef>,
}

/// A config key considered by deterministic alias matching.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct EnvironmentCandidate {
    /// Candidate config key node identifier.
    pub config_key_id: GraphNodeId,
    /// Candidate config key name.
    pub config_key: String,
    /// Stable reason this candidate was retained.
    pub reason: String,
    /// Integer score, avoiding platform-dependent floating-point ordering.
    pub score: u32,
    /// Explainable feature contributions to `score`.
    pub features: EnvironmentCandidateFeatures,
}

/// Deterministic local similarity feature contributions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct EnvironmentCandidateFeatures {
    /// Shared normalized-token contribution.
    pub token_overlap: u32,
    /// Character trigram overlap contribution.
    pub character_ngrams: u32,
    /// Canonical containment contribution.
    pub containment: u32,
    /// Acronym match contribution.
    pub acronym: u32,
    /// Evidence-path similarity contribution.
    pub path: u32,
    /// Existing graph-neighborhood contribution.
    pub graph_proximity: u32,
}

impl EnvironmentCandidateFeatures {
    fn score(self) -> u32 {
        self.token_overlap
            + self.character_ngrams
            + self.containment
            + self.acronym
            + self.path
            + self.graph_proximity
    }
}

/// Explains environment variables and their graph-backed config links.
pub(crate) fn explain_environment(graph: &Graph, filter: Option<&str>) -> EnvironmentExplanation {
    let config_keys: Vec<(&GraphNodeId, &str, &EvidenceRef)> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Config(config) if config.kind == ConfigNodeKind::Key => {
                Some((&config.id, config.name.as_str(), &config.evidence))
            }
            _ => None,
        })
        .collect();
    let mut variables = graph
        .nodes
        .iter()
        .filter_map(|node| {
            let GraphNode::EnvVar(env) = node else {
                return None;
            };
            if filter.is_some_and(|value| value != env.name && value != env.id.as_str()) {
                return None;
            }
            let Ok(normalized) = NormalizedName::new(&env.name) else {
                return None;
            };
            let outgoing: Vec<&Relation> = graph
                .relations
                .iter()
                .filter(|relation| relation.source == env.id)
                .collect();
            let incoming: Vec<&Relation> = graph
                .relations
                .iter()
                .filter(|relation| relation.target == env.id)
                .collect();
            let mut resolved = outgoing
                .iter()
                .filter_map(|relation| {
                    if relation.kind != RelationKind::BindsConfig {
                        return None;
                    }
                    let (config_key_id, config_key, _) = config_keys
                        .iter()
                        .find(|(id, _, _)| **id == relation.target)
                        .copied()?;
                    Some(EnvironmentResolvedLink {
                        relation_id: relation.id.clone(),
                        config_key_id: config_key_id.clone(),
                        config_key: config_key.to_owned(),
                        confidence: relation.confidence,
                        evidence: relation.evidence.clone(),
                    })
                })
                .collect::<Vec<_>>();
            resolved.sort_by(|left, right| {
                (&left.config_key, &left.relation_id).cmp(&(&right.config_key, &right.relation_id))
            });
            let mut code_users = incoming
                .iter()
                .filter(|relation| relation.kind == RelationKind::ReadsEnv)
                .map(|relation| EnvironmentCodeUser {
                    source: relation.source.clone(),
                    confidence: relation.confidence,
                    evidence: relation.evidence.clone(),
                })
                .collect::<Vec<_>>();
            code_users.sort_by(|left, right| left.source.cmp(&right.source));
            let mut definitions = incoming
                .iter()
                .filter(|relation| relation.kind == RelationKind::DefinesEnv)
                .map(|relation| EnvironmentCodeUser {
                    source: relation.source.clone(),
                    confidence: relation.confidence,
                    evidence: relation.evidence.clone(),
                })
                .collect::<Vec<_>>();
            definitions.sort_by(|left, right| left.source.cmp(&right.source));
            let candidates = rank_environment_candidates(graph, &env.id, &normalized, &config_keys);
            let unresolved_reason = if !resolved.is_empty() {
                None
            } else if candidates
                .first()
                .is_some_and(|candidate| candidate.reason.contains("alias"))
                && candidates.len() > 1
            {
                Some("ambiguous aliases; no authoritative link created".to_owned())
            } else if !candidates.is_empty() {
                Some("ranked candidates did not meet the authoritative threshold".to_owned())
            } else {
                Some("no deterministic config-key alias matched".to_owned())
            };
            Some(EnvironmentVariableExplanation {
                id: env.id.clone(),
                name: env.name.clone(),
                canonical: normalized.canonical,
                resolved,
                code_users,
                definitions,
                candidates,
                unresolved_reason,
            })
        })
        .collect::<Vec<_>>();
    variables
        .sort_by(|left, right| (&left.canonical, &left.id).cmp(&(&right.canonical, &right.id)));
    EnvironmentExplanation { variables }
}

fn rank_environment_candidates(
    graph: &Graph,
    env_id: &GraphNodeId,
    environment: &NormalizedName,
    config_keys: &[(&GraphNodeId, &str, &EvidenceRef)],
) -> Vec<EnvironmentCandidate> {
    let environment_aliases: BTreeSet<String> = environment
        .aliases()
        .into_iter()
        .map(|alias| alias.value)
        .collect();
    let mut candidates = config_keys
        .iter()
        .filter_map(|(id, name, evidence)| {
            let config = NormalizedName::new(*name).ok()?;
            let alias_match = config
                .aliases()
                .iter()
                .any(|alias| environment_aliases.contains(&alias.value));
            let features = candidate_features(graph, env_id, environment, &config, evidence, id);
            let score = features.score();
            (alias_match || score > 0).then_some(EnvironmentCandidate {
                config_key_id: (*id).clone(),
                config_key: (*name).to_owned(),
                reason: if alias_match {
                    "deterministic exact/framework alias match".to_owned()
                } else {
                    "ranked local similarity candidate".to_owned()
                },
                score,
                features,
            })
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| left.config_key.cmp(&right.config_key))
            .then_with(|| left.config_key_id.cmp(&right.config_key_id))
    });
    candidates.truncate(3);
    candidates
}

fn candidate_features(
    graph: &Graph,
    env_id: &GraphNodeId,
    environment: &NormalizedName,
    config: &NormalizedName,
    config_evidence: &EvidenceRef,
    config_id: &GraphNodeId,
) -> EnvironmentCandidateFeatures {
    let shared_tokens = environment
        .tokens
        .iter()
        .filter(|token| config.tokens.contains(token))
        .count() as u32;
    let token_overlap = (shared_tokens * 100).min(300);
    let environment_ngrams = ngrams(&environment.canonical);
    let config_ngrams = ngrams(&config.canonical);
    let shared_ngrams = environment_ngrams.intersection(&config_ngrams).count() as u32;
    let total_ngrams = environment_ngrams.union(&config_ngrams).count() as u32;
    let character_ngrams = shared_ngrams
        .saturating_mul(250)
        .checked_div(total_ngrams)
        .unwrap_or_default()
        .min(250);
    let containment = if environment.canonical == config.canonical {
        150
    } else if environment.canonical.contains(&config.canonical)
        || config.canonical.contains(&environment.canonical)
    {
        100
    } else if environment
        .tokens
        .windows(config.tokens.len())
        .any(|window| window == config.tokens.as_slice())
        || config
            .tokens
            .windows(environment.tokens.len())
            .any(|window| window == environment.tokens.as_slice())
    {
        50
    } else {
        0
    };
    let acronym = if acronym(environment) == acronym(config) {
        100
    } else {
        0
    };
    let path = path_similarity(graph, env_id, config_evidence);
    let graph_proximity = graph_proximity(graph, env_id, config_id);
    EnvironmentCandidateFeatures {
        token_overlap,
        character_ngrams,
        containment,
        acronym,
        path,
        graph_proximity,
    }
}

fn ngrams(value: &str) -> BTreeSet<String> {
    let characters: Vec<char> = value.chars().collect();
    if characters.len() < 3 {
        return characters
            .into_iter()
            .map(|character| character.to_string())
            .collect();
    }
    characters
        .windows(3)
        .map(|window| window.iter().collect::<String>())
        .collect()
}

fn acronym(name: &NormalizedName) -> String {
    name.tokens
        .iter()
        .filter_map(|token| token.chars().next())
        .collect()
}

fn path_similarity(graph: &Graph, env_id: &GraphNodeId, config_evidence: &EvidenceRef) -> u32 {
    if graph
        .relations
        .iter()
        .filter(|relation| relation.target == *env_id)
        .flat_map(|relation| relation.evidence.iter())
        .any(|evidence| evidence.path == config_evidence.path)
    {
        100
    } else {
        0
    }
}

fn graph_proximity(graph: &Graph, env_id: &GraphNodeId, config_id: &GraphNodeId) -> u32 {
    let users: BTreeSet<&GraphNodeId> = graph
        .relations
        .iter()
        .filter(|relation| relation.target == *env_id && relation.kind == RelationKind::ReadsEnv)
        .map(|relation| &relation.source)
        .collect();
    if graph.relations.iter().any(|relation| {
        users.contains(&&relation.source)
            && relation.target == *config_id
            && matches!(
                relation.kind,
                RelationKind::BindsConfig | RelationKind::ReferencesConfig
            )
    }) {
        100
    } else {
        0
    }
}

/// Adds high-confidence environment-to-config links when aliases identify one
/// canonical config key. Ambiguous aliases are deliberately left untouched.
pub(crate) fn resolve_environment_links(graph: &mut Graph) -> EnvironmentResolveReport {
    let mut aliases = BTreeMap::<String, BTreeSet<GraphNodeId>>::new();
    for node in &graph.nodes {
        let GraphNode::Config(config) = node else {
            continue;
        };
        if config.kind != ConfigNodeKind::Key {
            continue;
        }
        let Ok(name) = NormalizedName::new(&config.name) else {
            continue;
        };
        for alias in name.aliases() {
            aliases
                .entry(alias.value)
                .or_default()
                .insert(config.id.clone());
        }
    }

    let env_nodes: Vec<GraphNodeId> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::EnvVar(env) => Some(env.id.clone()),
            _ => None,
        })
        .collect();
    let mut additions = Vec::<(GraphNodeId, GraphNodeId)>::new();
    let mut report = EnvironmentResolveReport::default();
    for env_id in env_nodes {
        let Some(GraphNode::EnvVar(env)) = graph.nodes.iter().find(|node| node.id() == &env_id)
        else {
            continue;
        };
        let Ok(name) = NormalizedName::new(&env.name) else {
            continue;
        };
        let mut candidates = BTreeSet::new();
        for alias in name.aliases() {
            if let Some(ids) = aliases.get(&alias.value) {
                candidates.extend(ids.iter().cloned());
            }
        }
        match candidates.len() {
            1 => {
                let Some(target) = candidates.into_iter().next() else {
                    continue;
                };
                if !graph.relations.iter().any(|relation| {
                    relation.source == env_id
                        && relation.target == target
                        && relation.kind == RelationKind::BindsConfig
                }) {
                    additions.push((env_id, target));
                }
            }
            value if value > 1 => report.ambiguous += 1,
            _ => {}
        }
    }

    additions.sort();
    let next_relation = graph
        .relations
        .iter()
        .filter_map(|relation| relation.id.strip_prefix("relation:"))
        .filter_map(|value| value.parse::<usize>().ok())
        .max()
        .unwrap_or(graph.relations.len())
        + 1;
    for (offset, (source, target)) in additions.into_iter().enumerate() {
        let mut evidence = relation_evidence(graph, &source, &target);
        if evidence.is_empty() {
            evidence = node_evidence(graph, &target);
        }
        graph.relations.push(Relation {
            id: format!("relation:{}", next_relation + offset),
            source,
            target,
            kind: RelationKind::BindsConfig,
            confidence: Confidence::High,
            evidence,
            provenance: Some(RelationProvenance {
                language: Some("environment".to_owned()),
                resolver_strategy: "environment-relaxed-alias".to_owned(),
                resolution: RelationResolution::HybridResolved,
                confidence: Confidence::High,
            }),
        });
        report.linked += 1;
    }
    graph.relations.sort_by(|a, b| {
        (&a.source, a.kind, &a.target, &a.id).cmp(&(&b.source, b.kind, &b.target, &b.id))
    });
    report
}

fn relation_evidence(
    graph: &Graph,
    source: &GraphNodeId,
    target: &GraphNodeId,
) -> Vec<EvidenceRef> {
    graph
        .relations
        .iter()
        .filter(|relation| {
            relation.source == *source
                && (relation.target == *target
                    || matches!(
                        relation.kind,
                        RelationKind::ReadsEnv | RelationKind::DefinesEnv
                    ))
        })
        .flat_map(|relation| relation.evidence.clone())
        .collect()
}

fn node_evidence(graph: &Graph, node_id: &GraphNodeId) -> Vec<EvidenceRef> {
    graph
        .nodes
        .iter()
        .find(|node| node.id() == node_id)
        .and_then(|node| match node {
            GraphNode::Config(config) => Some(vec![config.evidence.clone()]),
            _ => None,
        })
        .unwrap_or_default()
}

fn contains_private_material(value: &str) -> bool {
    let upper = value.to_ascii_uppercase();
    upper.contains("BEGIN PRIVATE KEY") || upper.contains("BEGIN RSA PRIVATE KEY")
}

fn add_alias(aliases: &mut BTreeMap<String, NameAliasKind>, value: &str, kind: NameAliasKind) {
    if !value.is_empty() {
        aliases.entry(value.to_owned()).or_insert(kind);
    }
}

fn camel_case(tokens: &[String], pascal: bool) -> String {
    tokens
        .iter()
        .enumerate()
        .map(|(index, token)| {
            if index == 0 && !pascal {
                token.clone()
            } else {
                let mut chars = token.chars();
                chars
                    .next()
                    .map(|first| format!("{}{}", first.to_ascii_uppercase(), chars.as_str()))
                    .unwrap_or_default()
            }
        })
        .collect()
}

fn split_identifier(value: &str) -> Vec<String> {
    let chars: Vec<char> = value.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (index, character) in chars.iter().copied().enumerate() {
        if !character.is_ascii_alphanumeric() {
            push_token(&mut tokens, &mut current);
            continue;
        }
        let previous = index
            .checked_sub(1)
            .and_then(|position| chars.get(position));
        let next = chars.get(index + 1);
        let boundary = previous.is_some_and(|previous| {
            previous.is_ascii_alphabetic() && character.is_ascii_digit()
                || previous.is_ascii_digit() && character.is_ascii_alphabetic()
                || previous.is_ascii_lowercase() && character.is_ascii_uppercase()
                || previous.is_ascii_uppercase()
                    && character.is_ascii_uppercase()
                    && next.is_some_and(|next| next.is_ascii_lowercase())
        });
        if boundary {
            push_token(&mut tokens, &mut current);
        }
        current.push(character.to_ascii_lowercase());
    }
    push_token(&mut tokens, &mut current);
    tokens
}

fn push_token(tokens: &mut Vec<String>, current: &mut String) {
    if !current.is_empty() {
        tokens.push(std::mem::take(current));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactId, RepoPath};
    use std::collections::BTreeSet;

    fn evidence() -> Result<EvidenceRef, Box<dyn std::error::Error>> {
        let path = RepoPath::new("config/application.yml")?;
        Ok(EvidenceRef::file(ArtifactId::from_path(&path), path))
    }

    #[test]
    fn normalization_handles_separators_camel_case_acronyms_and_numbers()
    -> Result<(), Box<dyn std::error::Error>> {
        let name = NormalizedName::new("vehicleCacheTTLSeconds")?;
        assert_eq!(name.tokens, ["vehicle", "cache", "ttl", "seconds"]);
        assert_eq!(name.canonical, "vehicle.cache.ttl.seconds");

        let indexed = NormalizedName::new("my.service[0].other")?;
        assert_eq!(indexed.tokens, ["my", "service", "0", "other"]);
        assert_eq!(indexed.canonical, "my.service.0.other");
        Ok(())
    }

    #[test]
    fn aliases_cover_spring_relaxed_binding_forms() -> Result<(), Box<dyn std::error::Error>> {
        let name = NormalizedName::new("spring.datasource.url")?;
        let aliases: BTreeSet<_> = name
            .aliases()
            .into_iter()
            .map(|alias| alias.value)
            .collect();
        assert!(aliases.contains("spring.datasource.url"));
        assert!(aliases.contains("spring-datasource-url"));
        assert!(aliases.contains("springDatasourceUrl"));
        assert!(aliases.contains("SPRING_DATASOURCE_URL"));
        Ok(())
    }

    #[test]
    fn original_spelling_prevents_canonical_collision_from_becoming_lossy()
    -> Result<(), Box<dyn std::error::Error>> {
        let hyphenated = NormalizedName::new("service-url")?;
        let dotted = NormalizedName::new("service.url")?;
        assert_eq!(hyphenated.canonical, dotted.canonical);
        assert_ne!(hyphenated.original, dotted.original);
        Ok(())
    }

    #[test]
    fn facts_retain_provenance_and_redact_secret_values() -> Result<(), Box<dyn std::error::Error>>
    {
        let fact = EnvFact::new(
            "DB_PASSWORD",
            FactRole::Define,
            FactSourceKind::Dotenv,
            Some("artifact:.env".to_owned()),
            Some("do-not-persist".to_owned()),
            evidence()?,
            Confidence::High,
        )?;
        assert_eq!(fact.owner.as_deref(), Some("artifact:.env"));
        assert!(fact.value.as_ref().is_some_and(|value| value.redacted));
        assert!(
            fact.value
                .as_ref()
                .is_some_and(|value| value.value.is_none())
        );
        let json = serde_json::to_string(&fact)?;
        assert!(!json.contains("do-not-persist"));
        Ok(())
    }

    #[test]
    fn non_secret_values_are_retained_and_empty_names_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let fact = ConfigFact::new(
            "server.port",
            FactRole::Define,
            FactSourceKind::StructuredConfig,
            None,
            Some("8080".to_owned()),
            evidence()?,
            Confidence::High,
        )?;
        assert_eq!(
            fact.value.as_ref().and_then(|value| value.value.as_deref()),
            Some("8080")
        );
        assert!(NormalizedName::new("---").is_err());
        Ok(())
    }

    fn config_node(id: &str, name: &str, evidence: EvidenceRef) -> GraphNode {
        GraphNode::Config(crate::graph::ConfigNode {
            id: GraphNodeId::new(id),
            kind: ConfigNodeKind::Key,
            name: name.to_owned(),
            evidence,
        })
    }

    fn env_node(id: &str, name: &str) -> GraphNode {
        GraphNode::EnvVar(crate::graph::EnvVarNode {
            id: GraphNodeId::new(id),
            name: name.to_owned(),
        })
    }

    #[test]
    fn links_one_high_confidence_alias_and_is_idempotent() -> Result<(), Box<dyn std::error::Error>>
    {
        let evidence = evidence()?;
        let mut graph = Graph {
            nodes: vec![
                env_node("env:SPRING_DATASOURCE_URL", "SPRING_DATASOURCE_URL"),
                config_node(
                    "config-key:spring.datasource.url",
                    "spring.datasource.url",
                    evidence,
                ),
            ],
            relations: Vec::new(),
        };

        let first = resolve_environment_links(&mut graph);
        assert_eq!(
            first,
            EnvironmentResolveReport {
                linked: 1,
                ambiguous: 0
            }
        );
        let Some(relation) = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::BindsConfig)
        else {
            return Err("resolver should add a binding".into());
        };
        assert_eq!(relation.confidence, Confidence::High);
        assert_eq!(
            relation
                .provenance
                .as_ref()
                .map(|provenance| provenance.resolution),
            Some(RelationResolution::HybridResolved)
        );

        let second = resolve_environment_links(&mut graph);
        assert_eq!(second, EnvironmentResolveReport::default());
        assert_eq!(graph.relations.len(), 1);
        Ok(())
    }

    #[test]
    fn leaves_ambiguous_relaxed_aliases_unresolved() -> Result<(), Box<dyn std::error::Error>> {
        let evidence = evidence()?;
        let mut graph = Graph {
            nodes: vec![
                env_node("env:URL", "URL"),
                config_node("config-key:url-a", "url", evidence.clone()),
                config_node("config-key:url-b", "URL", evidence),
            ],
            relations: Vec::new(),
        };

        let report = resolve_environment_links(&mut graph);
        assert_eq!(
            report,
            EnvironmentResolveReport {
                linked: 0,
                ambiguous: 1
            }
        );
        assert!(graph.relations.is_empty());
        Ok(())
    }

    #[test]
    fn explanation_reports_links_users_and_ambiguous_candidates()
    -> Result<(), Box<dyn std::error::Error>> {
        let evidence = evidence()?;
        let mut graph = Graph {
            nodes: vec![
                env_node("env:URL", "URL"),
                config_node("config-key:url-a", "url", evidence.clone()),
                config_node("config-key:url-b", "URL", evidence.clone()),
            ],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source: GraphNodeId::new("symbol:config.py#read_url"),
                target: GraphNodeId::new("env:URL"),
                kind: RelationKind::ReadsEnv,
                confidence: Confidence::High,
                evidence: vec![evidence],
                provenance: None,
            }],
        };
        let report = resolve_environment_links(&mut graph);
        assert_eq!(report.ambiguous, 1);

        let explanation = explain_environment(&graph, Some("URL"));
        assert_eq!(explanation.variables.len(), 1);
        let variable = &explanation.variables[0];
        assert!(variable.resolved.is_empty());
        assert_eq!(variable.code_users.len(), 1);
        assert_eq!(variable.candidates.len(), 2);
        assert_eq!(
            variable.unresolved_reason.as_deref(),
            Some("ambiguous aliases; no authoritative link created")
        );
        assert_eq!(
            serde_json::to_string(&explanation)?,
            serde_json::to_string(&explain_environment(&graph, Some("URL")))?
        );
        Ok(())
    }

    #[test]
    fn labeled_candidate_fixture_reports_reproducible_precision_metrics()
    -> Result<(), Box<dyn std::error::Error>> {
        let evidence = evidence()?;
        let graph = Graph {
            nodes: vec![
                env_node("env:DATABASE_URL", "DATABASE_URL"),
                env_node("env:WORKER_CACHE_DIR", "WORKER_CACHE_DIR"),
                config_node("config-key:database.url", "database.url", evidence.clone()),
                config_node(
                    "config-key:worker.cache.dir",
                    "worker.cache.dir",
                    evidence.clone(),
                ),
                config_node("config-key:service.name", "service.name", evidence),
            ],
            relations: Vec::new(),
        };
        let labels = [
            ("DATABASE_URL", "database.url"),
            ("WORKER_CACHE_DIR", "worker.cache.dir"),
        ];
        let explanation = explain_environment(&graph, None);
        let mut precision_at_1 = 0;
        let mut precision_at_3 = 0;
        let mut recall = 0;
        for (variable, expected) in labels {
            let item = explanation
                .variables
                .iter()
                .find(|item| item.name == variable)
                .ok_or("missing labeled variable")?;
            if item
                .candidates
                .first()
                .is_some_and(|candidate| candidate.config_key == expected)
            {
                precision_at_1 += 1;
            }
            if item
                .candidates
                .iter()
                .take(3)
                .any(|candidate| candidate.config_key == expected)
            {
                precision_at_3 += 1;
                recall += 1;
            }
        }
        assert_eq!(precision_at_1, 2);
        assert_eq!(precision_at_3, 2);
        assert_eq!(recall, 2);
        // Ranking is review-only: no ambiguous/fuzzy candidate is auto-linked.
        let false_positive_auto_links = explanation
            .variables
            .iter()
            .flat_map(|item| item.resolved.iter())
            .count();
        assert_eq!(false_positive_auto_links, 0);
        assert_eq!(
            serde_json::to_string(&explanation)?,
            serde_json::to_string(&explain_environment(&graph, None))?
        );
        Ok(())
    }
}
