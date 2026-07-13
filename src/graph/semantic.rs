//! Local, deterministic semantic class profiling and explainable filtering.

use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind, SymbolKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// A deterministic, inspectable profile for one class-like symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticClassProfile {
    /// Profiled class, struct, or trait node.
    pub node_id: GraphNodeId,
    /// Qualified declaration name.
    pub name: String,
    /// Source path.
    pub path: String,
    /// Declaration documentation, if present.
    pub docs: String,
    /// Neighbor names grouped by semantic relation kind.
    pub annotations: Vec<String>,
    /// Inherited base classes and implemented interfaces.
    pub base_classes: Vec<String>,
    /// Owned methods.
    pub methods: Vec<String>,
    /// Inferred member fields.
    pub fields: Vec<String>,
    /// Imported symbols and modules.
    pub imports: Vec<String>,
    /// Direct call targets or callers.
    pub calls: Vec<String>,
    /// Associated packages.
    pub packages: Vec<String>,
}

/// Explainable score components. `vector` remains zero unless an optional
/// offline embedding implementation is supplied by a later caller.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticScore {
    /// Exact and token-based local text matches.
    pub lexical: f64,
    /// Domain-role vocabulary matches.
    pub taxonomy: f64,
    /// Optional offline vector contribution (zero by default).
    pub vector: f64,
    /// Imported, called, and package-neighbor evidence.
    pub graph_proximity: f64,
    /// Method, field, and inheritance evidence.
    pub structural: f64,
}

impl SemanticScore {
    /// Returns the complete explainable score.
    pub fn total(&self) -> f64 {
        self.lexical + self.taxonomy + self.vector + self.graph_proximity + self.structural
    }
}

/// One locally ranked semantic-filter result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SemanticClassMatch {
    /// Matched local profile.
    pub profile: SemanticClassProfile,
    /// Evidence breakdown for this match.
    pub score: SemanticScore,
}

/// Builds profiles only from local typed graph facts.
pub fn class_profiles(graph: &Graph) -> Vec<SemanticClassProfile> {
    let names: BTreeMap<_, _> = graph
        .nodes
        .iter()
        .map(|node| (node.id().clone(), node_name(node)))
        .collect();
    let packages: BTreeSet<_> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Package(package) => Some(package.id.clone()),
            _ => None,
        })
        .collect();
    let mut profiles = Vec::new();
    for node in &graph.nodes {
        let GraphNode::Symbol(symbol) = node else {
            continue;
        };
        if !matches!(
            symbol.kind,
            SymbolKind::Class | SymbolKind::Struct | SymbolKind::Trait
        ) {
            continue;
        }
        let mut profile = SemanticClassProfile {
            node_id: symbol.id.clone(),
            name: symbol.qualified_name.clone(),
            path: symbol.evidence.path.to_string(),
            docs: symbol.doc.clone().unwrap_or_default(),
            annotations: vec![],
            base_classes: vec![],
            methods: vec![],
            fields: vec![],
            imports: vec![],
            calls: vec![],
            packages: vec![],
        };
        for edge in &graph.relations {
            let (other, outbound) = if edge.source == symbol.id {
                (&edge.target, true)
            } else if edge.target == symbol.id {
                (&edge.source, false)
            } else {
                continue;
            };
            let value = names
                .get(other)
                .cloned()
                .unwrap_or_else(|| other.to_string());
            match edge.kind {
                RelationKind::Decorates if outbound => profile.annotations.push(value),
                RelationKind::Inherits | RelationKind::Implements if outbound => {
                    profile.base_classes.push(value)
                }
                RelationKind::HasMethod if outbound => profile.methods.push(value),
                RelationKind::MemberOf if !outbound => profile.fields.push(value),
                RelationKind::Imports => profile.imports.push(value),
                RelationKind::Calls => profile.calls.push(value),
                RelationKind::BelongsToPackage if outbound && packages.contains(other) => {
                    profile.packages.push(value)
                }
                _ => {}
            }
        }
        normalize_profile(&mut profile);
        profiles.push(profile);
    }
    profiles.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    profiles
}

/// Ranks class profiles with lexical, taxonomy, local graph, and structural evidence.
pub fn filter_classes(graph: &Graph, query: &str) -> Vec<SemanticClassMatch> {
    let tokens: BTreeSet<_> = query
        .to_ascii_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut matches: Vec<_> = class_profiles(graph)
        .into_iter()
        .filter_map(|profile| {
            let haystack = profile_text(&profile).to_ascii_lowercase();
            let lexical = tokens
                .iter()
                .filter(|token| haystack.contains(token.as_str()))
                .count() as f64;
            let taxonomy = role_tokens(query)
                .iter()
                .filter(|token| haystack.contains(token.as_str()))
                .count() as f64;
            let graph_proximity =
                (profile.imports.len() + profile.calls.len() + profile.packages.len()) as f64
                    / 10.0;
            let structural =
                (profile.methods.len() + profile.fields.len() + profile.base_classes.len()) as f64
                    / 10.0;
            let score = SemanticScore {
                lexical,
                taxonomy,
                vector: 0.0,
                graph_proximity,
                structural,
            };
            (score.total() > 0.0).then_some(SemanticClassMatch { profile, score })
        })
        .collect();
    matches.sort_by(|a, b| {
        b.score
            .total()
            .total_cmp(&a.score.total())
            .then(a.profile.node_id.cmp(&b.profile.node_id))
    });
    matches
}

fn node_name(node: &GraphNode) -> String {
    match node {
        GraphNode::Symbol(value) => value.qualified_name.clone(),
        GraphNode::Package(value) => value.name.clone(),
        _ => node.id().to_string(),
    }
}
fn normalize(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}
fn normalize_profile(profile: &mut SemanticClassProfile) {
    for values in [
        &mut profile.annotations,
        &mut profile.base_classes,
        &mut profile.methods,
        &mut profile.fields,
        &mut profile.imports,
        &mut profile.calls,
        &mut profile.packages,
    ] {
        normalize(values);
    }
}
fn profile_text(profile: &SemanticClassProfile) -> String {
    format!(
        "{} {} {} {} {} {} {} {} {}",
        profile.name,
        profile.path,
        profile.docs,
        profile.annotations.join(" "),
        profile.base_classes.join(" "),
        profile.methods.join(" "),
        profile.fields.join(" "),
        profile.imports.join(" "),
        profile.calls.join(" ")
    )
}
fn role_tokens(query: &str) -> Vec<String> {
    let query = query.to_ascii_lowercase();
    let mut tokens = Vec::new();
    if query.contains("controller") {
        tokens.extend(["controller", "route", "handler"]);
    }
    if query.contains("persistence") {
        tokens.extend(["repository", "adapter", "store", "database"]);
    }
    if query.contains("test") {
        tokens.extend(["test", "fixture", "mock", "util"]);
    }
    if query.contains("payment") {
        tokens.push("payment");
        if tokens.len() == 1 {
            tokens.extend(["service", "charge"]);
        }
    }
    tokens.sort_unstable();
    tokens.dedup();
    tokens.into_iter().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath};
    use crate::graph::{Relation, SymbolNode};
    fn symbol(
        id: &str,
        name: &str,
        kind: SymbolKind,
    ) -> Result<GraphNode, Box<dyn std::error::Error>> {
        let path = RepoPath::new("src/payment.rs")?;
        Ok(GraphNode::Symbol(SymbolNode {
            id: GraphNodeId::new(id),
            kind,
            qualified_name: name.into(),
            doc: Some(name.into()),
            evidence: EvidenceRef::file(ArtifactId::from_path(&path), path),
        }))
    }
    fn edge(from: &str, to: &str, kind: RelationKind) -> Relation {
        Relation {
            id: format!("{from}-{to}"),
            source: GraphNodeId::new(from),
            target: GraphNodeId::new(to),
            kind,
            confidence: Confidence::High,
            evidence: vec![],
            provenance: None,
        }
    }
    #[test]
    fn profiles_and_filters_are_local_explainable_and_deterministic()
    -> Result<(), Box<dyn std::error::Error>> {
        let graph = Graph {
            nodes: vec![
                symbol("payment", "PaymentService", SymbolKind::Class)?,
                symbol("charge", "charge", SymbolKind::Method)?,
            ],
            relations: vec![edge("payment", "charge", RelationKind::HasMethod)],
        };
        let profile = class_profiles(&graph).remove(0);
        assert_eq!(profile.methods, vec!["charge"]);
        let results = filter_classes(&graph, "payment services");
        assert!(results[0].score.lexical > 0.0 && results[0].score.taxonomy > 0.0);
        assert_eq!(results, filter_classes(&graph, "payment services"));
        Ok(())
    }
}
