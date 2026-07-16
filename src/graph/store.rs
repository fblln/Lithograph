//! Versioned persistent graph snapshots.

use crate::graph::{Graph, LadybugGraphStore};
use crate::storage::JsonStore;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::{Deserialize, Serialize};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

/// Current on-disk graph snapshot schema version.
pub const GRAPH_STORE_SCHEMA_VERSION: u32 = 1;

/// Current graph model version. This is separate from the store wrapper
/// version so future graph shape changes can invalidate or migrate snapshots
/// independently from the snapshot envelope.
///
/// 3: `SymbolKind::External` (LIT-56). Calls/Decorates/UsesType into an
/// imported standard-library or declared-dependency name now land on an
/// external symbol node instead of the package node, which the graph's own
/// target-kind rule rejects for those relation kinds.
pub const GRAPH_MODEL_VERSION: u32 = 3;

/// Current portable graph artifact envelope version.
pub const GRAPH_ARTIFACT_FORMAT_VERSION: u32 = 1;

/// Versioned graph snapshot envelope persisted under `.lithograph/graph/`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphSnapshot {
    /// Snapshot metadata.
    pub metadata: GraphStoreMetadata,
    /// Typed semantic graph.
    pub graph: Graph,
}

/// Metadata carried by every persisted graph snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphStoreMetadata {
    /// Version of the snapshot envelope.
    pub schema_version: u32,
    /// Version of the graph model inside the envelope.
    pub graph_model_version: u32,
    /// Stable graph node count at save time.
    pub node_count: usize,
    /// Stable graph relation count at save time.
    pub relation_count: usize,
    /// Names of migrations applied while loading this snapshot.
    pub migrations_applied: Vec<String>,
}

/// Metadata for a compressed team-shareable graph artifact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphArtifactMetadata {
    /// Portable artifact envelope version.
    pub artifact_format_version: u32,
    /// Compression algorithm used for the artifact file.
    pub compression: String,
    /// Checksum algorithm used for the canonical snapshot payload.
    pub checksum_algorithm: String,
    /// BLAKE3 checksum of the canonical snapshot JSON bytes.
    pub snapshot_checksum: String,
    /// Snapshot schema version in the payload.
    pub schema_version: u32,
    /// Graph model version in the payload.
    pub graph_model_version: u32,
    /// Graph node count recorded in the payload.
    pub node_count: usize,
    /// Graph relation count recorded in the payload.
    pub relation_count: usize,
}

/// Summary of a graph artifact import/export operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphArtifactReport {
    /// Artifact path read or written.
    pub artifact_path: PathBuf,
    /// Graph snapshot path read or written.
    pub snapshot_path: PathBuf,
    /// Legacy graph export path read or written.
    pub legacy_graph_path: PathBuf,
    /// Artifact metadata.
    pub metadata: GraphArtifactMetadata,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct GraphArtifact {
    metadata: GraphArtifactMetadata,
    snapshot: GraphSnapshot,
}

impl GraphSnapshot {
    /// Creates a current-version snapshot from a graph.
    pub fn current(graph: Graph) -> Self {
        Self {
            metadata: GraphStoreMetadata {
                schema_version: GRAPH_STORE_SCHEMA_VERSION,
                graph_model_version: GRAPH_MODEL_VERSION,
                node_count: graph.nodes.len(),
                relation_count: graph.relations.len(),
                migrations_applied: Vec::new(),
            },
            graph,
        }
    }
}

/// Filesystem-backed graph store for one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphStore {
    repo_root: PathBuf,
}

impl GraphStore {
    /// Creates a graph store rooted at one repository.
    pub fn new(repo_root: &Path) -> Self {
        Self {
            repo_root: repo_root.to_path_buf(),
        }
    }

    /// Path to the legacy graph export retained for compatibility.
    pub fn legacy_graph_path(&self) -> PathBuf {
        self.repo_root.join(".lithograph/graph.json")
    }

    /// Path to the current versioned graph snapshot.
    pub fn snapshot_path(&self) -> PathBuf {
        self.repo_root.join(".lithograph/graph/current.json")
    }

    /// Path to the primary embedded LadybugDB graph projection.
    pub fn ladybug_path(&self) -> PathBuf {
        self.repo_root.join(".lithograph/graph/ladybug.lbug")
    }

    /// Saves a current-version graph snapshot and the legacy graph export.
    pub fn save(&self, graph: &Graph) -> io::Result<GraphStoreWriteOutcome> {
        let snapshot = GraphSnapshot::current(graph.clone());
        let ladybug_written = LadybugGraphStore::new(self.ladybug_path()).save(&snapshot)?;
        let snapshot_written = JsonStore.write_if_changed(&self.snapshot_path(), &snapshot)?;
        let legacy_written = JsonStore.write_if_changed(&self.legacy_graph_path(), graph)?;
        Ok(GraphStoreWriteOutcome {
            snapshot_path: self.snapshot_path(),
            ladybug_path: self.ladybug_path(),
            legacy_graph_path: self.legacy_graph_path(),
            ladybug_written,
            snapshot_written,
            legacy_written,
        })
    }

    /// Loads the versioned graph snapshot, falling back to the legacy graph
    /// export when a repository was generated before the store envelope
    /// existed.
    pub fn load(&self) -> io::Result<GraphSnapshot> {
        let ladybug_store = LadybugGraphStore::new(self.ladybug_path());
        if ladybug_store.path().exists()
            && let Some(snapshot) = ladybug_store.load()?
        {
            return migrate(snapshot);
        }
        if let Some(snapshot) = JsonStore.read::<GraphSnapshot>(&self.snapshot_path())? {
            return migrate(snapshot);
        }
        let graph = JsonStore
            .read::<Graph>(&self.legacy_graph_path())?
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "no graph snapshot found at {} or {}",
                        self.snapshot_path().display(),
                        self.legacy_graph_path().display()
                    ),
                )
            })?;
        Ok(GraphSnapshot::current(graph))
    }

    /// Exports the current graph snapshot as a gzip-compressed artifact with
    /// embedded schema metadata and a checksum over canonical snapshot JSON.
    pub fn export_artifact(&self, artifact_path: &Path) -> io::Result<GraphArtifactReport> {
        let snapshot = self.load()?;
        let artifact = GraphArtifact {
            metadata: artifact_metadata(&snapshot)?,
            snapshot,
        };
        write_compressed_artifact(artifact_path, &artifact)?;
        Ok(GraphArtifactReport {
            artifact_path: artifact_path.to_path_buf(),
            snapshot_path: self.snapshot_path(),
            legacy_graph_path: self.legacy_graph_path(),
            metadata: artifact.metadata,
        })
    }

    /// Imports a gzip-compressed graph artifact after checksum and schema
    /// compatibility validation. A successful import writes both the current
    /// versioned snapshot and the legacy graph export.
    pub fn import_artifact(&self, artifact_path: &Path) -> io::Result<GraphArtifactReport> {
        let artifact = read_compressed_artifact(artifact_path)?;
        verify_artifact(&artifact)?;
        let snapshot = migrate(artifact.snapshot)?;
        let metadata = artifact_metadata(&snapshot)?;
        self.save(&snapshot.graph)?;
        Ok(GraphArtifactReport {
            artifact_path: artifact_path.to_path_buf(),
            snapshot_path: self.snapshot_path(),
            legacy_graph_path: self.legacy_graph_path(),
            metadata,
        })
    }
}

/// Result of a graph store save operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphStoreWriteOutcome {
    /// Versioned graph snapshot path.
    pub snapshot_path: PathBuf,
    /// Primary LadybugDB projection path.
    pub ladybug_path: PathBuf,
    /// Legacy graph export path.
    pub legacy_graph_path: PathBuf,
    /// Whether the Ladybug projection changed on disk.
    pub ladybug_written: bool,
    /// Whether the versioned snapshot changed on disk.
    pub snapshot_written: bool,
    /// Whether the legacy graph export changed on disk.
    pub legacy_written: bool,
}

fn migrate(mut snapshot: GraphSnapshot) -> io::Result<GraphSnapshot> {
    if snapshot.metadata.schema_version > GRAPH_STORE_SCHEMA_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "graph snapshot schema version {} is newer than supported version {}",
                snapshot.metadata.schema_version, GRAPH_STORE_SCHEMA_VERSION
            ),
        ));
    }
    if snapshot.metadata.schema_version < GRAPH_STORE_SCHEMA_VERSION {
        snapshot.metadata.migrations_applied.push(format!(
            "schema:{}->{}",
            snapshot.metadata.schema_version, GRAPH_STORE_SCHEMA_VERSION
        ));
        snapshot.metadata.schema_version = GRAPH_STORE_SCHEMA_VERSION;
    }
    if snapshot.metadata.graph_model_version > GRAPH_MODEL_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "graph model version {} is newer than supported version {}",
                snapshot.metadata.graph_model_version, GRAPH_MODEL_VERSION
            ),
        ));
    }
    if snapshot.metadata.graph_model_version < GRAPH_MODEL_VERSION {
        snapshot.metadata.migrations_applied.push(format!(
            "graph:{}->{}",
            snapshot.metadata.graph_model_version, GRAPH_MODEL_VERSION
        ));
        snapshot.metadata.graph_model_version = GRAPH_MODEL_VERSION;
    }
    snapshot.metadata.node_count = snapshot.graph.nodes.len();
    snapshot.metadata.relation_count = snapshot.graph.relations.len();
    Ok(snapshot)
}

fn artifact_metadata(snapshot: &GraphSnapshot) -> io::Result<GraphArtifactMetadata> {
    Ok(GraphArtifactMetadata {
        artifact_format_version: GRAPH_ARTIFACT_FORMAT_VERSION,
        compression: "gzip".to_owned(),
        checksum_algorithm: "blake3".to_owned(),
        snapshot_checksum: checksum_snapshot(snapshot)?,
        schema_version: snapshot.metadata.schema_version,
        graph_model_version: snapshot.metadata.graph_model_version,
        node_count: snapshot.graph.nodes.len(),
        relation_count: snapshot.graph.relations.len(),
    })
}

fn checksum_snapshot(snapshot: &GraphSnapshot) -> io::Result<String> {
    let bytes = serde_json::to_vec(snapshot).map_err(invalid_json)?;
    Ok(blake3::hash(&bytes).to_hex().to_string())
}

fn verify_artifact(artifact: &GraphArtifact) -> io::Result<()> {
    if artifact.metadata.artifact_format_version > GRAPH_ARTIFACT_FORMAT_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "graph artifact format version {} is newer than supported version {}",
                artifact.metadata.artifact_format_version, GRAPH_ARTIFACT_FORMAT_VERSION
            ),
        ));
    }
    if artifact.metadata.compression != "gzip" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported graph artifact compression {}",
                artifact.metadata.compression
            ),
        ));
    }
    if artifact.metadata.checksum_algorithm != "blake3" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "unsupported graph artifact checksum algorithm {}",
                artifact.metadata.checksum_algorithm
            ),
        ));
    }
    let expected = checksum_snapshot(&artifact.snapshot)?;
    if artifact.metadata.snapshot_checksum != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "graph artifact checksum does not match snapshot payload",
        ));
    }
    migrate(artifact.snapshot.clone())?;
    Ok(())
}

fn write_compressed_artifact(path: &Path, artifact: &GraphArtifact) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let bytes = serde_json::to_vec(artifact).map_err(invalid_json)?;
    let file = std::fs::File::create(path)?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    encoder.write_all(&bytes)?;
    encoder.finish()?;
    Ok(())
}

fn read_compressed_artifact(path: &Path) -> io::Result<GraphArtifact> {
    let file = std::fs::File::open(path)?;
    let mut decoder = GzDecoder::new(file);
    let mut bytes = Vec::new();
    decoder
        .read_to_end(&mut bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    serde_json::from_slice(&bytes).map_err(invalid_json)
}

fn invalid_json(error: serde_json::Error) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

#[cfg(test)]
mod tests {
    use super::{
        GRAPH_STORE_SCHEMA_VERSION, GraphArtifact, GraphSnapshot, GraphStore, artifact_metadata,
        write_compressed_artifact,
    };
    use crate::graph::Graph;
    use crate::storage::JsonStore;

    #[test]
    fn saves_versioned_snapshot_and_legacy_export() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let graph = Graph::default();
        let store = GraphStore::new(temp.path());

        let outcome = store.save(&graph)?;

        assert!(outcome.snapshot_written);
        assert!(outcome.legacy_written);
        assert!(store.snapshot_path().exists());
        assert!(store.legacy_graph_path().exists());

        let snapshot = store.load()?;

        assert_eq!(snapshot.graph, graph);
        assert_eq!(snapshot.metadata.schema_version, GRAPH_STORE_SCHEMA_VERSION);
        assert_eq!(snapshot.metadata.node_count, 0);
        assert_eq!(snapshot.metadata.relation_count, 0);

        Ok(())
    }

    #[test]
    fn falls_back_to_legacy_graph_export() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let graph = Graph::default();
        let store = GraphStore::new(temp.path());
        JsonStore.write(&store.legacy_graph_path(), &graph)?;

        let snapshot = store.load()?;

        assert_eq!(snapshot.graph, graph);
        assert_eq!(snapshot.metadata.schema_version, GRAPH_STORE_SCHEMA_VERSION);

        Ok(())
    }

    #[test]
    fn rejects_newer_snapshot_schema_versions() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = GraphStore::new(temp.path());
        let mut snapshot = GraphSnapshot::current(Graph::default());
        snapshot.metadata.schema_version = GRAPH_STORE_SCHEMA_VERSION + 1;
        JsonStore.write(&store.snapshot_path(), &snapshot)?;

        let error = match store.load() {
            Ok(_) => {
                return Err(std::io::Error::other("newer schema unexpectedly loaded").into());
            }
            Err(error) => error,
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        Ok(())
    }

    #[test]
    fn exports_and_imports_compressed_graph_artifact() -> Result<(), Box<dyn std::error::Error>> {
        let source = tempfile::TempDir::new()?;
        let destination = tempfile::TempDir::new()?;
        let graph = Graph::default();
        let source_store = GraphStore::new(source.path());
        let destination_store = GraphStore::new(destination.path());
        source_store.save(&graph)?;
        let artifact_path = source.path().join("graph.lithograph-graph.gz");

        let exported = source_store.export_artifact(&artifact_path)?;
        let imported = destination_store.import_artifact(&artifact_path)?;

        assert!(artifact_path.exists());
        assert_eq!(exported.metadata.compression, "gzip");
        assert_eq!(exported.metadata.checksum_algorithm, "blake3");
        assert_eq!(
            imported.metadata.snapshot_checksum,
            exported.metadata.snapshot_checksum
        );
        assert_eq!(destination_store.load()?.graph, graph);
        assert!(destination_store.snapshot_path().exists());
        assert!(destination_store.legacy_graph_path().exists());

        Ok(())
    }

    #[test]
    fn rejects_corrupt_graph_artifact() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = GraphStore::new(temp.path());
        let artifact_path = temp.path().join("corrupt.lithograph-graph.gz");
        std::fs::write(&artifact_path, b"not gzip")?;

        let error = match store.import_artifact(&artifact_path) {
            Ok(_) => return Err(std::io::Error::other("corrupt artifact imported").into()),
            Err(error) => error,
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        Ok(())
    }

    #[test]
    fn rejects_graph_artifact_with_mismatched_checksum() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = GraphStore::new(temp.path());
        let snapshot = GraphSnapshot::current(Graph::default());
        let mut metadata = artifact_metadata(&snapshot)?;
        metadata.snapshot_checksum = "wrong".to_owned();
        let artifact_path = temp.path().join("checksum.lithograph-graph.gz");
        write_compressed_artifact(&artifact_path, &GraphArtifact { metadata, snapshot })?;

        let error = match store.import_artifact(&artifact_path) {
            Ok(_) => return Err(std::io::Error::other("bad checksum imported").into()),
            Err(error) => error,
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        Ok(())
    }

    #[test]
    fn rejects_incompatible_graph_artifact_schema() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let store = GraphStore::new(temp.path());
        let mut snapshot = GraphSnapshot::current(Graph::default());
        snapshot.metadata.schema_version = GRAPH_STORE_SCHEMA_VERSION + 1;
        let metadata = artifact_metadata(&snapshot)?;
        let artifact_path = temp.path().join("future.lithograph-graph.gz");
        write_compressed_artifact(&artifact_path, &GraphArtifact { metadata, snapshot })?;

        let error = match store.import_artifact(&artifact_path) {
            Ok(_) => return Err(std::io::Error::other("future schema imported").into()),
            Err(error) => error,
        };

        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);

        Ok(())
    }
}
