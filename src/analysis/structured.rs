//! Structured YAML, JSON, and TOML configuration analysis.

use crate::domain::{Artifact, ArtifactId, EvidenceRef, ModelExposurePolicy, TextStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Number, Value};

/// Supported structured configuration formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum StructuredFormat {
    /// YAML document.
    Yaml,
    /// JSON document.
    Json,
    /// TOML document.
    Toml,
}

/// Structured analysis output for one configuration artifact.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct StructuredAnalysis {
    /// Config tree entities in deterministic walk order.
    pub entities: Vec<ConfigEntity>,
    /// Practical references extracted from scalar values and known config keys.
    pub references: Vec<ConfigReference>,
    /// Parse error when the document is malformed.
    pub parse_error: Option<String>,
}

/// One config tree node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConfigEntity {
    /// Stable config path such as `$`, `service.image`, or `required[0]`.
    pub config_path: String,
    /// Node value kind.
    pub value_kind: ConfigValueKind,
    /// Scalar summary for scalar nodes.
    pub scalar_summary: Option<String>,
    /// Evidence for this config path.
    pub evidence: EvidenceRef,
}

/// Config value category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ConfigValueKind {
    /// Mapping/object value.
    Object,
    /// Sequence/array value.
    Array,
    /// String scalar.
    String,
    /// Numeric scalar.
    Number,
    /// Boolean scalar.
    Boolean,
    /// Null scalar.
    Null,
}

/// Reference category extracted from config.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) enum ConfigReferenceKind {
    /// Local or absolute path-like value.
    Path,
    /// URL value.
    Url,
    /// TCP/HTTP port number.
    Port,
    /// Container image-like value.
    Image,
    /// Service name or service key.
    Service,
    /// Command string.
    Command,
    /// Environment variable name.
    EnvironmentVariable,
}

/// Config reference with structured-path evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConfigReference {
    /// Reference kind.
    pub kind: ConfigReferenceKind,
    /// Extracted reference value.
    pub value: String,
    /// Config path where the reference was found.
    pub config_path: String,
    /// Evidence for this config path.
    pub evidence: EvidenceRef,
}

/// Parser-backed analyzer for YAML, JSON, and TOML.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StructuredAnalyzer;

impl StructuredAnalyzer {
    /// Parses and analyzes a safe structured text artifact.
    pub(crate) fn analyze(
        &self,
        artifact: &Artifact,
        text: &str,
        format: StructuredFormat,
    ) -> StructuredAnalysis {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return StructuredAnalysis::default();
        }

        match parse_value(text, format) {
            Ok(value) => {
                let mut analysis = StructuredAnalysis::default();
                walk_value(&mut analysis, artifact, "$", &value);
                analysis
            }
            Err(error) => StructuredAnalysis {
                parse_error: Some(error),
                ..StructuredAnalysis::default()
            },
        }
    }
}

pub(crate) fn parse_value(text: &str, format: StructuredFormat) -> Result<Value, String> {
    match format {
        StructuredFormat::Json => serde_json::from_str(text).map_err(|error| error.to_string()),
        StructuredFormat::Yaml => {
            let yaml: serde_yaml::Value =
                serde_yaml::from_str(text).map_err(|error| error.to_string())?;
            Ok(serde_json::to_value(yaml).unwrap_or(Value::Null))
        }
        StructuredFormat::Toml => {
            let toml: toml::Value = toml::from_str(text).map_err(|error| error.to_string())?;
            Ok(serde_json::to_value(toml).unwrap_or(Value::Null))
        }
    }
}

fn walk_value(analysis: &mut StructuredAnalysis, artifact: &Artifact, path: &str, value: &Value) {
    analysis.entities.push(ConfigEntity {
        config_path: path.to_owned(),
        value_kind: value_kind(value),
        scalar_summary: scalar_summary(value),
        evidence: evidence(artifact, path),
    });

    match value {
        Value::Object(object) => walk_object(analysis, artifact, path, object),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk_value(analysis, artifact, &array_path(path, index), item);
            }
        }
        Value::String(text) => extract_scalar_references(analysis, artifact, path, text),
        Value::Number(number) => extract_number_references(analysis, artifact, path, number),
        Value::Bool(_) | Value::Null => {}
    }
}

fn walk_object(
    analysis: &mut StructuredAnalysis,
    artifact: &Artifact,
    path: &str,
    object: &Map<String, Value>,
) {
    for (key, value) in object {
        let child_path = child_path(path, key);
        if path == "services" || path == "$.services" {
            push_reference(
                analysis,
                artifact,
                ConfigReferenceKind::Service,
                key,
                &child_path,
            );
        }
        if key == "environment" || key == "env" {
            extract_environment_keys(analysis, artifact, &child_path, value);
        }
        walk_value(analysis, artifact, &child_path, value);
    }
}

fn extract_environment_keys(
    analysis: &mut StructuredAnalysis,
    artifact: &Artifact,
    path: &str,
    value: &Value,
) {
    if let Value::Object(object) = value {
        for key in object.keys() {
            push_reference(
                analysis,
                artifact,
                ConfigReferenceKind::EnvironmentVariable,
                key,
                &child_path(path, key),
            );
        }
    }
}

fn extract_scalar_references(
    analysis: &mut StructuredAnalysis,
    artifact: &Artifact,
    path: &str,
    text: &str,
) {
    if is_url(text) {
        push_reference(analysis, artifact, ConfigReferenceKind::Url, text, path);
    }
    if let Some(name) = env_var_name(text) {
        push_reference(
            analysis,
            artifact,
            ConfigReferenceKind::EnvironmentVariable,
            &name,
            path,
        );
    }
    if is_command_path(path) || is_command_text(text) {
        push_reference(analysis, artifact, ConfigReferenceKind::Command, text, path);
    }
    if is_image_path(path) || is_image_value(text) {
        push_reference(analysis, artifact, ConfigReferenceKind::Image, text, path);
    } else if is_path_value(text) {
        push_reference(analysis, artifact, ConfigReferenceKind::Path, text, path);
    }
    if is_service_path(path) {
        push_reference(analysis, artifact, ConfigReferenceKind::Service, text, path);
    }
    extract_port_text(analysis, artifact, path, text);
}

fn extract_number_references(
    analysis: &mut StructuredAnalysis,
    artifact: &Artifact,
    path: &str,
    number: &Number,
) {
    if is_port_path(path) {
        push_reference(
            analysis,
            artifact,
            ConfigReferenceKind::Port,
            &number.to_string(),
            path,
        );
    }
}

fn extract_port_text(
    analysis: &mut StructuredAnalysis,
    artifact: &Artifact,
    path: &str,
    text: &str,
) {
    if !is_port_path(path) && !text.contains(':') {
        return;
    }
    for part in text.split(':') {
        if part.chars().all(|character| character.is_ascii_digit()) && !part.is_empty() {
            push_reference(analysis, artifact, ConfigReferenceKind::Port, part, path);
        }
    }
}

fn push_reference(
    analysis: &mut StructuredAnalysis,
    artifact: &Artifact,
    kind: ConfigReferenceKind,
    value: &str,
    path: &str,
) {
    analysis.references.push(ConfigReference {
        kind,
        value: value.to_owned(),
        config_path: path.to_owned(),
        evidence: evidence(artifact, path),
    });
}

fn value_kind(value: &Value) -> ConfigValueKind {
    match value {
        Value::Object(_) => ConfigValueKind::Object,
        Value::Array(_) => ConfigValueKind::Array,
        Value::String(_) => ConfigValueKind::String,
        Value::Number(_) => ConfigValueKind::Number,
        Value::Bool(_) => ConfigValueKind::Boolean,
        Value::Null => ConfigValueKind::Null,
    }
}

fn scalar_summary(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.to_owned()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        Value::Null => Some("null".to_owned()),
        Value::Array(_) | Value::Object(_) => None,
    }
}

fn child_path(parent: &str, key: &str) -> String {
    if parent == "$" {
        key.to_owned()
    } else {
        format!("{parent}.{key}")
    }
}

fn array_path(parent: &str, index: usize) -> String {
    format!("{parent}[{index}]")
}

fn evidence(artifact: &Artifact, path: &str) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone())
        .with_structured_path(path)
}

fn is_url(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

fn env_var_name(text: &str) -> Option<String> {
    if let Some((_, rest)) = text.split_once("${") {
        return Some(rest.split(['}', ':']).next().unwrap_or(rest).to_owned());
    }
    if text
        .chars()
        .all(|character| character.is_ascii_uppercase() || character == '_')
        && text.contains('_')
    {
        return Some(text.to_owned());
    }
    None
}

fn is_path_value(text: &str) -> bool {
    !is_url(text)
        && (text.starts_with("./")
            || text.starts_with("/")
            || text.contains('/')
            || [".json", ".yaml", ".yml", ".toml", ".rs", ".py"]
                .iter()
                .any(|extension| text.ends_with(extension)))
}

fn is_image_value(text: &str) -> bool {
    !is_url(text) && text.contains('/') && text.contains(':') && !text.contains(' ')
}

fn is_command_text(text: &str) -> bool {
    let first = text.split_whitespace().next().unwrap_or("");
    matches!(
        first,
        "cargo" | "docker" | "make" | "npm" | "pnpm" | "python" | "pytest" | "yarn" | "vite"
    )
}

fn is_command_path(path: &str) -> bool {
    path.ends_with(".command") || path.ends_with(".run") || path.contains(".scripts.")
}

fn is_image_path(path: &str) -> bool {
    path.ends_with(".image") || path.ends_with(".default") && path.contains("image")
}

fn is_service_path(path: &str) -> bool {
    path.ends_with(".name") && path.contains("service")
}

fn is_port_path(path: &str) -> bool {
    path.ends_with(".port") || path.contains(".ports")
}

#[cfg(test)]
mod tests {
    use super::{
        ConfigReferenceKind, ConfigValueKind, StructuredAnalysis, StructuredAnalyzer,
        StructuredFormat,
    };
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;

    #[test]
    fn structured_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let yaml = analyze_fixture(&root, "config/settings.yaml", StructuredFormat::Yaml)?;
        let json = analyze_fixture(&root, "config/schema.json", StructuredFormat::Json)?;
        let toml = analyze_fixture(&root, "pyproject.toml", StructuredFormat::Toml)?;

        assert_eq!(
            snapshot(
                &yaml,
                &[
                    "service.name",
                    "service.image",
                    "service.port",
                    "service.command",
                    "service.env.RIDGELINE_WORKER",
                    "paths.schema",
                ]
            ),
            "\
service.name|String|route-api
service.image|String|ghcr.io/example/route-api:${VERSION}
service.port|Number|8080
service.command|String|python -m python_app.service
service.env.RIDGELINE_WORKER|String|/usr/local/bin/worker
paths.schema|String|config/schema.json
ref:Path:config/schema.json:paths.schema
ref:Path:assets/:paths.static_assets
ref:Command:python -m python_app.service:service.command
ref:EnvironmentVariable:RIDGELINE_CACHE_DIR:service.env.RIDGELINE_CACHE_DIR
ref:EnvironmentVariable:RIDGELINE_WORKER:service.env.RIDGELINE_WORKER
ref:Path:/var/cache/ridgeline:service.env.RIDGELINE_CACHE_DIR
ref:Path:/usr/local/bin/worker:service.env.RIDGELINE_WORKER
ref:EnvironmentVariable:VERSION:service.image
ref:Image:ghcr.io/example/route-api:${VERSION}:service.image
ref:Service:route-api:service.name
ref:Port:8080:service.port"
        );
        assert_eq!(
            snapshot(
                &json,
                &["$schema", "title", "properties.worker_image.default"]
            ),
            "\
$schema|String|https://json-schema.org/draft/2020-12/schema
title|String|RouteSettings
properties.worker_image.default|String|ghcr.io/example/worker:1.0
ref:Url:https://json-schema.org/draft/2020-12/schema:$schema
ref:Image:ghcr.io/example/worker:1.0:properties.worker_image.default"
        );
        assert_eq!(
            snapshot(
                &toml,
                &[
                    "project.name",
                    "project.dependencies[0]",
                    "tool.pytest.ini_options.testpaths[0]"
                ]
            ),
            "\
project.name|String|polyglot-fixture
project.dependencies[0]|String|pydantic>=2
tool.pytest.ini_options.testpaths[0]|String|tests"
        );

        Ok(())
    }

    #[test]
    fn structured_extracts_compose_and_package_refs() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let compose = analyze_fixture(&root, "docker-compose.yml", StructuredFormat::Yaml)?;
        let package = analyze_fixture(&root, "web/package.json", StructuredFormat::Json)?;

        assert!(has_ref(
            &compose,
            ConfigReferenceKind::Service,
            "api",
            "services.api"
        ));
        assert!(has_ref(
            &compose,
            ConfigReferenceKind::Image,
            "node:24-alpine",
            "services.web.image"
        ));
        assert!(has_ref(
            &compose,
            ConfigReferenceKind::Port,
            "8080",
            "services.api.ports[0]"
        ));
        assert!(has_ref(
            &package,
            ConfigReferenceKind::Command,
            "vite --host 0.0.0.0",
            "scripts.dev"
        ));

        Ok(())
    }

    #[test]
    fn structured_respects_policy_and_reports_parse_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = structured_artifact("config/settings.yaml")?;
        let never = structured_artifact("config/secret.yaml")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let binary =
            structured_artifact("config/binary.yaml")?.with_text_status(TextStatus::Binary, None);

        assert!(
            StructuredAnalyzer
                .analyze(&never, "key: value", StructuredFormat::Yaml)
                .entities
                .is_empty()
        );
        assert!(
            StructuredAnalyzer
                .analyze(&binary, "key: value", StructuredFormat::Yaml)
                .entities
                .is_empty()
        );
        assert!(
            StructuredAnalyzer
                .analyze(&artifact, ":", StructuredFormat::Yaml)
                .parse_error
                .is_some()
        );
        assert!(
            StructuredAnalyzer
                .analyze(&artifact, "{", StructuredFormat::Json)
                .parse_error
                .is_some()
        );
        assert!(
            StructuredAnalyzer
                .analyze(&artifact, "=", StructuredFormat::Toml)
                .parse_error
                .is_some()
        );

        Ok(())
    }

    #[test]
    fn structured_extracts_edge_scalar_refs() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = structured_artifact("config/custom.json")?;
        let text = r#"{
  "url": "http://example.test",
  "env": "RIDGELINE_CACHE_DIR",
  "path": "./data/file.py",
  "image": "docker.io/library/redis:7",
  "port": "127.0.0.1:9000",
  "enabled": true,
  "empty": null,
  "count": 3
}"#;
        let analysis = StructuredAnalyzer.analyze(&artifact, text, StructuredFormat::Json);

        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Url,
            "http://example.test",
            "url"
        ));
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::EnvironmentVariable,
            "RIDGELINE_CACHE_DIR",
            "env"
        ));
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Path,
            "./data/file.py",
            "path"
        ));
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Image,
            "docker.io/library/redis:7",
            "image"
        ));
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Port,
            "9000",
            "port"
        ));
        assert!(has_entity(
            &analysis,
            "enabled",
            ConfigValueKind::Boolean,
            Some("true")
        ));
        assert!(has_entity(
            &analysis,
            "empty",
            ConfigValueKind::Null,
            Some("null")
        ));
        assert!(has_entity(
            &analysis,
            "count",
            ConfigValueKind::Number,
            Some("3")
        ));
        assert_eq!(snapshot(&StructuredAnalysis::default(), &["missing"]), "");

        Ok(())
    }

    /// LIT-22.2.4 AC1: K8s and Kustomize manifests are plain YAML, so the
    /// path-based heuristics `StructuredAnalyzer` already applies to every
    /// YAML file (no K8s-specific analyzer needed) already extract their
    /// container image and local resource-path facts.
    #[test]
    fn structured_extracts_kubernetes_and_kustomize_facts() -> Result<(), Box<dyn std::error::Error>>
    {
        let deployment_artifact = structured_artifact("k8s/deployment.yaml")?;
        let deployment = r#"
apiVersion: apps/v1
kind: Deployment
metadata:
  name: route-api
spec:
  template:
    spec:
      containers:
        - name: route-api
          image: ghcr.io/example/route-api:1.2.3
"#;
        let analysis =
            StructuredAnalyzer.analyze(&deployment_artifact, deployment, StructuredFormat::Yaml);
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Image,
            "ghcr.io/example/route-api:1.2.3",
            "spec.template.spec.containers[0].image"
        ));

        let kustomization_artifact = structured_artifact("k8s/kustomization.yaml")?;
        let kustomization = r#"
resources:
  - ./deployment.yaml
  - ../base
"#;
        let analysis = StructuredAnalyzer.analyze(
            &kustomization_artifact,
            kustomization,
            StructuredFormat::Yaml,
        );
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Path,
            "./deployment.yaml",
            "resources[0]"
        ));
        assert!(has_ref(
            &analysis,
            ConfigReferenceKind::Path,
            "../base",
            "resources[1]"
        ));

        Ok(())
    }

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn analyze_fixture(
        root: &Path,
        path: &str,
        format: StructuredFormat,
    ) -> Result<StructuredAnalysis, Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact(root, path)?;
        Ok(StructuredAnalyzer.analyze(&artifact, &text, format))
    }

    fn fixture_artifact(
        root: &Path,
        path: &str,
    ) -> Result<(Artifact, String), Box<dyn std::error::Error>> {
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let not_found = std::io::ErrorKind::NotFound;
        let artifact = artifacts
            .into_iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or(std::io::Error::new(not_found, path.to_owned()))?;
        let text = fs::read_to_string(root.join(path))?;
        Ok((artifact, text))
    }

    fn structured_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::Configuration,
            SupportTier::StructuredFormat,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_detected_format("json")
        .with_text_status(TextStatus::Text, Some(1)))
    }

    fn snapshot(analysis: &StructuredAnalysis, paths: &[&str]) -> String {
        let mut lines = Vec::new();
        for path in paths {
            if let Some(entity) = analysis
                .entities
                .iter()
                .find(|entity| entity.config_path == *path)
            {
                lines.push(format!(
                    "{}|{:?}|{}",
                    entity.config_path,
                    entity.value_kind,
                    entity.scalar_summary.as_deref().unwrap_or("-")
                ));
            }
        }
        lines.extend(analysis.references.iter().map(|reference| {
            format!(
                "ref:{:?}:{}:{}",
                reference.kind, reference.value, reference.config_path
            )
        }));
        lines.join("\n")
    }

    fn has_ref(
        analysis: &StructuredAnalysis,
        kind: ConfigReferenceKind,
        value: &str,
        path: &str,
    ) -> bool {
        analysis.references.iter().any(|reference| {
            reference.kind == kind
                && reference.value == value
                && reference.config_path == path
                && reference.evidence.structured_path.as_deref() == Some(path)
        })
    }

    fn has_entity(
        analysis: &StructuredAnalysis,
        path: &str,
        kind: ConfigValueKind,
        summary: Option<&str>,
    ) -> bool {
        analysis.entities.iter().any(|entity| {
            entity.config_path == path
                && entity.value_kind == kind
                && entity.scalar_summary.as_deref() == summary
        })
    }
}
