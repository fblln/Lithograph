//! Package manifest analyzers for npm, Go, Composer, Maven, Gradle, and
//! .csproj, building a registry-backed package map (LIT-22.2.4). Each
//! analyzer extracts the manifest's own local package name (when the format
//! declares one) plus its declared dependencies; `GraphBuilder` turns these
//! into `Package` nodes distinguished by `is_external`, mirroring how
//! `CargoProfileAnalyzer`/`PyProjectAnalyzer` already feed the graph.

use crate::analysis::structured::{StructuredFormat, parse_value};
use crate::domain::{Artifact, ArtifactId, EvidenceRef, SourceSpan};
use serde::{Deserialize, Serialize};
use serde_json::Value;

fn file_evidence(artifact: &Artifact) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone())
}

fn line_evidence(artifact: &Artifact, line: usize) -> EvidenceRef {
    let line = u32::try_from(line.max(1)).unwrap_or(u32::MAX);
    match SourceSpan::new(line, line) {
        Ok(span) => file_evidence(artifact).with_span(span),
        Err(_) => file_evidence(artifact),
    }
}

/// Ecosystem a package manifest analyzer parsed. `Copy` so it can be used as
/// an [`AnalyzerKind`](crate::analysis::AnalyzerKind) cache-key discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum PackageManifestFormat {
    /// npm `package.json`.
    Npm,
    /// Go `go.mod`.
    GoMod,
    /// PHP Composer `composer.json`.
    Composer,
    /// Maven `pom.xml`.
    Maven,
    /// Gradle `build.gradle`/`build.gradle.kts`.
    Gradle,
    /// .NET `.csproj`.
    Csproj,
}

impl PackageManifestFormat {
    /// The registry/classifier format id matching this variant. Inverse of
    /// [`Self::from_format_id`].
    pub(crate) fn format_id(self) -> &'static str {
        match self {
            Self::Npm => "npm",
            Self::GoMod => "go-mod",
            Self::Composer => "composer",
            Self::Maven => "maven",
            Self::Gradle => "gradle",
            Self::Csproj => "csproj",
        }
    }

    /// Looks up the variant matching a classifier format id (see
    /// `inventory::classify::package_manifest`).
    pub(crate) fn from_format_id(id: &str) -> Option<Self> {
        Some(match id {
            "npm" => Self::Npm,
            "go-mod" => Self::GoMod,
            "composer" => Self::Composer,
            "maven" => Self::Maven,
            "gradle" => Self::Gradle,
            "csproj" => Self::Csproj,
            _ => return None,
        })
    }

    /// Runs this format's analyzer against `text`.
    pub(crate) fn analyze(self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        match self {
            Self::Npm => NpmPackageAnalyzer.analyze(artifact, text),
            Self::GoMod => GoModAnalyzer.analyze(artifact, text),
            Self::Composer => ComposerAnalyzer.analyze(artifact, text),
            Self::Maven => MavenPomAnalyzer.analyze(artifact, text),
            Self::Gradle => GradleAnalyzer.analyze(artifact, text),
            Self::Csproj => CsprojAnalyzer.analyze(artifact, text),
        }
    }
}

/// One dependency (or local package name) declared by a manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PackageDependency {
    /// Package name.
    pub name: String,
    /// Version requirement, when the manifest states one.
    pub version: Option<String>,
    /// Evidence for this entry.
    pub evidence: EvidenceRef,
}

/// Package facts extracted from one manifest file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct PackageManifestAnalysis {
    /// This manifest's own package name, when the format declares one.
    pub local_package: Option<PackageDependency>,
    /// Declared dependencies.
    pub dependencies: Vec<PackageDependency>,
    /// Parse error when the manifest is malformed.
    pub parse_error: Option<String>,
}

// --- npm: package.json -----------------------------------------------------

/// Parser-backed analyzer for npm `package.json` manifests.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct NpmPackageAnalyzer;

impl NpmPackageAnalyzer {
    /// Extracts package facts from `package.json` text.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        let value = match parse_value(text, StructuredFormat::Json) {
            Ok(value) => value,
            Err(error) => {
                return PackageManifestAnalysis {
                    parse_error: Some(error),
                    ..Default::default()
                };
            }
        };
        let Some(root) = value.as_object() else {
            return PackageManifestAnalysis::default();
        };

        let local_package = root.get("name").and_then(Value::as_str).map(|name| {
            let version = root
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned);
            PackageDependency {
                name: name.to_owned(),
                version,
                evidence: file_evidence(artifact),
            }
        });

        let mut dependencies = Vec::new();
        for key in ["dependencies", "devDependencies", "peerDependencies"] {
            let Some(map) = root.get(key).and_then(Value::as_object) else {
                continue;
            };
            for (name, requirement) in map {
                dependencies.push(PackageDependency {
                    name: name.clone(),
                    version: requirement.as_str().map(str::to_owned),
                    evidence: file_evidence(artifact),
                });
            }
        }

        PackageManifestAnalysis {
            local_package,
            dependencies,
            parse_error: None,
        }
    }
}

// --- Composer: composer.json ------------------------------------------------

/// Parser-backed analyzer for PHP Composer `composer.json` manifests.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ComposerAnalyzer;

impl ComposerAnalyzer {
    /// Extracts package facts from `composer.json` text.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        let value = match parse_value(text, StructuredFormat::Json) {
            Ok(value) => value,
            Err(error) => {
                return PackageManifestAnalysis {
                    parse_error: Some(error),
                    ..Default::default()
                };
            }
        };
        let Some(root) = value.as_object() else {
            return PackageManifestAnalysis::default();
        };

        let local_package = root.get("name").and_then(Value::as_str).map(|name| {
            let version = root
                .get("version")
                .and_then(Value::as_str)
                .map(str::to_owned);
            PackageDependency {
                name: name.to_owned(),
                version,
                evidence: file_evidence(artifact),
            }
        });

        let mut dependencies = Vec::new();
        for key in ["require", "require-dev"] {
            let Some(map) = root.get(key).and_then(Value::as_object) else {
                continue;
            };
            for (name, requirement) in map {
                // "php" and "ext-*" entries are platform requirements, not packages.
                if name == "php" || name.starts_with("ext-") {
                    continue;
                }
                dependencies.push(PackageDependency {
                    name: name.clone(),
                    version: requirement.as_str().map(str::to_owned),
                    evidence: file_evidence(artifact),
                });
            }
        }

        PackageManifestAnalysis {
            local_package,
            dependencies,
            parse_error: None,
        }
    }
}

// --- Go: go.mod --------------------------------------------------------------

/// Line-oriented analyzer for Go `go.mod` manifests. `go.mod` is not a
/// structured data format, so this parses the small stable grammar directly
/// rather than routing through [`parse_value`].
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GoModAnalyzer;

impl GoModAnalyzer {
    /// Extracts package facts from `go.mod` text.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        let mut local_package = None;
        let mut dependencies = Vec::new();
        let mut in_require_block = false;

        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            if let Some(module) = line.strip_prefix("module ") {
                local_package = Some(PackageDependency {
                    name: module.trim().to_owned(),
                    version: None,
                    evidence: line_evidence(artifact, index + 1),
                });
                continue;
            }
            if line == "require (" {
                in_require_block = true;
                continue;
            }
            if in_require_block {
                if line == ")" {
                    in_require_block = false;
                    continue;
                }
                if let Some(dependency) = parse_require_entry(line, artifact, index + 1) {
                    dependencies.push(dependency);
                }
                continue;
            }
            if let Some(entry) = line.strip_prefix("require ")
                && let Some(dependency) = parse_require_entry(entry, artifact, index + 1)
            {
                dependencies.push(dependency);
            }
        }

        PackageManifestAnalysis {
            local_package,
            dependencies,
            parse_error: None,
        }
    }
}

fn parse_require_entry(entry: &str, artifact: &Artifact, line: usize) -> Option<PackageDependency> {
    let mut parts = entry.split_whitespace();
    let name = parts.next()?;
    let version = parts.next().map(str::to_owned);
    Some(PackageDependency {
        name: name.to_owned(),
        version,
        evidence: line_evidence(artifact, line),
    })
}

// --- Maven: pom.xml and Gradle: build.gradle[.kts] --------------------------

/// XML-backed analyzer for Maven `pom.xml` manifests.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct MavenPomAnalyzer;

impl MavenPomAnalyzer {
    /// Extracts package facts from `pom.xml` text.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        let document = match roxmltree::Document::parse(text) {
            Ok(document) => document,
            Err(error) => {
                return PackageManifestAnalysis {
                    parse_error: Some(error.to_string()),
                    ..Default::default()
                };
            }
        };
        let Some(project) = document
            .root()
            .children()
            .find(|node| node.has_tag_name("project"))
        else {
            return PackageManifestAnalysis::default();
        };

        let group_id = xml_child_text(project, "groupId");
        let artifact_id = xml_child_text(project, "artifactId");
        let local_package = artifact_id.map(|artifact_id| {
            let name = group_id.map_or_else(
                || artifact_id.to_owned(),
                |group_id| format!("{group_id}:{artifact_id}"),
            );
            PackageDependency {
                name,
                version: xml_child_text(project, "version").map(str::to_owned),
                evidence: file_evidence(artifact),
            }
        });

        let mut dependencies = Vec::new();
        if let Some(dependencies_node) = project
            .children()
            .find(|node| node.has_tag_name("dependencies"))
        {
            for dependency_node in dependencies_node
                .children()
                .filter(|node| node.has_tag_name("dependency"))
            {
                let Some(artifact_id) = xml_child_text(dependency_node, "artifactId") else {
                    continue;
                };
                let name = xml_child_text(dependency_node, "groupId").map_or_else(
                    || artifact_id.to_owned(),
                    |group_id| format!("{group_id}:{artifact_id}"),
                );
                dependencies.push(PackageDependency {
                    name,
                    version: xml_child_text(dependency_node, "version").map(str::to_owned),
                    evidence: file_evidence(artifact),
                });
            }
        }

        PackageManifestAnalysis {
            local_package,
            dependencies,
            parse_error: None,
        }
    }
}

fn xml_child_text<'a>(node: roxmltree::Node<'a, 'a>, tag: &str) -> Option<&'a str> {
    node.children()
        .find(|child| child.has_tag_name(tag))
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
}

/// Heuristic analyzer for Gradle `build.gradle`/`build.gradle.kts` manifests.
/// Gradle build files are a Groovy/Kotlin DSL, not a data format, so this
/// scans for `<configuration>("group:artifact:version")`-shaped dependency
/// declarations rather than parsing the DSL. Gradle has no in-file local
/// package name (that lives in `settings.gradle`), so `local_package` is
/// always `None`.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct GradleAnalyzer;

const GRADLE_DEPENDENCY_CONFIGURATIONS: &[&str] = &[
    "api",
    "implementation",
    "compileOnly",
    "runtimeOnly",
    "testImplementation",
    "testApi",
    "testCompileOnly",
    "testRuntimeOnly",
    "kapt",
    "annotationProcessor",
];

impl GradleAnalyzer {
    /// Extracts dependency facts from `build.gradle`/`build.gradle.kts` text.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        let mut dependencies = Vec::new();
        for (index, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let Some(configuration) = GRADLE_DEPENDENCY_CONFIGURATIONS
                .iter()
                .find(|configuration| trimmed.starts_with(**configuration))
            else {
                continue;
            };
            let rest = trimmed[configuration.len()..].trim_start();
            if !rest.starts_with('(') && !rest.starts_with('"') && !rest.starts_with('\'') {
                continue;
            }
            let Some(coordinate) = quoted_literal(rest) else {
                continue;
            };
            // Gradle coordinates are `group:artifact:version`; keep the
            // `group:artifact` pair as the name and the rest as version, or
            // treat the whole string as the name when there's no version.
            let mut segments = coordinate.splitn(3, ':');
            let (Some(group), Some(artifact_name)) = (segments.next(), segments.next()) else {
                continue;
            };
            dependencies.push(PackageDependency {
                name: format!("{group}:{artifact_name}"),
                version: segments.next().map(str::to_owned),
                evidence: line_evidence(artifact, index + 1),
            });
        }

        PackageManifestAnalysis {
            local_package: None,
            dependencies,
            parse_error: None,
        }
    }
}

/// Extracts the first single- or double-quoted literal from `text`.
fn quoted_literal(text: &str) -> Option<&str> {
    let mut chars = text.char_indices();
    let (start, quote) = chars.find_map(|(index, character)| {
        (character == '"' || character == '\'').then_some((index, character))
    })?;
    let end = text[start + 1..].find(quote)? + start + 1;
    Some(&text[start + 1..end])
}

// --- .NET: .csproj ------------------------------------------------------------

/// XML-backed analyzer for .NET `.csproj` project files.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct CsprojAnalyzer;

impl CsprojAnalyzer {
    /// Extracts package facts from `.csproj` text. The project's own name is
    /// its `AssemblyName` property when present, otherwise the artifact's
    /// file stem (`MyApp.csproj` -> `MyApp`), matching how `dotnet` derives
    /// the default assembly name.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> PackageManifestAnalysis {
        let document = match roxmltree::Document::parse(text) {
            Ok(document) => document,
            Err(error) => {
                return PackageManifestAnalysis {
                    parse_error: Some(error.to_string()),
                    ..Default::default()
                };
            }
        };
        let Some(project) = document
            .root()
            .children()
            .find(|node| node.has_tag_name("Project"))
        else {
            return PackageManifestAnalysis::default();
        };

        let assembly_name = project
            .descendants()
            .find(|node| node.has_tag_name("AssemblyName"))
            .and_then(|node| node.text())
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
            .or_else(|| csproj_file_stem(artifact));
        let local_package = assembly_name.map(|name| PackageDependency {
            name,
            version: project
                .descendants()
                .find(|node| node.has_tag_name("Version"))
                .and_then(|node| node.text())
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            evidence: file_evidence(artifact),
        });

        let dependencies = project
            .descendants()
            .filter(|node| node.has_tag_name("PackageReference"))
            .filter_map(|node| {
                let name = node.attribute("Include")?;
                Some(PackageDependency {
                    name: name.to_owned(),
                    version: node.attribute("Version").map(str::to_owned),
                    evidence: file_evidence(artifact),
                })
            })
            .collect();

        PackageManifestAnalysis {
            local_package,
            dependencies,
            parse_error: None,
        }
    }
}

fn csproj_file_stem(artifact: &Artifact) -> Option<String> {
    std::path::Path::new(artifact.path.as_str())
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, RepoPath, SupportTier, TextStatus,
    };

    fn artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::PackageManifest,
            SupportTier::StructuredFormat,
            ContentHash::new("aaaaaaaa")?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn npm_extracts_local_package_and_dependency_kinds() -> Result<(), Box<dyn std::error::Error>> {
        let text = r#"{
            "name": "my-app",
            "version": "1.2.3",
            "dependencies": { "react": "^18.0.0" },
            "devDependencies": { "vitest": "^1.0.0" },
            "peerDependencies": { "react-dom": "^18.0.0" }
        }"#;
        let analysis = NpmPackageAnalyzer.analyze(&artifact("package.json")?, text);

        let local = analysis.local_package.ok_or("missing local package")?;
        assert_eq!(local.name, "my-app");
        assert_eq!(local.version.as_deref(), Some("1.2.3"));
        assert_eq!(analysis.dependencies.len(), 3);
        assert!(
            analysis
                .dependencies
                .iter()
                .any(|dependency| dependency.name == "react")
        );

        Ok(())
    }

    #[test]
    fn npm_malformed_json_reports_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = NpmPackageAnalyzer.analyze(&artifact("package.json")?, "{ not json");
        assert!(analysis.parse_error.is_some());
        assert!(analysis.dependencies.is_empty());

        Ok(())
    }

    #[test]
    fn composer_skips_platform_requirements() -> Result<(), Box<dyn std::error::Error>> {
        let text = r#"{
            "name": "acme/app",
            "require": { "php": "^8.2", "ext-json": "*", "guzzlehttp/guzzle": "^7.0" }
        }"#;
        let analysis = ComposerAnalyzer.analyze(&artifact("composer.json")?, text);

        assert_eq!(
            analysis.local_package.map(|p| p.name),
            Some("acme/app".to_owned())
        );
        assert_eq!(analysis.dependencies.len(), 1);
        assert_eq!(analysis.dependencies[0].name, "guzzlehttp/guzzle");

        Ok(())
    }

    #[test]
    fn go_mod_parses_module_and_require_block() -> Result<(), Box<dyn std::error::Error>> {
        let text = "module github.com/example/app\n\ngo 1.22\n\nrequire (\n\tgithub.com/foo/bar v1.2.3\n\tgithub.com/baz/qux v0.1.0 // indirect\n)\n\nrequire github.com/single/dep v2.0.0\n";
        let analysis = GoModAnalyzer.analyze(&artifact("go.mod")?, text);

        assert_eq!(
            analysis.local_package.map(|p| p.name),
            Some("github.com/example/app".to_owned())
        );
        assert_eq!(analysis.dependencies.len(), 3);
        assert!(
            analysis
                .dependencies
                .iter()
                .any(|dependency| dependency.name == "github.com/single/dep"
                    && dependency.version.as_deref() == Some("v2.0.0"))
        );

        Ok(())
    }

    #[test]
    fn go_mod_without_module_directive_has_no_local_package()
    -> Result<(), Box<dyn std::error::Error>> {
        let analysis = GoModAnalyzer.analyze(&artifact("go.mod")?, "go 1.22\n");
        assert!(analysis.local_package.is_none());
        assert!(analysis.dependencies.is_empty());

        Ok(())
    }

    #[test]
    fn maven_extracts_coordinates_and_dependencies() -> Result<(), Box<dyn std::error::Error>> {
        let text = r#"<project>
            <groupId>com.example</groupId>
            <artifactId>app</artifactId>
            <version>1.0.0</version>
            <dependencies>
                <dependency>
                    <groupId>org.apache.commons</groupId>
                    <artifactId>commons-lang3</artifactId>
                    <version>3.14.0</version>
                </dependency>
            </dependencies>
        </project>"#;
        let analysis = MavenPomAnalyzer.analyze(&artifact("pom.xml")?, text);

        let local = analysis.local_package.ok_or("missing local package")?;
        assert_eq!(local.name, "com.example:app");
        assert_eq!(local.version.as_deref(), Some("1.0.0"));
        assert_eq!(analysis.dependencies.len(), 1);
        assert_eq!(
            analysis.dependencies[0].name,
            "org.apache.commons:commons-lang3"
        );

        Ok(())
    }

    #[test]
    fn maven_malformed_xml_reports_parse_error() -> Result<(), Box<dyn std::error::Error>> {
        let analysis = MavenPomAnalyzer.analyze(&artifact("pom.xml")?, "<project><unclosed>");
        assert!(analysis.parse_error.is_some());

        Ok(())
    }

    #[test]
    fn gradle_extracts_quoted_dependency_coordinates() -> Result<(), Box<dyn std::error::Error>> {
        let text = "dependencies {\n    implementation(\"com.squareup.okhttp3:okhttp:4.12.0\")\n    testImplementation 'junit:junit:4.13.2'\n    api project(':core')\n}\n";
        let analysis = GradleAnalyzer.analyze(&artifact("build.gradle")?, text);

        assert!(analysis.local_package.is_none());
        assert_eq!(analysis.dependencies.len(), 2);
        assert!(analysis.dependencies.iter().any(|dependency| {
            dependency.name == "com.squareup.okhttp3:okhttp"
                && dependency.version.as_deref() == Some("4.12.0")
        }));
        assert!(
            analysis
                .dependencies
                .iter()
                .any(|dependency| dependency.name == "junit:junit")
        );

        Ok(())
    }

    #[test]
    fn csproj_extracts_package_references_and_assembly_name()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = r#"<Project Sdk="Microsoft.NET.Sdk">
            <PropertyGroup>
                <AssemblyName>Acme.App</AssemblyName>
            </PropertyGroup>
            <ItemGroup>
                <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
            </ItemGroup>
        </Project>"#;
        let analysis = CsprojAnalyzer.analyze(&artifact("src/App.csproj")?, text);

        assert_eq!(
            analysis.local_package.map(|p| p.name),
            Some("Acme.App".to_owned())
        );
        assert_eq!(analysis.dependencies.len(), 1);
        assert_eq!(analysis.dependencies[0].name, "Newtonsoft.Json");
        assert_eq!(analysis.dependencies[0].version.as_deref(), Some("13.0.3"));

        Ok(())
    }

    #[test]
    fn csproj_falls_back_to_file_stem_when_assembly_name_missing()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = r#"<Project Sdk="Microsoft.NET.Sdk"></Project>"#;
        let analysis = CsprojAnalyzer.analyze(&artifact("src/Widgets.csproj")?, text);

        assert_eq!(
            analysis.local_package.map(|p| p.name),
            Some("Widgets".to_owned())
        );

        Ok(())
    }

    #[test]
    fn package_manifest_format_round_trips_format_ids() {
        for id in ["npm", "go-mod", "composer", "maven", "gradle", "csproj"] {
            let Some(format) = PackageManifestFormat::from_format_id(id) else {
                unreachable!("missing PackageManifestFormat mapping for {id}");
            };
            assert_eq!(format.format_id(), id);
        }
        assert!(PackageManifestFormat::from_format_id("nuget").is_none());
    }
}
