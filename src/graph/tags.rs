//! Typed deterministic tags and taxonomy queries over graph entities.
use crate::architecture::{LayerDetector, LayerKind};
use crate::domain::Confidence;
use crate::graph::{ArchitectureCluster, Graph, GraphNode, Relation, RepositoryTension};
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
    let detected_layers = LayerDetector.detect(graph);
    let layers: BTreeMap<&str, LayerKind> = detected_layers
        .iter()
        .map(|layer| (layer.artifact_path.as_str(), layer.layer))
        .collect();
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
        if let Some(path) = artifact_path(node)
            && let Some(layer) = layers.get(path)
        {
            tags.push(GraphTag::new(
                id,
                "layer",
                layer_value(*layer),
                TagSource::Architecture,
                snapshot,
            ));
        }
    }
    TagIndex::new(tags).tags
}

/// Returns the repository-relative artifact path a node's evidence is
/// attached to, when it carries evidence -- `Package`, `EnvVar`, `Container`,
/// and `Unresolved` nodes have no single owning artifact and get no layer tag.
fn artifact_path(node: &GraphNode) -> Option<&str> {
    match node {
        GraphNode::Artifact(artifact) => Some(artifact.path.as_str()),
        GraphNode::Symbol(symbol) => Some(symbol.evidence.path.as_str()),
        GraphNode::Config(config) => Some(config.evidence.path.as_str()),
        GraphNode::Documentation(doc) => Some(doc.evidence.path.as_str()),
        GraphNode::Command(command) => Some(command.evidence.path.as_str()),
        GraphNode::Module(module) => Some(module.evidence.path.as_str()),
        GraphNode::Rationale(rationale) => Some(rationale.evidence.path.as_str()),
        GraphNode::Container(_) | GraphNode::EnvVar(_) | GraphNode::Package(_) => None,
        GraphNode::Unresolved(_) => None,
    }
}

/// Lowercase tag value for a `LayerKind`, matching this module's namespace
/// value conventions (e.g. `kind`, `role`).
fn layer_value(layer: LayerKind) -> &'static str {
    match layer {
        LayerKind::Ui => "ui",
        LayerKind::Api => "api",
        LayerKind::Domain => "domain",
        LayerKind::Data => "data",
        LayerKind::Infra => "infra",
        LayerKind::Test => "test",
        LayerKind::Unknown => "unknown",
    }
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

/// Builds display-only tags for a relation without adding them to the
/// node-scope tag index.
pub fn relation_display_tags(relation: &Relation, snapshot: &str) -> Vec<GraphTag> {
    let mut tag = GraphTag::new(
        &relation.id,
        "relation",
        format!("{:?}", relation.kind).to_ascii_lowercase(),
        TagSource::DependencyRole,
        snapshot,
    );
    tag.confidence = relation.confidence;
    tag.evidence = relation
        .evidence
        .iter()
        .map(|evidence| evidence.path.as_str().to_owned())
        .collect();
    let mut tags = vec![tag];
    if let Some(provenance) = &relation.provenance {
        let mut resolution = GraphTag::new(
            &relation.id,
            "resolution",
            &provenance.resolver_strategy,
            TagSource::DependencyRole,
            snapshot,
        );
        resolution.confidence = provenance.confidence;
        resolution.evidence = tags[0].evidence.clone();
        tags.push(resolution);
    }
    tags
}

/// Builds display-only architecture tags for a cluster. Member ids are kept
/// as provenance evidence; these tags never participate in node filtering.
pub fn cluster_display_tags(cluster: &ArchitectureCluster, snapshot: &str) -> Vec<GraphTag> {
    let mut tag = GraphTag::new(
        &cluster.id,
        "kind",
        "cluster",
        TagSource::Architecture,
        snapshot,
    );
    tag.evidence = cluster
        .members
        .iter()
        .map(|member| member.as_str().to_owned())
        .collect();
    vec![tag]
}

/// Builds display-only risk tags for one deterministic tension.
pub fn tension_display_tags(tension: &RepositoryTension, snapshot: &str) -> Vec<GraphTag> {
    let mut tag = GraphTag::new(
        &tension.id,
        "risk",
        format!("{:?}", tension.severity).to_ascii_lowercase(),
        TagSource::Tension,
        snapshot,
    );
    tag.confidence = tension.confidence;
    tag.evidence = tension.evidence_references.clone();
    vec![tag]
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{ArtifactCategory, ArtifactId, EvidenceRef, RepoPath};
    use crate::graph::model::{ArtifactNode, GraphNodeId, PackageNode, SymbolKind, SymbolNode};
    use crate::graph::{
        HealthSeverity, RelationKind, RelationProvenance, RelationResolution, TensionCategory,
    };

    #[test]
    fn derive_tags_adds_layer_tags_from_evidence_path() -> Result<(), Box<dyn std::error::Error>> {
        let ui_path = RepoPath::new("src/components/Button.tsx")?;
        let ui_evidence = EvidenceRef::file(ArtifactId::from_path(&ui_path), ui_path.clone());
        let readme_path = RepoPath::new("README.md")?;
        let readme_evidence =
            EvidenceRef::file(ArtifactId::from_path(&readme_path), readme_path.clone());

        let graph = Graph {
            nodes: vec![
                GraphNode::Artifact(ArtifactNode {
                    id: GraphNodeId::new("artifact:src/components/Button.tsx"),
                    path: ui_path.as_str().to_owned(),
                    category: ArtifactCategory::SourceCode,
                    evidence: ui_evidence.clone(),
                }),
                GraphNode::Symbol(SymbolNode {
                    id: GraphNodeId::new("symbol:src/components/Button.tsx#Button"),
                    kind: SymbolKind::Function,
                    qualified_name: "Button".to_owned(),
                    doc: None,
                    evidence: ui_evidence,
                }),
                GraphNode::Artifact(ArtifactNode {
                    id: GraphNodeId::new("artifact:README.md"),
                    path: readme_path.as_str().to_owned(),
                    category: ArtifactCategory::Documentation,
                    evidence: readme_evidence,
                }),
                GraphNode::Package(PackageNode {
                    id: GraphNodeId::new("package:left-pad"),
                    name: "left-pad".to_owned(),
                    is_external: true,
                }),
            ],
            relations: vec![],
        };

        let tags = derive_tags(&graph, "g1");
        let layer_of = |entity_id: &str| {
            tags.iter()
                .find(|tag| tag.entity_id == entity_id && tag.namespace == "layer")
                .map(|tag| tag.value.as_str())
        };

        assert_eq!(layer_of("artifact:src/components/Button.tsx"), Some("ui"));
        assert_eq!(
            layer_of("symbol:src/components/Button.tsx#Button"),
            Some("ui")
        );
        assert_eq!(layer_of("artifact:README.md"), Some("unknown"));
        assert_eq!(layer_of("package:left-pad"), None);
        assert!(
            tags.iter()
                .any(|tag| tag.namespace == "layer" && tag.source == TagSource::Architecture)
        );

        Ok(())
    }
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
    fn display_tags_keep_entity_provenance_and_real_snapshot()
    -> Result<(), Box<dyn std::error::Error>> {
        let path = RepoPath::new("src/lib.rs")?;
        let evidence = EvidenceRef::file(ArtifactId::from_path(&path), path);
        let relation = Relation {
            id: "edge:a-b".into(),
            source: GraphNodeId::new("symbol:a"),
            target: GraphNodeId::new("symbol:b"),
            kind: RelationKind::Calls,
            confidence: Confidence::Low,
            evidence: vec![evidence],
            provenance: Some(RelationProvenance {
                language: Some("rust".into()),
                resolver_strategy: "type-aware".into(),
                resolution: RelationResolution::HybridResolved,
                confidence: Confidence::High,
            }),
        };
        let cluster = ArchitectureCluster {
            id: "cluster:a".into(),
            members: vec![GraphNodeId::new("symbol:a")],
            top_nodes: vec![],
            packages: vec![],
            edge_types: vec![],
            cohesion: 0.0,
            incoming_pressure: 0,
            outgoing_pressure: 0,
            tags: vec![],
        };
        let tension = RepositoryTension {
            id: "risk:a".into(),
            category: TensionCategory::BlastRadius,
            severity: HealthSeverity::High,
            confidence: Confidence::Low,
            affected_nodes: vec![GraphNodeId::new("symbol:a")],
            affected_edges: vec![],
            metric_inputs: BTreeMap::new(),
            evidence_references: vec!["edge:a-b".into()],
            explanation: "risk".into(),
            follow_up_queries: vec![],
            tags: vec![],
        };

        let relation_tag = &relation_display_tags(&relation, "blake3:real")[0];
        assert_eq!(relation_tag.source, TagSource::DependencyRole);
        assert_eq!(relation_tag.confidence, Confidence::Low);
        assert_eq!(relation_tag.evidence, vec!["src/lib.rs"]);
        assert_eq!(relation_tag.graph_snapshot_id, "blake3:real");
        let resolution_tag = &relation_display_tags(&relation, "blake3:real")[1];
        assert_eq!(resolution_tag.namespace, "resolution");
        assert_eq!(resolution_tag.value, "type-aware");
        assert_eq!(resolution_tag.confidence, Confidence::High);
        assert_eq!(
            cluster_display_tags(&cluster, "blake3:real")[0].evidence,
            vec!["symbol:a"]
        );
        let tension_tag = &tension_display_tags(&tension, "blake3:real")[0];
        assert_eq!(tension_tag.source, TagSource::Tension);
        assert_eq!(tension_tag.confidence, Confidence::Low);
        assert_eq!(tension_tag.evidence, vec!["edge:a-b"]);
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
