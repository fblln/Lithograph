//! Conservative, deterministic graph code-health findings.

use crate::domain::Confidence;
use crate::graph::{
    Graph, GraphNodeId, RelationKind, analyze_communities, architecture_aware_scope,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Conservative thresholds for local graph-health detectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthThresholds {
    /// Minimum total degree before reporting a god-class candidate.
    pub god_class_degree: usize,
    /// Minimum incident edges before reporting a bridge bottleneck.
    pub bridge_degree: usize,
    /// Maximum cohesion considered weak.
    pub low_cohesion_percent: u8,
    /// Minimum co-change neighbors for shotgun-surgery risk.
    pub shotgun_neighbors: usize,
}
impl Default for HealthThresholds {
    fn default() -> Self {
        Self {
            god_class_degree: 12,
            bridge_degree: 8,
            low_cohesion_percent: 25,
            shotgun_neighbors: 5,
        }
    }
}

/// Stable finding category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthRule {
    /// Oversized high-degree class-like node.
    GodClass,
    /// High-degree bridge node.
    BridgeBottleneck,
    /// Directed two-node dependency cycle.
    Cycle,
    /// Node with no graph relations.
    OrphanedCode,
    /// Explicit co-change coupling.
    HiddenCoupling,
    /// Explicit service/layer-boundary crossing.
    LayerViolation,
    /// Community below the configured cohesion threshold.
    LowCohesionCluster,
    /// File coupled to many co-change neighbors.
    ShotgunSurgery,
}
/// Triage severity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HealthSeverity {
    /// Worth tracking during normal maintenance.
    Low,
    /// Needs an investigation when modifying the area.
    Medium,
    /// Likely architectural risk.
    High,
}
/// Explainable persisted code-health finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthFinding {
    /// Deterministic rule and affected-node identity.
    pub id: String,
    /// Detector that produced the result.
    pub rule: HealthRule,
    /// Conservative triage level.
    pub severity: HealthSeverity,
    /// Confidence in the graph evidence.
    pub confidence: Confidence,
    /// Affected graph nodes in stable order.
    pub affected_nodes: Vec<GraphNodeId>,
    /// Supporting relation ids or node ids.
    pub evidence: Vec<String>,
    /// Named detector values used for the decision.
    pub metric_inputs: BTreeMap<String, usize>,
    /// Local typed-query starting point for investigation.
    pub investigation_query: String,
}

/// Runs every offline detector with the supplied conservative thresholds.
pub fn detect_health(graph: &Graph, thresholds: &HealthThresholds) -> Vec<HealthFinding> {
    let mut degrees = BTreeMap::<GraphNodeId, usize>::new();
    let mut cochange = BTreeMap::<GraphNodeId, usize>::new();
    let mut findings = Vec::new();
    for node in &graph.nodes {
        degrees.insert(node.id().clone(), 0);
    }
    for edge in &graph.relations {
        *degrees.entry(edge.source.clone()).or_default() += 1;
        *degrees.entry(edge.target.clone()).or_default() += 1;
        if edge.kind == RelationKind::FileChangesWith {
            *cochange.entry(edge.source.clone()).or_default() += 1;
            *cochange.entry(edge.target.clone()).or_default() += 1;
        }
    }
    for (node, degree) in &degrees {
        if *degree == 0 {
            findings.push(finding(
                HealthRule::OrphanedCode,
                HealthSeverity::Low,
                vec![node.clone()],
                vec![node.to_string()],
                [("degree", *degree)],
            ));
        }
        if *degree >= thresholds.god_class_degree {
            findings.push(finding(
                HealthRule::GodClass,
                HealthSeverity::High,
                vec![node.clone()],
                vec![node.to_string()],
                [("degree", *degree)],
            ));
        }
        if *degree >= thresholds.bridge_degree {
            findings.push(finding(
                HealthRule::BridgeBottleneck,
                HealthSeverity::Medium,
                vec![node.clone()],
                vec![node.to_string()],
                [("degree", *degree)],
            ));
        }
    }
    for edge in &graph.relations {
        if edge.kind == RelationKind::FileChangesWith {
            findings.push(finding(
                HealthRule::HiddenCoupling,
                HealthSeverity::Medium,
                vec![edge.source.clone(), edge.target.clone()],
                vec![edge.id.clone()],
                [("cochange_edges", 1)],
            ));
        }
        if edge.kind == RelationKind::CrossesServiceBoundary {
            findings.push(finding(
                HealthRule::LayerViolation,
                HealthSeverity::Medium,
                vec![edge.source.clone(), edge.target.clone()],
                vec![edge.id.clone()],
                [("boundary_crossings", 1)],
            ));
        }
        if graph
            .relations
            .iter()
            .any(|other| other.source == edge.target && other.target == edge.source)
            && edge.source < edge.target
        {
            findings.push(finding(
                HealthRule::Cycle,
                HealthSeverity::Medium,
                vec![edge.source.clone(), edge.target.clone()],
                vec![edge.id.clone()],
                [("cycle_size", 2)],
            ));
        }
    }
    for (node, count) in cochange {
        if count >= thresholds.shotgun_neighbors {
            findings.push(finding(
                HealthRule::ShotgunSurgery,
                HealthSeverity::High,
                vec![node.clone()],
                vec![node.to_string()],
                [("cochange_neighbors", count)],
            ));
        }
    }
    let communities = analyze_communities(graph, &architecture_aware_scope(), None)
        .map(|analysis| analysis.communities)
        .unwrap_or_default();
    for community in communities {
        let cohesion = (community.cohesion * 100.0) as usize;
        if cohesion <= thresholds.low_cohesion_percent as usize {
            findings.push(finding(
                HealthRule::LowCohesionCluster,
                HealthSeverity::Low,
                community.members,
                community.boundary_edges,
                [("cohesion_percent", cohesion)],
            ));
        }
    }
    findings.sort_by(|a, b| a.rule.cmp(&b.rule).then(a.id.cmp(&b.id)));
    findings.dedup_by(|a, b| a.id == b.id);
    findings
}

fn finding<const N: usize>(
    rule: HealthRule,
    severity: HealthSeverity,
    mut nodes: Vec<GraphNodeId>,
    evidence: Vec<String>,
    inputs: [(&str, usize); N],
) -> HealthFinding {
    nodes.sort();
    nodes.dedup();
    let id = format!(
        "{:?}:{}",
        rule,
        nodes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    HealthFinding {
        id,
        rule,
        severity,
        confidence: Confidence::High,
        affected_nodes: nodes,
        evidence,
        metric_inputs: inputs.into_iter().map(|(k, v)| (k.into(), v)).collect(),
        investigation_query: "MATCH (n)-[r]-(m) RETURN n,r,m".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Confidence;
    use crate::graph::{GraphNode, Relation, UnresolvedNode};
    fn edge(id: &str, source: &str, target: &str, kind: RelationKind) -> Relation {
        Relation {
            id: id.into(),
            source: GraphNodeId::new(source),
            target: GraphNodeId::new(target),
            kind,
            confidence: Confidence::High,
            evidence: vec![],
            provenance: None,
        }
    }
    #[test]
    fn detectors_are_deterministic_and_cover_required_patterns() {
        let graph = Graph {
            nodes: vec![GraphNode::Unresolved(UnresolvedNode {
                id: GraphNodeId::new("orphan"),
                value: "orphan".to_owned(),
            })],
            relations: vec![
                edge("ab", "a", "b", RelationKind::Calls),
                edge("ba", "b", "a", RelationKind::Calls),
                edge("xy", "x", "y", RelationKind::FileChangesWith),
                edge("layer", "api", "data", RelationKind::CrossesServiceBoundary),
            ],
        };
        let thresholds = HealthThresholds {
            god_class_degree: 2,
            bridge_degree: 2,
            low_cohesion_percent: 100,
            shotgun_neighbors: 1,
        };
        let findings = detect_health(&graph, &thresholds);
        for rule in [
            HealthRule::GodClass,
            HealthRule::BridgeBottleneck,
            HealthRule::Cycle,
            HealthRule::OrphanedCode,
            HealthRule::HiddenCoupling,
            HealthRule::LayerViolation,
            HealthRule::LowCohesionCluster,
            HealthRule::ShotgunSurgery,
        ] {
            assert!(findings.iter().any(|finding| finding.rule == rule));
        }
        assert_eq!(findings, detect_health(&graph, &thresholds));
    }
}
