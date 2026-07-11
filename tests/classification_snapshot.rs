//! Snapshot coverage for deterministic fixture artifact classification.

use lithograph::domain::{AnalyzerSelection, Artifact, ModelExposurePolicy};
use lithograph::inventory::{RepositoryWalker, WalkOptions};
use std::path::Path;

/// `vendor/example/lib.rs` classifies opaque (LIT-23.4): a directory
/// literally named `vendor` marks its content vendored_score 100, which
/// `VendorPolicy` now turns into `SupportTier::Opaque` /
/// `AnalyzerSelection::Opaque` / `ModelExposurePolicy::ExcerptOnly`, the
/// same treatment an oversized file already gets from `SizePolicy`.
#[test]
fn polyglot_fixture_classification_snapshot() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
    let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
    let snapshot = artifacts
        .iter()
        .map(snapshot_line)
        .collect::<Vec<_>>()
        .join("\n");

    assert_eq!(
        snapshot,
        "\
.github/workflows/ci.yml|ContinuousIntegration|github-actions|StructuredFormat|Text|Structured(github-actions)|Allowed|0|0
Dockerfile|ContainerDefinition|dockerfile|StructuredFormat|Text|Structured(dockerfile)|Allowed|0|0
LICENSE|Documentation|license|GenericText|Text|GenericText|Allowed|0|0
Makefile|BuildDefinition|makefile|GenericText|Text|GenericText|Allowed|0|0
README.md|Documentation|markdown|StructuredFormat|Text|Structured(markdown)|Allowed|0|0
assets/logo.svg|StaticAsset|svg|GenericText|Text|GenericText|ExcerptOnly|0|0
config/schema.json|Configuration|json|StructuredFormat|Text|Structured(json)|Allowed|0|0
config/settings.yaml|Configuration|yaml|StructuredFormat|Text|Structured(yaml)|Allowed|0|0
data/sample.bin|BinaryAsset|bin|Opaque|Binary|Opaque|Never|0|0
docker-compose.yml|ContainerDefinition|docker-compose|StructuredFormat|Text|Structured(docker-compose)|Allowed|0|0
docs/architecture.md|Documentation|markdown|StructuredFormat|Text|Structured(markdown)|Allowed|0|0
generated/client.py|GeneratedSource|python|DeepLanguage|Text|Specialized(python)|Allowed|100|0
pyproject.toml|PackageManifest|toml|StructuredFormat|Text|Structured(toml)|Allowed|0|0
requirements.txt|PackageManifest|requirements-txt|GenericText|Text|Specialized(requirements-txt)|Allowed|0|0
rust/Cargo.toml|PackageManifest|toml|StructuredFormat|Text|Structured(toml)|Allowed|0|0
rust/src/bin/worker.rs|SourceCode|rust|DeepLanguage|Text|Specialized(rust)|Allowed|0|0
rust/src/lib.rs|SourceCode|rust|DeepLanguage|Text|Specialized(rust)|Allowed|0|0
src/python_app/__init__.py|SourceCode|python|DeepLanguage|Text|Specialized(python)|Allowed|0|0
src/python_app/service.py|SourceCode|python|DeepLanguage|Text|Specialized(python)|Allowed|0|0
vendor/example/lib.rs|SourceCode|rust|Opaque|Text|Opaque|ExcerptOnly|0|100
web/index.html|Template|html|StructuredFormat|Text|SyntaxIndexed(html)|Allowed|0|0
web/package.json|PackageManifest|npm|StructuredFormat|Text|Specialized(npm)|Allowed|0|0
web/src/App.tsx|SourceCode|tsx|DeepLanguage|Text|Specialized(tsx)|Allowed|0|0"
    );

    Ok(())
}

fn snapshot_line(artifact: &Artifact) -> String {
    format!(
        "{}|{:?}|{}|{:?}|{:?}|{}|{}|{}|{}",
        artifact.path.as_str(),
        artifact.category,
        artifact.detected_format.as_deref().unwrap_or("-"),
        artifact.support_tier,
        artifact.text_status,
        analyzer_name(&artifact.analyzer),
        model_policy_name(artifact.model_policy),
        artifact.generated_score,
        artifact.vendored_score
    )
}

fn analyzer_name(analyzer: &AnalyzerSelection) -> String {
    match analyzer {
        AnalyzerSelection::Unassigned => "Unassigned".to_owned(),
        AnalyzerSelection::Specialized(name) => format!("Specialized({name})"),
        AnalyzerSelection::Structured(name) => format!("Structured({name})"),
        AnalyzerSelection::SyntaxIndexed(name) => format!("SyntaxIndexed({name})"),
        AnalyzerSelection::GenericText => "GenericText".to_owned(),
        AnalyzerSelection::Opaque => "Opaque".to_owned(),
    }
}

fn model_policy_name(model_policy: ModelExposurePolicy) -> &'static str {
    match model_policy {
        ModelExposurePolicy::Allowed => "Allowed",
        ModelExposurePolicy::ExcerptOnly => "ExcerptOnly",
        ModelExposurePolicy::Redacted => "Redacted",
        ModelExposurePolicy::Never => "Never",
    }
}
