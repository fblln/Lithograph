//! LadybugDB-backed persistence for typed Lithograph graph snapshots.

use crate::graph::{
    GraphNode, GraphSnapshot, LADYBUG_ALGORITHM_VERSION, LADYBUG_SCHEMA_VERSION,
    ladybug_creation_statements,
};
use lbug::{Connection, Database, SystemConfig, Value};
use std::io;
use std::path::{Path, PathBuf};

/// Ladybug database projection for one repository graph store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LadybugGraphStore {
    path: PathBuf,
}

impl LadybugGraphStore {
    /// Opens the projection rooted at `path`; the database is created lazily
    /// during the first successful write.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Path to the embedded Ladybug database directory.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a snapshot transactionally. Returns `false` when the currently
    /// stored canonical snapshot is byte-for-byte identical, avoiding a
    /// needless delete/reprojection cycle on no-op updates.
    pub fn save(&self, snapshot: &GraphSnapshot) -> io::Result<bool> {
        let payload = serde_json::to_string(snapshot).map_err(invalid_json)?;
        if self.load_payload()?.as_deref() == Some(payload.as_str()) {
            return Ok(false);
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let database = Database::new(&self.path, SystemConfig::default()).map_err(lbug_error)?;
        let connection = Connection::new(&database).map_err(lbug_error)?;
        ensure_schema(&connection)?;

        // Ladybug permits one writer at a time by default. One connection and
        // one explicit transaction give the projection all-or-nothing update
        // semantics and avoid accidentally interleaving relationship rows
        // from separate graph snapshots.
        query(&connection, "BEGIN TRANSACTION")?;
        let result = (|| {
            query(&connection, "MATCH (node:CodeNode) DETACH DELETE node")?;
            query(
                &connection,
                "MATCH (snapshot:Snapshot) DETACH DELETE snapshot",
            )?;
            write_snapshot(&connection, snapshot, &payload)?;
            for node in &snapshot.graph.nodes {
                write_node(&connection, snapshot, node)?;
            }
            for relation in &snapshot.graph.relations {
                write_relation(&connection, snapshot, relation)?;
            }
            Ok(())
        })();
        match result {
            Ok(()) => query(&connection, "COMMIT")?,
            Err(error) => {
                let _ = query(&connection, "ROLLBACK");
                return Err(error);
            }
        }
        Ok(true)
    }

    /// Reads the canonical snapshot from a distinct read-only Ladybug handle.
    /// Returning `None` means no Ladybug database/snapshot exists yet, so the
    /// caller may use the legacy JSON compatibility path.
    pub fn load(&self) -> io::Result<Option<GraphSnapshot>> {
        let Some(payload) = self.load_payload()? else {
            return Ok(None);
        };
        serde_json::from_str(&payload)
            .map(Some)
            .map_err(invalid_json)
    }

    fn load_payload(&self) -> io::Result<Option<String>> {
        if !self.path.exists() {
            return Ok(None);
        }
        let database = Database::new(&self.path, SystemConfig::default().read_only(true))
            .map_err(lbug_error)?;
        let connection = Connection::new(&database).map_err(lbug_error)?;
        let mut rows = query_result(
            &connection,
            "MATCH (snapshot:Snapshot) RETURN snapshot.metadata_json LIMIT 1",
        )?;
        match rows.next() {
            Some(values) if values.len() == 1 => match &values[0] {
                Value::String(payload) => Ok(Some(payload.clone())),
                value => Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Ladybug Snapshot.metadata_json has unexpected value {value:?}"),
                )),
            },
            Some(_) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Ladybug snapshot query returned an unexpected column count",
            )),
            None => Ok(None),
        }
    }
}

fn ensure_schema(connection: &Connection<'_>) -> io::Result<()> {
    for statement in ladybug_creation_statements() {
        match connection.query(statement) {
            Ok(_) => {}
            // Ladybug reports an error when a table exists; all other schema
            // errors (including a corrupted/incompatible database) are real
            // failures and must not be silently treated as initialization.
            Err(error)
                if error
                    .to_string()
                    .to_ascii_lowercase()
                    .contains("already exists") => {}
            Err(error) => return Err(lbug_error(error)),
        }
    }
    Ok(())
}

fn write_snapshot(
    connection: &Connection<'_>,
    snapshot: &GraphSnapshot,
    payload: &str,
) -> io::Result<()> {
    let id = snapshot_id(payload);
    query(
        connection,
        &format!(
            "CREATE (:Snapshot {{snapshot_id: {}, schema_version: {}, graph_model_version: {}, algorithm_version: {}, repository_hash: {}, created_at: \"\", node_count: {}, relation_count: {}, metadata_json: {}}})",
            cypher_string(&id),
            LADYBUG_SCHEMA_VERSION,
            snapshot.metadata.graph_model_version,
            LADYBUG_ALGORITHM_VERSION,
            cypher_string(blake3::hash(payload.as_bytes()).to_hex().as_ref()),
            snapshot.graph.nodes.len(),
            snapshot.graph.relations.len(),
            cypher_string(payload),
        ),
    )
}

fn write_node(
    connection: &Connection<'_>,
    snapshot: &GraphSnapshot,
    node: &GraphNode,
) -> io::Result<()> {
    let payload = serde_json::to_string(node).map_err(invalid_json)?;
    let snapshot_id = snapshot_id(&serde_json::to_string(snapshot).map_err(invalid_json)?);
    let node_id = node.id().as_str();
    query(
        connection,
        &format!(
            "CREATE (:CodeNode {{node_key: {}, snapshot_id: {}, graph_node_id: {}, node_label: {}, node_kind: \"\", display_name: {}, artifact_path: \"\", language: \"\", is_external: false, is_dynamic: false, evidence_path: \"\", evidence_start_line: 0, evidence_end_line: 0, payload_json: {}}})",
            cypher_string(&node_key(&snapshot_id, node_id)),
            cypher_string(&snapshot_id),
            cypher_string(node_id),
            cypher_string(node_label(node)),
            cypher_string(node_id),
            cypher_string(&payload),
        ),
    )
}

fn write_relation(
    connection: &Connection<'_>,
    snapshot: &GraphSnapshot,
    relation: &crate::graph::Relation,
) -> io::Result<()> {
    let snapshot_id = snapshot_id(&serde_json::to_string(snapshot).map_err(invalid_json)?);
    let payload = serde_json::to_string(relation).map_err(invalid_json)?;
    let relation_key = format!("{}:{}", snapshot_id, relation.id);
    let resolution = relation
        .provenance
        .as_ref()
        .map_or_else(String::new, |value| format!("{:?}", value.resolution));
    query(
        connection,
        &format!(
            "MATCH (source:CodeNode {{node_key: {}}}), (target:CodeNode {{node_key: {}}}) CREATE (source)-[:GraphRelation {{relation_key: {}, snapshot_id: {}, relation_id: {}, relation_kind: {}, confidence: {}, resolution: {}, resolver_strategy: {}, provenance_language: {}, evidence_json: {}, payload_json: {}}}]->(target)",
            cypher_string(&node_key(&snapshot_id, relation.source.as_str())),
            cypher_string(&node_key(&snapshot_id, relation.target.as_str())),
            cypher_string(&relation_key),
            cypher_string(&snapshot_id),
            cypher_string(&relation.id),
            cypher_string(&format!("{:?}", relation.kind)),
            cypher_string(&format!("{:?}", relation.confidence)),
            cypher_string(&resolution),
            cypher_string(
                relation
                    .provenance
                    .as_ref()
                    .map_or("", |value| value.resolver_strategy.as_str())
            ),
            cypher_string(
                relation
                    .provenance
                    .as_ref()
                    .and_then(|value| value.language.as_deref())
                    .unwrap_or("")
            ),
            cypher_string(&serde_json::to_string(&relation.evidence).map_err(invalid_json)?),
            cypher_string(&payload),
        ),
    )
}

fn query(connection: &Connection<'_>, statement: &str) -> io::Result<()> {
    connection.query(statement).map(|_| ()).map_err(lbug_error)
}

fn query_result<'a>(
    connection: &Connection<'a>,
    statement: &str,
) -> io::Result<lbug::QueryResult<'a>> {
    connection.query(statement).map_err(lbug_error)
}

fn cypher_string(value: &str) -> String {
    serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_owned())
}

fn snapshot_id(payload: &str) -> String {
    format!("snapshot:{}", blake3::hash(payload.as_bytes()).to_hex())
}

fn node_key(snapshot_id: &str, node_id: &str) -> String {
    format!("{snapshot_id}::{node_id}")
}

fn node_label(node: &GraphNode) -> &'static str {
    match node {
        GraphNode::Artifact(_) => "Artifact",
        GraphNode::Symbol(_) => "Symbol",
        GraphNode::Config(_) => "Config",
        GraphNode::Documentation(_) => "Documentation",
        GraphNode::Container(_) => "Container",
        GraphNode::Command(_) => "Command",
        GraphNode::EnvVar(_) => "EnvVar",
        GraphNode::Module(_) => "Module",
        GraphNode::Package(_) => "Package",
        GraphNode::Unresolved(_) => "Unresolved",
    }
}

fn lbug_error(error: lbug::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, format!("LadybugDB: {error}"))
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::LadybugGraphStore;
    use crate::domain::{ArtifactId, Confidence, EvidenceRef, RepoPath};
    use crate::graph::{
        Graph, GraphNode, GraphNodeId, GraphSnapshot, Relation, RelationKind, RelationProvenance,
        RelationResolution, UnresolvedNode,
    };

    #[test]
    fn writes_reads_and_skips_an_identical_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = LadybugGraphStore::new(temp.path().join("graph.lbug"));
        let snapshot = GraphSnapshot::current(Graph::default());

        assert!(store.save(&snapshot)?);
        assert_eq!(store.load()?, Some(snapshot.clone()));
        assert!(!store.save(&snapshot)?);
        Ok(())
    }

    #[test]
    fn replaces_the_projection_without_losing_typed_nodes() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        let store = LadybugGraphStore::new(temp.path().join("graph.lbug"));
        let initial = GraphSnapshot::current(Graph::default());
        assert!(store.save(&initial)?);

        let source = GraphNodeId::new("unresolved:source");
        let target = GraphNodeId::new("unresolved:missing");
        let path = RepoPath::new("src/app.ts")?;
        let updated = GraphSnapshot::current(Graph {
            nodes: vec![
                GraphNode::Unresolved(UnresolvedNode {
                    id: source.clone(),
                    value: "source".to_owned(),
                }),
                GraphNode::Unresolved(UnresolvedNode {
                    id: target.clone(),
                    value: "missing".to_owned(),
                }),
            ],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source,
                target,
                kind: RelationKind::References,
                confidence: Confidence::High,
                evidence: vec![EvidenceRef::file(ArtifactId::from_path(&path), path)],
                provenance: Some(RelationProvenance {
                    language: Some("typescript".to_owned()),
                    resolver_strategy: "test".to_owned(),
                    resolution: RelationResolution::HybridResolved,
                    confidence: Confidence::High,
                }),
            }],
        });
        assert!(store.save(&updated)?);
        assert_eq!(store.load()?, Some(updated));
        Ok(())
    }

    #[test]
    fn corrupted_database_is_not_silently_treated_as_empty()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path = temp.path().join("corrupt.lbug");
        std::fs::write(&path, "not a Ladybug database")?;
        let error = LadybugGraphStore::new(path)
            .load()
            .err()
            .ok_or("corrupt Ladybug data must fail explicitly")?;
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
        Ok(())
    }
}
