//! LadybugDB schema contract for Lithograph's durable graph projection.
//!
//! The Rust [`Graph`](crate::graph::Graph) remains canonical: this module
//! describes its lossless, query-oriented Ladybug projection without exposing
//! untyped Cypher strings throughout the rest of the application. The adapter
//! introduced by LIT-24.2 will execute these statements transactionally.

/// First LadybugDB schema version owned by Lithograph.
pub const LADYBUG_SCHEMA_VERSION: u32 = 1;

/// Version of deterministic metric and classification algorithms whose output
/// is stored in the analytics tables. Bump it when an algorithm's meaning,
/// rather than only its implementation, changes.
pub const LADYBUG_ALGORITHM_VERSION: u32 = 1;

/// A Ladybug table owned by the Lithograph projection schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LadybugTable {
    /// One immutable graph snapshot identity and compatibility metadata.
    Snapshot,
    /// One canonical [`GraphNode`](crate::graph::GraphNode) projection.
    CodeNode,
    /// One graph relation projection, stored as a Ladybug relationship row.
    GraphRelation,
    /// The metrics computation attached to one graph snapshot.
    MetricSnapshot,
    /// One scalar metric attached to one code node.
    NodeMetric,
    /// One deterministic community computed for a snapshot.
    Community,
    /// One semantic class/profile attached to a code node.
    SemanticProfile,
    /// One code-health finding attached to a code node or relation.
    HealthFinding,
    /// Applied schema/model/algorithm migration ledger entry.
    SchemaMigration,
    /// Snapshot-to-code-node ownership relationship.
    SnapshotOwnsNode,
    /// Metric-snapshot-to-node-metric relationship.
    MetricSnapshotHasMetric,
    /// Node-metric-to-code-node relationship.
    MetricMeasuresNode,
    /// Community-to-code-node membership relationship.
    CommunityMember,
    /// Semantic-profile-to-code-node relationship.
    ProfileClassifiesNode,
    /// Health-finding-to-code-node target relationship.
    FindingTargetsNode,
}

impl LadybugTable {
    /// Stable Ladybug table name. These names are part of the persisted
    /// contract and must only change through an explicit schema migration.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Snapshot => "Snapshot",
            Self::CodeNode => "CodeNode",
            Self::GraphRelation => "GraphRelation",
            Self::MetricSnapshot => "MetricSnapshot",
            Self::NodeMetric => "NodeMetric",
            Self::Community => "Community",
            Self::SemanticProfile => "SemanticProfile",
            Self::HealthFinding => "HealthFinding",
            Self::SchemaMigration => "SchemaMigration",
            Self::SnapshotOwnsNode => "SNAPSHOT_OWNS_NODE",
            Self::MetricSnapshotHasMetric => "METRIC_SNAPSHOT_HAS_METRIC",
            Self::MetricMeasuresNode => "METRIC_MEASURES_NODE",
            Self::CommunityMember => "COMMUNITY_MEMBER",
            Self::ProfileClassifiesNode => "PROFILE_CLASSIFIES_NODE",
            Self::FindingTargetsNode => "FINDING_TARGETS_NODE",
        }
    }
}

/// Every table required by schema version one, in migration order: nodes
/// first, followed by relationship tables whose endpoint labels now exist.
pub const LADYBUG_TABLES_V1: &[LadybugTable] = &[
    LadybugTable::Snapshot,
    LadybugTable::CodeNode,
    LadybugTable::MetricSnapshot,
    LadybugTable::NodeMetric,
    LadybugTable::Community,
    LadybugTable::SemanticProfile,
    LadybugTable::HealthFinding,
    LadybugTable::SchemaMigration,
    LadybugTable::GraphRelation,
    LadybugTable::SnapshotOwnsNode,
    LadybugTable::MetricSnapshotHasMetric,
    LadybugTable::MetricMeasuresNode,
    LadybugTable::CommunityMember,
    LadybugTable::ProfileClassifiesNode,
    LadybugTable::FindingTargetsNode,
];

/// Returns the schema DDL in the only valid application order.
///
/// Column types and primary keys follow Ladybug's structured property-graph
/// requirements. JSON payloads are serialized strings intentionally: the
/// Rust types retain semantic ownership, while these optional full-fidelity
/// payloads make new typed fields forward-compatible without weakening the
/// indexed query columns.
pub fn creation_statements() -> &'static [&'static str] {
    &[
        "CREATE NODE TABLE Snapshot(snapshot_id STRING PRIMARY KEY, schema_version INT64, graph_model_version INT64, algorithm_version INT64, repository_hash STRING, created_at STRING, node_count INT64, relation_count INT64, metadata_json STRING)",
        "CREATE NODE TABLE CodeNode(node_key STRING PRIMARY KEY, snapshot_id STRING, graph_node_id STRING, node_label STRING, node_kind STRING, display_name STRING, artifact_path STRING, language STRING, is_external BOOL, is_dynamic BOOL, evidence_path STRING, evidence_start_line INT64, evidence_end_line INT64, payload_json STRING)",
        "CREATE NODE TABLE MetricSnapshot(metric_snapshot_id STRING PRIMARY KEY, snapshot_id STRING, algorithm_version INT64, metric_set_hash STRING, created_at STRING, parameters_json STRING)",
        "CREATE NODE TABLE NodeMetric(metric_key STRING PRIMARY KEY, metric_snapshot_id STRING, metric_name STRING, metric_value DOUBLE, rank INT64, payload_json STRING)",
        "CREATE NODE TABLE Community(community_key STRING PRIMARY KEY, snapshot_id STRING, algorithm_version INT64, community_id STRING, cohesion DOUBLE, member_count INT64, payload_json STRING)",
        "CREATE NODE TABLE SemanticProfile(profile_key STRING PRIMARY KEY, snapshot_id STRING, algorithm_version INT64, profile_kind STRING, confidence STRING, payload_json STRING)",
        "CREATE NODE TABLE HealthFinding(finding_key STRING PRIMARY KEY, snapshot_id STRING, algorithm_version INT64, rule_id STRING, severity STRING, status STRING, evidence_json STRING, payload_json STRING)",
        "CREATE NODE TABLE SchemaMigration(migration_id STRING PRIMARY KEY, schema_version INT64, graph_model_version INT64, algorithm_version INT64, applied_at STRING, checksum STRING, description STRING)",
        "CREATE REL TABLE GraphRelation(FROM CodeNode TO CodeNode, relation_key STRING, snapshot_id STRING, relation_id STRING, relation_kind STRING, confidence STRING, resolution STRING, resolver_strategy STRING, provenance_language STRING, evidence_json STRING, payload_json STRING)",
        "CREATE REL TABLE SNAPSHOT_OWNS_NODE(FROM Snapshot TO CodeNode, snapshot_id STRING)",
        "CREATE REL TABLE METRIC_SNAPSHOT_HAS_METRIC(FROM MetricSnapshot TO NodeMetric, metric_snapshot_id STRING)",
        "CREATE REL TABLE METRIC_MEASURES_NODE(FROM NodeMetric TO CodeNode, metric_key STRING)",
        "CREATE REL TABLE COMMUNITY_MEMBER(FROM Community TO CodeNode, community_key STRING, role STRING)",
        "CREATE REL TABLE PROFILE_CLASSIFIES_NODE(FROM SemanticProfile TO CodeNode, profile_key STRING)",
        "CREATE REL TABLE FINDING_TARGETS_NODE(FROM HealthFinding TO CodeNode, finding_key STRING, target_role STRING)",
    ]
}

/// Stable migration identifier used in the `SchemaMigration` ledger.
pub fn migration_id(from_schema_version: u32, to_schema_version: u32) -> String {
    format!("ladybug-schema:{from_schema_version}->{to_schema_version}")
}

#[cfg(test)]
mod tests {
    use super::{
        LADYBUG_SCHEMA_VERSION, LADYBUG_TABLES_V1, LadybugTable, creation_statements, migration_id,
    };
    use std::collections::BTreeSet;

    #[test]
    fn version_one_schema_has_unique_stable_table_names() {
        let names: BTreeSet<&str> = LADYBUG_TABLES_V1.iter().map(|table| table.name()).collect();
        assert_eq!(names.len(), LADYBUG_TABLES_V1.len());
        assert!(names.contains(LadybugTable::GraphRelation.name()));
        assert!(names.contains(LadybugTable::HealthFinding.name()));
    }

    #[test]
    fn every_declared_table_has_a_creation_statement() {
        let statements = creation_statements();
        assert_eq!(statements.len(), LADYBUG_TABLES_V1.len());
        for table in LADYBUG_TABLES_V1 {
            assert!(
                statements
                    .iter()
                    .any(|statement| statement.contains(table.name())),
                "missing DDL for {}",
                table.name()
            );
        }
        assert!(
            statements
                .iter()
                .any(|statement| statement.contains("payload_json STRING"))
        );
        assert!(
            statements
                .iter()
                .any(|statement| statement.contains("evidence_json STRING"))
        );
    }

    #[test]
    fn migration_ids_are_deterministic_and_versioned() {
        assert_eq!(LADYBUG_SCHEMA_VERSION, 1);
        assert_eq!(migration_id(1, 2), "ladybug-schema:1->2");
    }
}
