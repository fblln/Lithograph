//! Typed deterministic tags and taxonomy queries over graph entities.
use crate::domain::Confidence;
use crate::graph::Graph;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::collections::BTreeSet;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum TagSource {
    Parser,
    Path,
    DependencyRole,
    Architecture,
    Tension,
    User,
    Agent,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct GraphTag {
    pub id: String,
    pub entity_id: String,
    pub namespace: String,
    pub value: String,
    pub source: TagSource,
    pub confidence: Confidence,
    pub evidence: Vec<String>,
    pub inherited_from: Option<String>,
    pub graph_snapshot_id: String,
}
#[allow(missing_docs)]
impl GraphTag {
    pub fn new(
        entity_id: impl Into<String>,
        namespace: impl Into<String>,
        value: impl Into<String>,
        source: TagSource,
        graph_snapshot_id: impl Into<String>,
    ) -> Self {
        let entity_id = entity_id.into();
        let namespace = namespace.into();
        let value = value.into();
        let id = format!(
            "tag:{}",
            blake3::hash(format!("{entity_id}:{namespace}:{value}:{source:?}").as_bytes()).to_hex()
        );
        Self {
            id,
            entity_id,
            namespace,
            value,
            source,
            confidence: Confidence::High,
            evidence: vec![],
            inherited_from: None,
            graph_snapshot_id: graph_snapshot_id.into(),
        }
    }
}
#[derive(Debug, Clone, Default)]
#[allow(missing_docs)]
pub struct TagIndex {
    tags: Vec<GraphTag>,
}
#[allow(missing_docs)]
impl TagIndex {
    pub fn new(mut tags: Vec<GraphTag>) -> Self {
        tags.sort_by(|a, b| a.id.cmp(&b.id));
        tags.dedup_by(|a, b| a.id == b.id);
        Self { tags }
    }
    pub fn query(&self, include: &[(&str, &str)], exclude: &[(&str, &str)]) -> Vec<String> {
        let values: BTreeSet<_> = self.tags.iter().map(|tag| tag.entity_id.clone()).collect();
        let values: BTreeSet<_> = values
            .into_iter()
            .filter(|entity| {
                include.iter().all(|(namespace, value)| {
                    self.tags.iter().any(|tag| {
                        tag.entity_id == *entity
                            && tag.namespace == *namespace
                            && tag.value == *value
                    })
                }) && !exclude.iter().any(|(namespace, value)| {
                    self.tags.iter().any(|tag| {
                        tag.entity_id == *entity
                            && tag.namespace == *namespace
                            && tag.value == *value
                    })
                })
            })
            .collect();
        values.into_iter().collect()
    }
    pub fn namespace(&self, namespace: &str) -> Vec<&GraphTag> {
        self.tags
            .iter()
            .filter(|tag| tag.namespace == namespace)
            .collect()
    }
    /// Returns all tags in stable id order.
    pub fn all(&self) -> &[GraphTag] {
        &self.tags
    }
    /// Finds tags whose canonical `namespace:value` begins with a prefix.
    pub fn search_prefix(&self, prefix: &str) -> Vec<&GraphTag> {
        self.tags
            .iter()
            .filter(|tag| format!("{}:{}", tag.namespace, tag.value).starts_with(prefix))
            .collect()
    }
    /// Returns stable `namespace:value` facet counts.
    pub fn facets(&self) -> BTreeMap<String, usize> {
        let mut facets = BTreeMap::new();
        for tag in &self.tags {
            *facets
                .entry(format!("{}:{}", tag.namespace, tag.value))
                .or_default() += 1;
        }
        facets
    }
}
/// Resolves a compact `namespace:value` expression with comma-union and `!` exclusions.
pub fn resolve_expression(index: &TagIndex, expression: &str) -> Result<Vec<String>, String> {
    let mut union = BTreeSet::new();
    for branch in expression.split(';') {
        let mut include = Vec::new();
        let mut exclude = Vec::new();
        for term in branch.split(',').filter(|term| !term.is_empty()) {
            let (negated, term) = term
                .strip_prefix('!')
                .map_or((false, term), |value| (true, value));
            let Some((namespace, value)) = term.split_once(':') else {
                return Err(format!("invalid tag expression: {term}"));
            };
            if namespace.is_empty() || value.is_empty() {
                return Err(format!("invalid tag expression: {term}"));
            }
            if negated {
                exclude.push((namespace, value));
            } else {
                include.push((namespace, value));
            }
        }
        if include.is_empty() {
            return Err("tag expression needs an include term".into());
        }
        union.extend(index.query(&include, &exclude));
    }
    if union.is_empty() && expression.trim().is_empty() {
        return Err("tag expression needs an include term".into());
    }
    Ok(union.into_iter().collect())
}
/// Derives conservative parser/path-style tags from stable graph identifiers.
pub fn derive_tags(graph: &Graph, snapshot: &str) -> Vec<GraphTag> {
    let mut tags = Vec::new();
    for node in &graph.nodes {
        let id = node.id().as_str();
        let value = id.split(':').next().unwrap_or("graph");
        tags.push(GraphTag::new(
            id,
            "kind",
            value,
            TagSource::Parser,
            snapshot,
        ));
        if id.contains("test") {
            tags.push(GraphTag::new(id, "role", "test", TagSource::Path, snapshot));
        }
    }
    TagIndex::new(tags).tags
}
/// Inherits a cluster or subsystem tag while retaining its exact provenance.
pub fn inherit_tag(parent: &GraphTag, entity_id: impl Into<String>) -> GraphTag {
    let mut tag = GraphTag::new(
        entity_id,
        parent.namespace.clone(),
        parent.value.clone(),
        parent.source,
        parent.graph_snapshot_id.clone(),
    );
    tag.inherited_from = Some(parent.id.clone());
    tag.confidence = parent.confidence;
    tag.evidence = parent.evidence.clone();
    tag
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tags_are_stable_and_queryable() {
        let a = GraphTag::new("symbol:a", "layer", "api", TagSource::Path, "g1");
        let b = GraphTag::new("symbol:a", "risk", "high", TagSource::Tension, "g1");
        let index = TagIndex::new(vec![a.clone(), b]);
        assert_eq!(
            a.id,
            GraphTag::new("symbol:a", "layer", "api", TagSource::Path, "g1").id
        );
        assert_eq!(index.query(&[("layer", "api")], &[]), vec!["symbol:a"]);
        assert!(index.namespace("risk").len() == 1);
    }
    #[test]
    fn inherited_tags_preserve_provenance_and_serialize() -> Result<(), Box<dyn std::error::Error>>
    {
        let parent = GraphTag::new(
            "cluster:payments",
            "owner",
            "payments",
            TagSource::User,
            "g1",
        );
        let child = inherit_tag(&parent, "symbol:charge");
        assert_eq!(child.inherited_from.as_deref(), Some(parent.id.as_str()));
        assert_eq!(
            serde_json::from_str::<GraphTag>(&serde_json::to_string(&child)?)?,
            child
        );
        Ok(())
    }
    #[test]
    fn expressions_support_exclusions_and_reject_invalid_input()
    -> Result<(), Box<dyn std::error::Error>> {
        let index = TagIndex::new(vec![
            GraphTag::new("symbol:a", "layer", "api", TagSource::Path, "g1"),
            GraphTag::new("symbol:a", "risk", "high", TagSource::Tension, "g1"),
        ]);
        assert_eq!(
            resolve_expression(&index, "layer:api,!risk:high")?,
            Vec::<String>::new()
        );
        assert!(resolve_expression(&index, "bad").is_err());
        assert_eq!(
            resolve_expression(&index, "layer:api;risk:high")?,
            vec!["symbol:a"]
        );
        Ok(())
    }
}
