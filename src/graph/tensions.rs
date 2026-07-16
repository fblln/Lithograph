//! Versioned, typed repository-tension scoring built from local analytics.

use crate::domain::Confidence;
use crate::graph::{
    Graph, GraphNodeId, HealthRule, HealthSeverity, HealthThresholds, detect_health,
};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Versioned semantics for repository-tension scoring.
pub const TENSION_ALGORITHM_VERSION: u32 = 1;
/// First-class tension categories shared by non-UI and UI consumers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[allow(missing_docs)]
pub enum TensionCategory {
    CouplingHotspot,
    DependencyCycle,
    BridgeBottleneck,
    BoundaryViolation,
    BlastRadius,
    LowCohesion,
    DeadCode,
    DriftRisk,
    ChangeConcentration,
}
/// Typed explainable repository tension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct RepositoryTension {
    pub id: String,
    pub category: TensionCategory,
    pub severity: HealthSeverity,
    pub confidence: Confidence,
    pub affected_nodes: Vec<GraphNodeId>,
    pub affected_edges: Vec<String>,
    pub metric_inputs: BTreeMap<String, usize>,
    pub evidence_references: Vec<String>,
    pub explanation: String,
    pub follow_up_queries: Vec<String>,
    /// Snapshot-bound display tags added by API surfaces. Scoring itself is
    /// independent of persistence and therefore leaves these empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<crate::graph::GraphTag>,
}
/// A graph-snapshot-bound tension result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(missing_docs)]
pub struct TensionSnapshot {
    pub graph_snapshot_id: String,
    pub algorithm_version: u32,
    pub tensions: Vec<RepositoryTension>,
}
/// JSON persistence for typed tension snapshots.
#[derive(Debug, Clone)]
#[allow(missing_docs)]
pub struct TensionSnapshotStore {
    root: std::path::PathBuf,
}
#[allow(missing_docs)]
impl TensionSnapshotStore {
    pub fn new(root: impl Into<std::path::PathBuf>) -> Self {
        Self { root: root.into() }
    }
    pub fn save(&self, snapshot: &TensionSnapshot) -> std::io::Result<bool> {
        let payload = serde_json::to_string(snapshot).map_err(std::io::Error::other)?;
        let path = self.path(snapshot);
        if JsonStore.read::<String>(&path)?.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        JsonStore.write(&path, &payload)?;
        Ok(true)
    }
    pub fn load(&self, snapshot: &TensionSnapshot) -> std::io::Result<Option<TensionSnapshot>> {
        let Some(payload): Option<String> = JsonStore.read(&self.path(snapshot))? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(std::io::Error::other)
    }
    fn path(&self, snapshot: &TensionSnapshot) -> std::path::PathBuf {
        self.root.join(format!(
            "{}.json",
            blake3::hash(
                format!(
                    "{}:{}",
                    snapshot.graph_snapshot_id, snapshot.algorithm_version
                )
                .as_bytes()
            )
            .to_hex()
        ))
    }
}
/// Scores health, graph-impact, and supplied drift evidence without UI recomputation.
pub fn score_tensions(
    graph: &Graph,
    thresholds: &HealthThresholds,
    drift_evidence: &[String],
) -> Vec<RepositoryTension> {
    let mut tensions: Vec<_> = detect_health(graph, thresholds)
        .into_iter()
        .flat_map(from_health)
        .collect();
    if !drift_evidence.is_empty() {
        tensions.push(tension(
            TensionCategory::DriftRisk,
            HealthSeverity::Medium,
            vec![],
            drift_evidence.to_vec(),
            BTreeMap::from([("drift_findings".into(), drift_evidence.len())]),
        ));
    }
    tensions.sort_by(|a, b| a.category.cmp(&b.category).then(a.id.cmp(&b.id)));
    tensions.dedup_by(|a, b| a.id == b.id);
    tensions
}
fn from_health(finding: crate::graph::HealthFinding) -> Vec<RepositoryTension> {
    let category = match finding.rule {
        HealthRule::GodClass => TensionCategory::CouplingHotspot,
        HealthRule::Cycle => TensionCategory::DependencyCycle,
        HealthRule::BridgeBottleneck => TensionCategory::BridgeBottleneck,
        HealthRule::LayerViolation => TensionCategory::BoundaryViolation,
        HealthRule::LowCohesionCluster => TensionCategory::LowCohesion,
        HealthRule::OrphanedCode => TensionCategory::DeadCode,
        HealthRule::HiddenCoupling | HealthRule::ShotgunSurgery => {
            TensionCategory::ChangeConcentration
        }
    };
    let mut values = vec![tension(
        category,
        finding.severity,
        finding.affected_nodes.clone(),
        finding.evidence.clone(),
        finding.metric_inputs.clone(),
    )];
    if finding.rule == HealthRule::BridgeBottleneck {
        values.push(tension(
            TensionCategory::BlastRadius,
            finding.severity,
            finding.affected_nodes,
            finding.evidence,
            finding.metric_inputs,
        ));
    }
    values
}
fn tension(
    category: TensionCategory,
    severity: HealthSeverity,
    mut nodes: Vec<GraphNodeId>,
    mut evidence: Vec<String>,
    inputs: BTreeMap<String, usize>,
) -> RepositoryTension {
    nodes.sort();
    evidence.sort();
    let id = format!(
        "{:?}:{}",
        category,
        nodes
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
    );
    RepositoryTension {
        id,
        category,
        severity,
        confidence: Confidence::High,
        affected_nodes: nodes,
        affected_edges: evidence.clone(),
        metric_inputs: inputs,
        evidence_references: evidence,
        explanation: format!("{:?} detected from deterministic graph evidence", category),
        follow_up_queries: vec!["MATCH (n)-[r]-(m) RETURN n,r,m".into()],
        tags: vec![],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Confidence;
    use crate::graph::{Relation, RelationKind};
    fn edge(a: &str, b: &str, k: RelationKind) -> Relation {
        Relation {
            id: format!("{a}-{b}"),
            source: GraphNodeId::new(a),
            target: GraphNodeId::new(b),
            kind: k,
            confidence: Confidence::High,
            evidence: vec![],
            provenance: None,
        }
    }
    #[test]
    fn scores_order_evidence_and_empty_cases() {
        let empty = Graph {
            nodes: vec![],
            relations: vec![],
        };
        assert!(score_tensions(&empty, &HealthThresholds::default(), &[]).is_empty());
        let graph = Graph {
            nodes: vec![],
            relations: vec![
                edge("a", "b", RelationKind::Calls),
                edge("b", "a", RelationKind::Calls),
            ],
        };
        let t = score_tensions(
            &graph,
            &HealthThresholds {
                god_class_degree: 2,
                bridge_degree: 2,
                low_cohesion_percent: 0,
                shotgun_neighbors: 9,
            },
            &["docs/a.md".into()],
        );
        assert!(
            t.iter()
                .any(|x| x.category == TensionCategory::DependencyCycle)
        );
        assert!(t.iter().any(|x| x.category == TensionCategory::DriftRisk));
        assert_eq!(
            t,
            score_tensions(
                &graph,
                &HealthThresholds {
                    god_class_degree: 2,
                    bridge_degree: 2,
                    low_cohesion_percent: 0,
                    shotgun_neighbors: 9
                },
                &["docs/a.md".into()]
            )
        );
    }
    #[test]
    fn snapshots_round_trip_without_rewriting() -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = TensionSnapshot {
            graph_snapshot_id: "g1".into(),
            algorithm_version: TENSION_ALGORITHM_VERSION,
            tensions: vec![],
        };
        let temp = tempfile::TempDir::new()?;
        let store = TensionSnapshotStore::new(temp.path());
        assert!(store.save(&snapshot)?);
        assert!(!store.save(&snapshot)?);
        assert_eq!(store.load(&snapshot)?, Some(snapshot));
        Ok(())
    }
}
