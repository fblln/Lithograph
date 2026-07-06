//! Ordered artifact classification rules.

use crate::domain::{
    AnalyzerSelection, ArtifactCategory, ModelExposurePolicy, RepoPath, SupportTier, TextStatus,
};
use std::path::Path;

/// Input passed to the artifact classifier.
#[derive(Debug, Clone, Copy)]
pub struct ClassificationInput<'a> {
    /// Repository-relative path.
    pub path: &'a RepoPath,
    /// Text/binary status computed by inventory.
    pub text_status: TextStatus,
    /// UTF-8 text content when inventory established that reading is safe.
    pub text: Option<&'a str>,
}

/// Classification decision for a repository artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// Coarse artifact category.
    pub category: ArtifactCategory,
    /// Detected format or language.
    pub detected_format: Option<String>,
    /// Semantic support tier.
    pub support_tier: SupportTier,
    /// Model exposure policy after classification.
    pub model_policy: ModelExposurePolicy,
    /// Analyzer to run in later phases.
    pub analyzer: AnalyzerSelection,
    /// Generated-file probability from 0 to 100.
    pub generated_score: u8,
    /// Vendored-file probability from 0 to 100.
    pub vendored_score: u8,
}

impl Classification {
    fn new(
        category: ArtifactCategory,
        detected_format: Option<&str>,
        support_tier: SupportTier,
        analyzer: AnalyzerSelection,
    ) -> Self {
        Self {
            category,
            detected_format: detected_format.map(str::to_owned),
            support_tier,
            model_policy: ModelExposurePolicy::Allowed,
            analyzer,
            generated_score: 0,
            vendored_score: 0,
        }
    }

    fn with_origin_scores(mut self, generated_score: u8, vendored_score: u8) -> Self {
        self.generated_score = generated_score;
        self.vendored_score = vendored_score;
        self
    }

    fn with_model_policy(mut self, model_policy: ModelExposurePolicy) -> Self {
        self.model_policy = model_policy;
        self
    }
}

/// Deterministic layered artifact classifier.
#[derive(Debug, Clone, Copy, Default)]
pub struct ArtifactClassifier;

impl ArtifactClassifier {
    /// Classifies an artifact using stable ordered rules.
    pub fn classify(&self, input: ClassificationInput<'_>) -> Classification {
        let path = input.path.as_str();
        let mut classification = if let Some(classification) = exact_filename_rule(path) {
            classification
        } else if let Some(classification) = path_context_rule(path, input.text_status) {
            classification
        } else if let Some(classification) = extension_rule(path, input.text_status) {
            classification
        } else if let Some(classification) = shebang_rule(input.text) {
            classification
        } else if let Some(classification) = content_signature_rule(input.text) {
            classification
        } else if input.text_status == TextStatus::Binary {
            binary_fallback(path)
        } else {
            text_fallback()
        };

        classification = apply_origin_scores(path, classification);
        if input.text_status == TextStatus::Binary {
            classification.with_model_policy(ModelExposurePolicy::Never)
        } else {
            classification
        }
    }
}

fn exact_filename_rule(path: &str) -> Option<Classification> {
    let name = file_name(path);
    match name {
        "Dockerfile" => Some(structured(
            ArtifactCategory::ContainerDefinition,
            "dockerfile",
        )),
        "Makefile" => Some(generic(ArtifactCategory::BuildDefinition, Some("makefile"))),
        "Justfile" => Some(generic(ArtifactCategory::BuildDefinition, Some("justfile"))),
        "Jenkinsfile" => Some(generic(
            ArtifactCategory::ContinuousIntegration,
            Some("jenkinsfile"),
        )),
        "LICENSE" => Some(generic(ArtifactCategory::Documentation, Some("license"))),
        "NOTICE" => Some(generic(ArtifactCategory::Documentation, Some("notice"))),
        "Cargo.toml" => Some(structured(ArtifactCategory::PackageManifest, "toml")),
        "pyproject.toml" => Some(structured(ArtifactCategory::PackageManifest, "toml")),
        "requirements.txt" => Some(Classification::new(
            ArtifactCategory::PackageManifest,
            Some("requirements-txt"),
            SupportTier::GenericText,
            AnalyzerSelection::Specialized("requirements-txt".to_owned()),
        )),
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml" => Some(
            structured(ArtifactCategory::ContainerDefinition, "docker-compose"),
        ),
        _ => None,
    }
}

fn path_context_rule(path: &str, text_status: TextStatus) -> Option<Classification> {
    if path.starts_with(".github/workflows/") {
        return Some(structured(
            ArtifactCategory::ContinuousIntegration,
            "github-actions",
        ));
    }
    if has_component(path, "migrations") {
        return Some(generic(
            ArtifactCategory::DatabaseMigration,
            extension_name(path),
        ));
    }
    if path.contains("tests/fixtures/") || path.contains("test/fixtures/") {
        return Some(test_data_classification(path, text_status));
    }
    if has_component(path, "vendor") {
        return Some(
            extension_rule(path, text_status)
                .unwrap_or_else(text_fallback)
                .with_origin_scores(0, 100),
        );
    }

    None
}

fn extension_rule(path: &str, text_status: TextStatus) -> Option<Classification> {
    let extension = extension_name(path)?;
    match extension {
        "py" => Some(language("python", SupportTier::DeepLanguage)),
        "rs" => Some(language("rust", SupportTier::DeepLanguage)),
        "ts" => Some(language("typescript", SupportTier::GenericText)),
        "tsx" => Some(language("tsx", SupportTier::GenericText)),
        "js" => Some(language("javascript", SupportTier::GenericText)),
        "html" | "htm" => Some(generic(ArtifactCategory::Template, Some("html"))),
        "md" | "markdown" => Some(structured(ArtifactCategory::Documentation, "markdown")),
        "yaml" | "yml" => Some(structured(ArtifactCategory::Configuration, "yaml")),
        "json" => Some(structured(ArtifactCategory::Configuration, "json")),
        "toml" => Some(structured(ArtifactCategory::Configuration, "toml")),
        "lock" => Some(
            structured(ArtifactCategory::DependencyLockfile, "lockfile")
                .with_model_policy(ModelExposurePolicy::ExcerptOnly),
        ),
        "svg" => Some(static_asset("svg", text_status)),
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "ico" => {
            Some(static_asset(extension, text_status))
        }
        "bin" | "dat" | "wasm" | "so" | "dylib" | "dll" | "exe" => Some(binary_asset(extension)),
        _ => None,
    }
}

fn shebang_rule(text: Option<&str>) -> Option<Classification> {
    let first_line = text?.lines().next()?;
    if !first_line.starts_with("#!") {
        return None;
    }
    if first_line.contains("python") {
        return Some(language("python", SupportTier::DeepLanguage));
    }
    if first_line.contains("bash") || first_line.contains("sh") {
        return Some(generic(ArtifactCategory::Script, Some("shell")));
    }
    Some(generic(ArtifactCategory::Script, Some("script")))
}

fn content_signature_rule(text: Option<&str>) -> Option<Classification> {
    let text = text?.trim_start();
    if text.starts_with("FROM ") || text.starts_with("ARG ") {
        return Some(structured(
            ArtifactCategory::ContainerDefinition,
            "dockerfile",
        ));
    }
    if text.starts_with("<svg") || text.starts_with("<?xml") && text.contains("<svg") {
        return Some(static_asset("svg", TextStatus::Text));
    }
    None
}

fn apply_origin_scores(path: &str, classification: Classification) -> Classification {
    if has_component(path, "generated") {
        return Classification {
            category: ArtifactCategory::GeneratedSource,
            generated_score: 100,
            ..classification
        };
    }
    if has_component(path, "vendor") {
        return Classification {
            vendored_score: 100,
            ..classification
        };
    }
    classification
}

fn binary_fallback(path: &str) -> Classification {
    Classification::new(
        binary_category(path),
        extension_name(path),
        SupportTier::Opaque,
        AnalyzerSelection::Opaque,
    )
    .with_model_policy(ModelExposurePolicy::Never)
}

fn text_fallback() -> Classification {
    Classification::new(
        ArtifactCategory::UnknownText,
        None,
        SupportTier::GenericText,
        AnalyzerSelection::GenericText,
    )
}

fn language(format: &str, support_tier: SupportTier) -> Classification {
    let analyzer = match support_tier {
        SupportTier::DeepLanguage => AnalyzerSelection::Specialized(format.to_owned()),
        SupportTier::StructuredFormat | SupportTier::GenericText | SupportTier::Opaque => {
            AnalyzerSelection::GenericText
        }
    };
    Classification::new(
        ArtifactCategory::SourceCode,
        Some(format),
        support_tier,
        analyzer,
    )
}

fn structured(category: ArtifactCategory, format: &str) -> Classification {
    Classification::new(
        category,
        Some(format),
        SupportTier::StructuredFormat,
        AnalyzerSelection::Structured(format.to_owned()),
    )
}

fn generic(category: ArtifactCategory, format: Option<&str>) -> Classification {
    Classification::new(
        category,
        format,
        SupportTier::GenericText,
        AnalyzerSelection::GenericText,
    )
}

fn static_asset(format: &str, text_status: TextStatus) -> Classification {
    match text_status {
        TextStatus::Binary => Classification::new(
            ArtifactCategory::StaticAsset,
            Some(format),
            SupportTier::Opaque,
            AnalyzerSelection::Opaque,
        )
        .with_model_policy(ModelExposurePolicy::Never),
        TextStatus::Unknown | TextStatus::Text | TextStatus::UnsafeText => Classification::new(
            ArtifactCategory::StaticAsset,
            Some(format),
            SupportTier::GenericText,
            AnalyzerSelection::GenericText,
        )
        .with_model_policy(ModelExposurePolicy::ExcerptOnly),
    }
}

fn binary_asset(format: &str) -> Classification {
    Classification::new(
        ArtifactCategory::BinaryAsset,
        Some(format),
        SupportTier::Opaque,
        AnalyzerSelection::Opaque,
    )
    .with_model_policy(ModelExposurePolicy::Never)
}

fn test_data_classification(path: &str, text_status: TextStatus) -> Classification {
    match text_status {
        TextStatus::Binary => binary_asset(extension_name(path).unwrap_or("binary")),
        TextStatus::Unknown | TextStatus::Text | TextStatus::UnsafeText => {
            generic(ArtifactCategory::TestData, extension_name(path))
                .with_model_policy(ModelExposurePolicy::ExcerptOnly)
        }
    }
}

fn binary_category(_path: &str) -> ArtifactCategory {
    ArtifactCategory::UnknownBinary
}

fn file_name(path: &str) -> &str {
    Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

fn extension_name(path: &str) -> Option<&str> {
    Path::new(path)
        .extension()
        .and_then(|extension| extension.to_str())
}

fn has_component(path: &str, component: &str) -> bool {
    path.split('/').any(|part| part == component)
}

#[cfg(test)]
mod tests {
    use super::{ArtifactClassifier, ClassificationInput};
    use crate::domain::{
        AnalyzerSelection, ArtifactCategory, ModelExposurePolicy, RepoPath, TextStatus,
    };

    fn classify(
        path: &str,
        text_status: TextStatus,
        text: Option<&str>,
    ) -> Result<super::Classification, Box<dyn std::error::Error>> {
        let path = RepoPath::new(path)?;
        Ok(ArtifactClassifier.classify(ClassificationInput {
            path: &path,
            text_status,
            text,
        }))
    }

    #[test]
    fn exact_filename_rules_cover_first_release_names() -> Result<(), Box<dyn std::error::Error>> {
        let cases = [
            (
                "Dockerfile",
                ArtifactCategory::ContainerDefinition,
                Some("dockerfile"),
            ),
            (
                "Makefile",
                ArtifactCategory::BuildDefinition,
                Some("makefile"),
            ),
            (
                "Justfile",
                ArtifactCategory::BuildDefinition,
                Some("justfile"),
            ),
            (
                "Jenkinsfile",
                ArtifactCategory::ContinuousIntegration,
                Some("jenkinsfile"),
            ),
            ("LICENSE", ArtifactCategory::Documentation, Some("license")),
            ("NOTICE", ArtifactCategory::Documentation, Some("notice")),
            (
                "Cargo.toml",
                ArtifactCategory::PackageManifest,
                Some("toml"),
            ),
            (
                "pyproject.toml",
                ArtifactCategory::PackageManifest,
                Some("toml"),
            ),
            (
                "requirements.txt",
                ArtifactCategory::PackageManifest,
                Some("requirements-txt"),
            ),
        ];

        for (path, category, format) in cases {
            let classification = classify(path, TextStatus::Text, Some(""))?;
            assert_eq!(classification.category, category);
            assert_eq!(classification.detected_format.as_deref(), format);
        }
        assert_eq!(
            classify("requirements.txt", TextStatus::Text, Some(""))?.analyzer,
            AnalyzerSelection::Specialized("requirements-txt".to_owned())
        );

        Ok(())
    }

    #[test]
    fn path_context_rules_cover_ci_migrations_fixtures_and_vendor()
    -> Result<(), Box<dyn std::error::Error>> {
        let ci = classify(".github/workflows/ci.yml", TextStatus::Text, Some(""))?;
        let migration = classify("db/migrations/001_init.sql", TextStatus::Text, Some(""))?;
        let fixture = classify("tests/fixtures/sample.json", TextStatus::Text, Some("{}"))?;
        let vendor_text = "fn main() {}";
        let vendor = classify("vendor/example/lib.rs", TextStatus::Text, Some(vendor_text))?;

        assert_eq!(ci.category, ArtifactCategory::ContinuousIntegration);
        assert_eq!(ci.detected_format.as_deref(), Some("github-actions"));
        assert_eq!(migration.category, ArtifactCategory::DatabaseMigration);
        assert_eq!(fixture.category, ArtifactCategory::TestData);
        assert_eq!(fixture.model_policy, ModelExposurePolicy::ExcerptOnly);
        assert_eq!(vendor.category, ArtifactCategory::SourceCode);
        assert_eq!(vendor.vendored_score, 100);

        Ok(())
    }

    #[test]
    fn later_layers_apply_extensions_shebang_content_and_fallback()
    -> Result<(), Box<dyn std::error::Error>> {
        let rust = classify("src/lib.rs", TextStatus::Text, Some("pub fn run() {}"))?;
        let typescript = classify("web/src/app.ts", TextStatus::Text, Some("export {};\n"))?;
        let javascript = classify("web/src/app.js", TextStatus::Text, Some("export {};\n"))?;
        let toml = classify("config/settings.toml", TextStatus::Text, Some("[tool]\n"))?;
        let bash = "#!/usr/bin/env bash\n";
        let dockerfile = "FROM rust:latest\n";
        let script = classify("scripts/run", TextStatus::Text, Some(bash))?;
        let docker = classify("containers/app", TextStatus::Text, Some(dockerfile))?;
        let text = classify("notes/unknown", TextStatus::Text, Some("plain\n"))?;
        let binary = classify("data/blob.bin", TextStatus::Binary, None)?;

        assert_eq!(rust.detected_format.as_deref(), Some("rust"));
        assert_eq!(
            rust.analyzer,
            AnalyzerSelection::Specialized("rust".to_owned())
        );
        assert_eq!(typescript.detected_format.as_deref(), Some("typescript"));
        assert_eq!(javascript.detected_format.as_deref(), Some("javascript"));
        assert_eq!(toml.category, ArtifactCategory::Configuration);
        assert_eq!(script.category, ArtifactCategory::Script);
        assert_eq!(docker.category, ArtifactCategory::ContainerDefinition);
        assert_eq!(text.category, ArtifactCategory::UnknownText);
        assert_eq!(binary.category, ArtifactCategory::BinaryAsset);
        assert_eq!(binary.model_policy, ModelExposurePolicy::Never);

        Ok(())
    }

    #[test]
    fn coverage_rules_include_lockfiles_images_and_no_extension_binary()
    -> Result<(), Box<dyn std::error::Error>> {
        let lockfile = classify("Cargo.lock", TextStatus::Text, Some("[[package]]\n"))?;
        let image = classify("assets/logo.png", TextStatus::Binary, None)?;
        let fixture_binary = classify("tests/fixtures/blob", TextStatus::Binary, None)?;
        let opaque_binary = classify("data/blob", TextStatus::Binary, None)?;

        assert_eq!(lockfile.category, ArtifactCategory::DependencyLockfile);
        assert_eq!(lockfile.model_policy, ModelExposurePolicy::ExcerptOnly);
        assert_eq!(image.category, ArtifactCategory::StaticAsset);
        assert_eq!(image.model_policy, ModelExposurePolicy::Never);
        assert_eq!(fixture_binary.category, ArtifactCategory::BinaryAsset);
        assert_eq!(opaque_binary.category, ArtifactCategory::UnknownBinary);

        Ok(())
    }

    #[test]
    fn shebang_and_content_signature_edges_are_classified() -> Result<(), Box<dyn std::error::Error>>
    {
        let python_script = "#!/usr/bin/env python3\n";
        let perl_script = "#!/usr/bin/env perl\n";
        let docker_arg = "ARG BASE=alpine\n";
        let svg_text = "<svg viewBox=\"0 0 1 1\"/>";
        let xml_svg_text = "<?xml version=\"1.0\"?><svg></svg>";
        let python = classify("scripts/manage", TextStatus::Text, Some(python_script))?;
        let generic_script = classify("scripts/tool", TextStatus::Text, Some(perl_script))?;
        let arg_dockerfile = classify("containers/base", TextStatus::Text, Some(docker_arg))?;
        let svg = classify("assets/generated", TextStatus::Text, Some(svg_text))?;
        let xml_svg = classify("assets/xml-image", TextStatus::Text, Some(xml_svg_text))?;
        let empty = classify("empty", TextStatus::Text, Some(""))?;
        let no_text = classify("unknown", TextStatus::Unknown, None)?;

        assert_eq!(python.detected_format.as_deref(), Some("python"));
        assert_eq!(
            python.analyzer,
            AnalyzerSelection::Specialized("python".to_owned())
        );
        assert_eq!(generic_script.category, ArtifactCategory::Script);
        assert_eq!(generic_script.detected_format.as_deref(), Some("script"));
        assert_eq!(
            arg_dockerfile.category,
            ArtifactCategory::ContainerDefinition
        );
        assert_eq!(svg.category, ArtifactCategory::GeneratedSource);
        assert_eq!(svg.detected_format.as_deref(), Some("svg"));
        assert_eq!(xml_svg.category, ArtifactCategory::StaticAsset);
        assert_eq!(empty.category, ArtifactCategory::UnknownText);
        assert_eq!(no_text.category, ArtifactCategory::UnknownText);

        Ok(())
    }
}
