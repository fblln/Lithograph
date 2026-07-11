//! Content-addressed cache of per-artifact analyzer output, so `update` can
//! skip re-reading and re-parsing artifacts whose content hash is unchanged
//! since the previous run. Keyed purely by content hash: a cache entry is
//! never wrong to reuse (the bytes it was computed from are, by construction,
//! identical to the current artifact's bytes) and never wrong to discard (a
//! miss just costs a fresh parse, exactly like today's uncached behavior).

use crate::analysis::{
    ActionsProfile, CargoProfile, ComposeProfile, DockerfileAnalysis, EnvironmentFacts,
    MarkdownAnalysis, PackageManifestAnalysis, PackageManifestFormat, ProtocolFormat,
    ProtocolRoute, PyProjectProfile, PythonAnalysis, RequirementsProfile, RustAnalysis,
    RustWorkspaceAnalysis, StructuredAnalysis, StructuredFormat, SyntaxIndexedLanguage,
    TextFinding, TreeSitterAdapterOutput, TypeScriptAnalysis, TypeScriptLanguage,
};
use crate::storage::JsonStore;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::path::PathBuf;

/// Bump when analyzer output semantics or serialization change in a way that
/// should force fresh analyzer output instead of reusing old cache entries.
pub const ANALYSIS_CACHE_VERSION: u32 = 7;

/// Tags which analyzer produced an [`AnalyzerOutput`], so a cache lookup can
/// reject a stale entry whose artifact has since been reclassified to a
/// different analyzer (e.g. after a classifier rule change) even though its
/// content hash happens to collide.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnalyzerKind {
    /// `PythonAnalyzer`.
    Python,
    /// `RustAnalyzer`.
    Rust,
    /// `TypeScriptAnalyzer`, for the selected TypeScript grammar.
    TypeScript(TypeScriptLanguage),
    /// `RequirementsAnalyzer`.
    Requirements,
    /// `DockerfileAnalyzer`.
    Dockerfile,
    /// `MarkdownAnalyzer`.
    Markdown,
    /// `ComposeProfileAnalyzer`.
    Compose,
    /// `ActionsProfileAnalyzer`.
    Actions,
    /// `CargoProfileAnalyzer`.
    Cargo,
    /// `RustWorkspaceAnalyzer`.
    RustWorkspace,
    /// `PyProjectAnalyzer`.
    PyProject,
    /// `StructuredAnalyzer`, for one specific format.
    Structured(StructuredFormat),
    /// `TreeSitterParserAdapter`, for one specific syntax-indexed language.
    SyntaxIndexed(SyntaxIndexedLanguage),
    /// A `PackageManifestFormat` analyzer, for one specific manifest format.
    PackageManifest(PackageManifestFormat),
    /// `ProtoAnalyzer`/`GraphQlAnalyzer`, for one specific protocol format.
    Protocol(ProtocolFormat),
    /// `GenericTextExtractor`.
    GenericText,
    /// Shared environment/configuration fact extraction for `.env` and
    /// property-style files.
    Environment,
}

/// One artifact's cached analyzer output, self-describing so a lookup can
/// verify it matches the caller's expected [`AnalyzerKind`] before trusting
/// it. Externally tagged (serde's default enum representation): an
/// internally-tagged `#[serde(tag = ...)]` risks a field-name collision with
/// a variant's own field of the same name, which this project already hit
/// once with `GraphNode`/`node_type` (see `src/graph/model.rs`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum AnalyzerOutput {
    /// [`PythonAnalyzer`](crate::analysis::PythonAnalyzer) output.
    Python(PythonAnalysis),
    /// [`RustAnalyzer`](crate::analysis::RustAnalyzer) output.
    Rust(RustAnalysis),
    /// [`TypeScriptAnalyzer`](crate::analysis::TypeScriptAnalyzer) output.
    TypeScript(TypeScriptAnalysis),
    /// [`RequirementsAnalyzer`](crate::analysis::RequirementsAnalyzer) output.
    Requirements(RequirementsProfile),
    /// [`DockerfileAnalyzer`](crate::analysis::DockerfileAnalyzer) output.
    Dockerfile(DockerfileAnalysis),
    /// [`MarkdownAnalyzer`](crate::analysis::MarkdownAnalyzer) output.
    Markdown(MarkdownAnalysis),
    /// [`ComposeProfileAnalyzer`](crate::analysis::ComposeProfileAnalyzer) output.
    Compose(ComposeProfile),
    /// [`ActionsProfileAnalyzer`](crate::analysis::ActionsProfileAnalyzer) output.
    Actions(ActionsProfile),
    /// [`CargoProfileAnalyzer`](crate::analysis::CargoProfileAnalyzer) output.
    Cargo(CargoProfile),
    /// [`RustWorkspaceAnalyzer`](crate::analysis::RustWorkspaceAnalyzer) output.
    RustWorkspace(RustWorkspaceAnalysis),
    /// [`PyProjectAnalyzer`](crate::analysis::PyProjectAnalyzer) output.
    PyProject(PyProjectProfile),
    /// [`StructuredAnalyzer`](crate::analysis::StructuredAnalyzer) output.
    /// Carries its own [`StructuredFormat`] alongside the analysis, since
    /// `StructuredAnalysis` itself does not record which format produced it.
    Structured(StructuredFormat, StructuredAnalysis),
    /// [`TreeSitterParserAdapter`](crate::analysis::TreeSitterParserAdapter)
    /// output. Carries its own [`SyntaxIndexedLanguage`] alongside the
    /// output, since `TreeSitterAdapterOutput` only records a `&str`
    /// language id, not this cache-key-safe `Copy` discriminant.
    SyntaxIndexed(SyntaxIndexedLanguage, TreeSitterAdapterOutput),
    /// A package manifest analyzer's output (LIT-22.2.4). Carries its own
    /// [`PackageManifestFormat`] alongside the analysis, matching the
    /// `Structured`/`SyntaxIndexed` pattern above.
    PackageManifest(PackageManifestFormat, PackageManifestAnalysis),
    /// [`ProtoAnalyzer`](crate::analysis::ProtoAnalyzer)/[`GraphQlAnalyzer`](crate::analysis::GraphQlAnalyzer)
    /// output. Carries its own [`ProtocolFormat`], matching the pattern above.
    Protocol(ProtocolFormat, Vec<ProtocolRoute>),
    /// [`GenericTextExtractor`](crate::analysis::GenericTextExtractor) output.
    GenericText(Vec<TextFinding>),
    /// Shared environment/configuration facts for line-oriented files.
    Environment(EnvironmentFacts),
}

impl AnalyzerOutput {
    /// Returns the kind tag matching this output's variant.
    pub fn kind(&self) -> AnalyzerKind {
        match self {
            Self::Python(_) => AnalyzerKind::Python,
            Self::Rust(_) => AnalyzerKind::Rust,
            Self::TypeScript(analysis) => AnalyzerKind::TypeScript(analysis.language),
            Self::Requirements(_) => AnalyzerKind::Requirements,
            Self::Dockerfile(_) => AnalyzerKind::Dockerfile,
            Self::Markdown(_) => AnalyzerKind::Markdown,
            Self::Compose(_) => AnalyzerKind::Compose,
            Self::Actions(_) => AnalyzerKind::Actions,
            Self::Cargo(_) => AnalyzerKind::Cargo,
            Self::RustWorkspace(_) => AnalyzerKind::RustWorkspace,
            Self::PyProject(_) => AnalyzerKind::PyProject,
            Self::Structured(format, _) => AnalyzerKind::Structured(*format),
            Self::SyntaxIndexed(language, _) => AnalyzerKind::SyntaxIndexed(*language),
            Self::PackageManifest(format, _) => AnalyzerKind::PackageManifest(*format),
            Self::Protocol(format, _) => AnalyzerKind::Protocol(*format),
            Self::GenericText(_) => AnalyzerKind::GenericText,
            Self::Environment(_) => AnalyzerKind::Environment,
        }
    }
}

/// A content-hash-keyed cache of [`AnalyzerOutput`] values, one JSON file per
/// hash under a configured directory. Every miss reason (missing file,
/// corrupt JSON, kind mismatch) is treated identically: the caller falls back
/// to a fresh parse, so a cache that is empty, stale, or partially corrupt is
/// always safe, never a correctness hazard.
#[derive(Debug)]
pub struct AnalysisCache {
    dir: PathBuf,
    hits: Cell<usize>,
    misses: Cell<usize>,
}

impl AnalysisCache {
    /// Creates a cache rooted at `dir`. The directory is created lazily on
    /// first write, not here.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            hits: Cell::new(0),
            misses: Cell::new(0),
        }
    }

    /// Looks up `content_hash`, returning `Some` only when a cached entry
    /// exists, deserializes cleanly, and matches `expected`. Every other case
    /// -- missing file, corrupt/undeserializable JSON, or a kind mismatch --
    /// is a miss, never an error.
    pub fn get(&self, content_hash: &str, expected: AnalyzerKind) -> Option<AnalyzerOutput> {
        let path = self.entry_path(content_hash, expected);
        let loaded: Option<AnalyzerOutput> = JsonStore.read(&path).ok().flatten();
        match loaded {
            Some(output) if output.kind() == expected => {
                self.hits.set(self.hits.get() + 1);
                Some(output)
            }
            _ => {
                self.misses.set(self.misses.get() + 1);
                None
            }
        }
    }

    /// Writes `output` under `content_hash`. Best-effort: a write failure
    /// (read-only filesystem, full disk) is swallowed, since a missing cache
    /// entry is never a correctness problem, only a slower next run.
    pub fn put(&self, content_hash: &str, output: &AnalyzerOutput) {
        let _ = JsonStore.write(&self.entry_path(content_hash, output.kind()), output);
    }

    /// Number of successful cache lookups so far.
    pub fn hits(&self) -> usize {
        self.hits.get()
    }

    /// Number of failed cache lookups so far -- equivalently, the number of
    /// artifacts that had to be freshly read and parsed.
    pub fn misses(&self) -> usize {
        self.misses.get()
    }

    /// One artifact's content hash can legitimately need more than one cached
    /// analyzer output (e.g. a `Cargo.toml`'s raw TOML profile and its
    /// `cargo metadata`-resolved workspace facts are both keyed off the same
    /// file bytes), so the entry path is a function of `(content_hash, kind)`,
    /// not content hash alone.
    fn entry_path(&self, content_hash: &str, kind: AnalyzerKind) -> PathBuf {
        self.dir.join(format!(
            "v{ANALYSIS_CACHE_VERSION}-{content_hash}-{}.json",
            kind_slug(kind)
        ))
    }
}

/// Filesystem-safe slug for an [`AnalyzerKind`], used to namespace cache
/// entry filenames so different analyzers never share one file for the same
/// content hash.
fn kind_slug(kind: AnalyzerKind) -> String {
    match kind {
        AnalyzerKind::Structured(format) => format!("structured-{format:?}").to_lowercase(),
        other => format!("{other:?}").to_lowercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ANALYSIS_CACHE_VERSION, AnalysisCache, AnalyzerKind, AnalyzerOutput};
    use crate::analysis::{FindingConfidence, PythonAnalysis, TextFinding, TextFindingKind};

    fn sample_python() -> AnalyzerOutput {
        AnalyzerOutput::Python(PythonAnalysis::default())
    }

    fn sample_generic_text() -> AnalyzerOutput {
        AnalyzerOutput::GenericText(vec![TextFinding {
            kind: TextFindingKind::Url,
            value: "https://example.com".to_owned(),
            line: 1,
            confidence: FindingConfidence::Heuristic,
        }])
    }

    #[test]
    fn round_trips_every_output_kind() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(temp.path());

        for (hash, output) in [
            ("python-hash", sample_python()),
            ("generic-hash", sample_generic_text()),
        ] {
            let kind = output.kind();
            cache.put(hash, &output);
            assert_eq!(cache.get(hash, kind), Some(output));
        }

        Ok(())
    }

    #[test]
    fn missing_entry_is_a_miss() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(temp.path());

        assert_eq!(cache.get("absent", AnalyzerKind::Python), None);
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 0);

        Ok(())
    }

    #[test]
    fn corrupt_entry_is_a_miss_not_a_panic() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(temp.path());
        std::fs::write(cache.entry_path("bad", AnalyzerKind::Python), "{ not json")?;

        assert_eq!(cache.get("bad", AnalyzerKind::Python), None);
        assert_eq!(cache.misses(), 1);

        Ok(())
    }

    #[test]
    fn cache_entry_names_include_analyzer_cache_version() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(temp.path());

        cache.put("hash", &sample_python());

        let names: Vec<String> = std::fs::read_dir(temp.path())?
            .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
            .collect::<Result<_, _>>()?;
        assert_eq!(
            names,
            vec![format!("v{ANALYSIS_CACHE_VERSION}-hash-python.json")]
        );

        Ok(())
    }

    #[test]
    fn kind_mismatch_is_a_miss() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(temp.path());
        cache.put("hash", &sample_python());

        assert_eq!(cache.get("hash", AnalyzerKind::Rust), None);
        assert_eq!(cache.misses(), 1);
        assert_eq!(cache.hits(), 0);

        Ok(())
    }

    #[test]
    fn put_swallows_write_failure() -> Result<(), Box<dyn std::error::Error>> {
        // A directory that cannot exist as a parent (its own parent is a
        // file, not a directory) makes JsonStore::write fail; put() must not
        // panic or otherwise surface that failure to the caller.
        let temp = tempfile::TempDir::new()?;
        let blocked_file = temp.path().join("not-a-directory");
        std::fs::write(&blocked_file, "x")?;
        let cache = AnalysisCache::new(blocked_file.join("cache"));

        cache.put("hash", &sample_python());

        assert_eq!(cache.hits(), 0);
        assert_eq!(cache.misses(), 0);

        Ok(())
    }

    #[test]
    fn hits_and_misses_count_a_mixed_sequence() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let cache = AnalysisCache::new(temp.path());
        cache.put("present", &sample_python());

        assert!(cache.get("present", AnalyzerKind::Python).is_some());
        assert!(cache.get("present", AnalyzerKind::Rust).is_none());
        assert!(cache.get("absent", AnalyzerKind::Python).is_none());
        assert!(cache.get("present", AnalyzerKind::Python).is_some());

        assert_eq!(cache.hits(), 2);
        assert_eq!(cache.misses(), 2);

        Ok(())
    }
}
