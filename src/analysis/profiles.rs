//! Semantic profiles for package, compose, and CI files on top of generic
//! structured parsing.

use crate::analysis::structured::{StructuredFormat, parse_value};
use crate::domain::{
    Artifact, ArtifactId, EvidenceRef, ModelExposurePolicy, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

fn exposable(artifact: &Artifact) -> bool {
    artifact.text_status == TextStatus::Text && artifact.model_policy != ModelExposurePolicy::Never
}

fn evidence(artifact: &Artifact, path: &str) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone())
        .with_structured_path(path)
}

fn line_evidence(artifact: &Artifact, line_number: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    match SourceSpan::new(line_number, line_number) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

fn root_obj(value: &Value) -> Option<&Map<String, Value>> {
    value.as_object()
}

fn map_str<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn map_obj<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Map<String, Value>> {
    map.get(key).and_then(Value::as_object)
}

fn map_arr<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a Vec<Value>> {
    map.get(key).and_then(Value::as_array)
}

fn scalar_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(boolean) => Some(boolean.to_string()),
        _ => None,
    }
}

/// Key/value fact shared by Compose environment blocks and CI step environments.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvVarFact {
    /// Environment variable name.
    pub key: String,
    /// Assigned value, when a literal value is present.
    pub value: Option<String>,
    /// Evidence for this assignment.
    pub evidence: EvidenceRef,
}

fn object_environment(artifact: &Artifact, path: &str, value: &Value) -> Vec<EnvVarFact> {
    match value {
        Value::Object(map) => map
            .iter()
            .map(|(key, value)| EnvVarFact {
                key: key.clone(),
                value: value
                    .as_str()
                    .map(str::to_owned)
                    .or_else(|| scalar_to_string(value)),
                evidence: evidence(artifact, path),
            })
            .collect(),
        Value::Array(items) => items
            .iter()
            .filter_map(Value::as_str)
            .map(|entry| {
                let (key, value) = entry
                    .split_once('=')
                    .map_or((entry, None), |(key, value)| (key, Some(value)));
                EnvVarFact {
                    key: key.to_owned(),
                    value: value.map(str::to_owned),
                    evidence: evidence(artifact, path),
                }
            })
            .collect(),
        _ => Vec::new(),
    }
}

// --- Cargo manifest profile ------------------------------------------------

/// Cargo package or workspace facts extracted from `Cargo.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CargoProfile {
    /// `[package]` facts, when present.
    pub package: Option<CargoPackage>,
    /// `[workspace] members` entries.
    pub workspace_members: Vec<CargoWorkspaceMember>,
    /// `[lib]` and `[[bin]]` build targets.
    pub targets: Vec<CargoTarget>,
    /// `[features]` entries.
    pub features: Vec<CargoFeature>,
    /// Dependencies from `[dependencies]`, `[dev-dependencies]`, and `[build-dependencies]`.
    pub dependencies: Vec<CargoDependency>,
    /// Parse error when the manifest is malformed.
    pub parse_error: Option<String>,
}

/// `[package]` facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoPackage {
    /// Package name.
    pub name: Option<String>,
    /// Package version.
    pub version: Option<String>,
    /// Rust edition.
    pub edition: Option<String>,
    /// Evidence for the `[package]` table.
    pub evidence: EvidenceRef,
}

/// `[workspace] members` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoWorkspaceMember {
    /// Member path or glob.
    pub path: String,
    /// Evidence for the workspace members list.
    pub evidence: EvidenceRef,
}

/// Cargo build target kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CargoTargetKind {
    /// `[lib]` target.
    Lib,
    /// `[[bin]]` target.
    Bin,
}

/// `[lib]` or `[[bin]]` build target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoTarget {
    /// Target kind.
    pub kind: CargoTargetKind,
    /// Target name.
    pub name: Option<String>,
    /// Target source path.
    pub path: Option<String>,
    /// Evidence for the target table.
    pub evidence: EvidenceRef,
}

/// `[features]` entry.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoFeature {
    /// Feature name.
    pub name: String,
    /// Features and optional dependencies this feature enables.
    pub enables: Vec<String>,
    /// Evidence for this feature entry.
    pub evidence: EvidenceRef,
}

/// Dependency table category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CargoDependencyKind {
    /// `[dependencies]`.
    Normal,
    /// `[dev-dependencies]`.
    Dev,
    /// `[build-dependencies]`.
    Build,
}

/// Dependency entry from a dependency table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CargoDependency {
    /// Dependency crate name.
    pub name: String,
    /// Dependency table category.
    pub kind: CargoDependencyKind,
    /// Version, `path:`, `git:`, or `workspace` requirement summary.
    pub requirement: Option<String>,
    /// Evidence for this dependency entry.
    pub evidence: EvidenceRef,
}

/// Parser-backed analyzer for `Cargo.toml` manifests.
#[derive(Debug, Clone, Copy, Default)]
pub struct CargoProfileAnalyzer;

impl CargoProfileAnalyzer {
    /// Parses and analyzes a safe `Cargo.toml` artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> CargoProfile {
        if !exposable(artifact) {
            return CargoProfile::default();
        }
        match parse_value(text, StructuredFormat::Toml) {
            Ok(value) => build_cargo_profile(artifact, &value),
            Err(error) => CargoProfile {
                parse_error: Some(error),
                ..CargoProfile::default()
            },
        }
    }
}

fn build_cargo_profile(artifact: &Artifact, value: &Value) -> CargoProfile {
    let Some(root) = root_obj(value) else {
        return CargoProfile::default();
    };

    let package = map_obj(root, "package").map(|package| CargoPackage {
        name: map_str(package, "name").map(str::to_owned),
        version: map_str(package, "version").map(str::to_owned),
        edition: map_str(package, "edition").map(str::to_owned),
        evidence: evidence(artifact, "package"),
    });

    let workspace_members = map_obj(root, "workspace")
        .and_then(|workspace| map_arr(workspace, "members"))
        .map(|members| {
            members
                .iter()
                .filter_map(Value::as_str)
                .map(|member| CargoWorkspaceMember {
                    path: member.to_owned(),
                    evidence: evidence(artifact, "workspace.members"),
                })
                .collect()
        })
        .unwrap_or_default();

    let mut targets = Vec::new();
    if let Some(lib) = map_obj(root, "lib") {
        targets.push(CargoTarget {
            kind: CargoTargetKind::Lib,
            name: map_str(lib, "name").map(str::to_owned),
            path: map_str(lib, "path").map(str::to_owned),
            evidence: evidence(artifact, "lib"),
        });
    }
    if let Some(bins) = map_arr(root, "bin") {
        for (index, bin) in bins.iter().enumerate() {
            if let Some(bin) = bin.as_object() {
                targets.push(CargoTarget {
                    kind: CargoTargetKind::Bin,
                    name: map_str(bin, "name").map(str::to_owned),
                    path: map_str(bin, "path").map(str::to_owned),
                    evidence: evidence(artifact, &format!("bin[{index}]")),
                });
            }
        }
    }

    let features = map_obj(root, "features")
        .map(|features| {
            features
                .iter()
                .map(|(name, enables)| CargoFeature {
                    name: name.clone(),
                    enables: enables
                        .as_array()
                        .map(|items| {
                            items
                                .iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                    evidence: evidence(artifact, &format!("features.{name}")),
                })
                .collect()
        })
        .unwrap_or_default();

    let mut dependencies = Vec::new();
    for (table, kind) in [
        ("dependencies", CargoDependencyKind::Normal),
        ("dev-dependencies", CargoDependencyKind::Dev),
        ("build-dependencies", CargoDependencyKind::Build),
    ] {
        if let Some(deps) = map_obj(root, table) {
            for (name, spec) in deps {
                dependencies.push(CargoDependency {
                    name: name.clone(),
                    kind,
                    requirement: dependency_requirement(spec),
                    evidence: evidence(artifact, &format!("{table}.{name}")),
                });
            }
        }
    }

    CargoProfile {
        package,
        workspace_members,
        targets,
        features,
        dependencies,
        parse_error: None,
    }
}

fn dependency_requirement(spec: &Value) -> Option<String> {
    match spec {
        Value::String(version) => Some(version.clone()),
        Value::Object(table) => {
            if let Some(version) = map_str(table, "version") {
                Some(version.to_owned())
            } else if let Some(path) = map_str(table, "path") {
                Some(format!("path:{path}"))
            } else if let Some(git) = map_str(table, "git") {
                Some(format!("git:{git}"))
            } else if table.get("workspace").and_then(Value::as_bool) == Some(true) {
                Some("workspace".to_owned())
            } else {
                None
            }
        }
        _ => None,
    }
}

// --- Python project profile -------------------------------------------------

/// `pyproject.toml` project metadata extracted with PEP 621 conventions.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PyProjectProfile {
    /// `[project]` facts, when present.
    pub project: Option<PythonProject>,
    /// Parse error when the manifest is malformed.
    pub parse_error: Option<String>,
}

/// `[project]` facts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonProject {
    /// Project name.
    pub name: Option<String>,
    /// Project version.
    pub version: Option<String>,
    /// Supported Python version range.
    pub requires_python: Option<String>,
    /// PEP 508 dependency requirement strings.
    pub dependencies: Vec<PythonDependency>,
    /// Evidence for the `[project]` table.
    pub evidence: EvidenceRef,
}

/// PEP 508 dependency requirement from `[project] dependencies`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonDependency {
    /// Raw PEP 508 requirement string.
    pub requirement: String,
    /// Evidence for this requirement.
    pub evidence: EvidenceRef,
}

/// Parser-backed analyzer for `pyproject.toml` project metadata.
#[derive(Debug, Clone, Copy, Default)]
pub struct PyProjectAnalyzer;

impl PyProjectAnalyzer {
    /// Parses and analyzes a safe `pyproject.toml` artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> PyProjectProfile {
        if !exposable(artifact) {
            return PyProjectProfile::default();
        }
        match parse_value(text, StructuredFormat::Toml) {
            Ok(value) => build_pyproject_profile(artifact, &value),
            Err(error) => PyProjectProfile {
                parse_error: Some(error),
                ..PyProjectProfile::default()
            },
        }
    }
}

fn build_pyproject_profile(artifact: &Artifact, value: &Value) -> PyProjectProfile {
    let Some(root) = root_obj(value) else {
        return PyProjectProfile::default();
    };

    let pep621 = map_obj(root, "project");
    let poetry = map_obj(root, "tool").and_then(|tool| map_obj(tool, "poetry"));
    if pep621.is_none() && poetry.is_none() {
        return PyProjectProfile::default();
    }

    let mut dependencies: Vec<PythonDependency> = pep621
        .and_then(|project| map_arr(project, "dependencies"))
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(|requirement| PythonDependency {
                    requirement: requirement.to_owned(),
                    evidence: evidence(artifact, "project.dependencies"),
                })
                .collect()
        })
        .unwrap_or_default();

    // Poetry's legacy manifest shape (LIT-72): `[tool.poetry.dependencies]`
    // plus one `[tool.poetry.group.<name>.dependencies]` table per dev
    // group, each dependency a TOML key rather than a PEP 508 string. A
    // pyproject.toml declaring only this shape (no `[project]` table at
    // all) is common -- the official FastAPI template's own backend is one
    // -- so without this, LIT-44.1's manifest-declared-name matching has
    // nothing to match against and every backend import falls through to
    // Unresolved.
    if let Some(poetry) = poetry {
        if let Some(main) = map_obj(poetry, "dependencies") {
            collect_poetry_dependencies(
                artifact,
                main,
                "tool.poetry.dependencies",
                &mut dependencies,
            );
        }
        if let Some(groups) = map_obj(poetry, "group") {
            for (group_name, group_value) in groups {
                let Some(group_table) = group_value.as_object() else {
                    continue;
                };
                let Some(group_deps) = map_obj(group_table, "dependencies") else {
                    continue;
                };
                let path = format!("tool.poetry.group.{group_name}.dependencies");
                collect_poetry_dependencies(artifact, group_deps, &path, &mut dependencies);
            }
        }
    }
    // PEP 621 `[project.dependencies]` wins when both shapes declare the
    // same package -- it was collected first, so this keeps the earliest.
    let mut seen_names = std::collections::HashSet::new();
    dependencies.retain(|dependency| {
        seen_names.insert(dependency_name(&dependency.requirement).to_ascii_lowercase())
    });

    let name = pep621
        .and_then(|project| map_str(project, "name"))
        .or_else(|| poetry.and_then(|poetry| map_str(poetry, "name")))
        .map(str::to_owned);
    let version = pep621
        .and_then(|project| map_str(project, "version"))
        .or_else(|| poetry.and_then(|poetry| map_str(poetry, "version")))
        .map(str::to_owned);
    let requires_python = pep621
        .and_then(|project| map_str(project, "requires-python"))
        .map(str::to_owned);
    let evidence_path = if pep621.is_some() {
        "project"
    } else {
        "tool.poetry"
    };

    PyProjectProfile {
        project: Some(PythonProject {
            name,
            version,
            requires_python,
            dependencies,
            evidence: evidence(artifact, evidence_path),
        }),
        parse_error: None,
    }
}

/// Reads one Poetry dependency table (main or a dev group) into
/// [`PythonDependency`] entries, skipping the `python` version-constraint
/// key and path/git/url-sourced entries that name a local or VCS package
/// rather than an external one.
fn collect_poetry_dependencies(
    artifact: &Artifact,
    table: &Map<String, Value>,
    structured_path: &str,
    dependencies: &mut Vec<PythonDependency>,
) {
    for (name, value) in table {
        if name == "python" {
            continue;
        }
        let Some(requirement) = poetry_dependency_requirement(name, value) else {
            continue;
        };
        dependencies.push(PythonDependency {
            requirement,
            evidence: evidence(artifact, structured_path),
        });
    }
}

/// Builds a PEP 508-shaped requirement string (`name` immediately followed
/// by its version constraint, e.g. `tenacity^8.2.3`) from one Poetry
/// dependency value, which is either a bare version string or a table with
/// `extras`/`version`/`path`/`git` keys. `python_dependency_name` only reads
/// the leading name run, so the exact constraint syntax doesn't need to be
/// valid PEP 508 -- it only needs to start where the name ends.
fn poetry_dependency_requirement(name: &str, value: &Value) -> Option<String> {
    match value {
        Value::String(version) => Some(format!("{name}{version}")),
        Value::Object(table) => {
            if table.contains_key("path") || table.contains_key("git") || table.contains_key("url")
            {
                return None;
            }
            match table.get("version").and_then(Value::as_str) {
                Some(version) => Some(format!("{name}{version}")),
                None => Some(name.to_owned()),
            }
        }
        _ => Some(name.to_owned()),
    }
}

fn dependency_name(requirement: &str) -> &str {
    let end = requirement
        .find(|character: char| {
            !(character.is_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '.')
        })
        .unwrap_or(requirement.len());
    &requirement[..end]
}

/// `requirements.txt` package requirement.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PythonRequirement {
    /// Package name.
    pub name: String,
    /// Version specifier or marker text, when present.
    pub specifier: Option<String>,
    /// One-based source line.
    pub line: u32,
    /// Evidence for this requirement line.
    pub evidence: EvidenceRef,
}

/// `requirements.txt` analysis output.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RequirementsProfile {
    /// Package requirements in file order.
    pub requirements: Vec<PythonRequirement>,
}

/// Line-based analyzer for pip `requirements.txt` files.
#[derive(Debug, Clone, Copy, Default)]
pub struct RequirementsAnalyzer;

impl RequirementsAnalyzer {
    /// Parses and analyzes a safe `requirements.txt` artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> RequirementsProfile {
        if !exposable(artifact) {
            return RequirementsProfile::default();
        }

        let mut requirements = Vec::new();
        for (index, raw_line) in text.lines().enumerate() {
            let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            let line = raw_line.split('#').next().unwrap_or("").trim();
            if line.is_empty() || line.starts_with('-') {
                continue;
            }
            let (name, specifier) = match line.find(['=', '<', '>', '!', '~', ';']) {
                Some(split_at) => (line[..split_at].trim(), Some(line[split_at..].trim())),
                None => (line, None),
            };
            if name.is_empty() {
                continue;
            }
            requirements.push(PythonRequirement {
                name: name.to_owned(),
                specifier: specifier
                    .filter(|specifier| !specifier.is_empty())
                    .map(str::to_owned),
                line: line_number,
                evidence: line_evidence(artifact, line_number),
            });
        }

        RequirementsProfile { requirements }
    }
}

// --- Docker Compose profile --------------------------------------------------

/// Docker Compose service facts extracted from a compose file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ComposeProfile {
    /// Services in declaration order.
    pub services: Vec<ComposeService>,
    /// Parse error when the compose file is malformed.
    pub parse_error: Option<String>,
}

/// One Docker Compose service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposeService {
    /// Service name.
    pub name: String,
    /// Image reference, when set directly.
    pub image: Option<String>,
    /// Build context path, from shorthand or long `build` form.
    pub build_context: Option<String>,
    /// Build Dockerfile path, from the long `build` form.
    pub build_dockerfile: Option<String>,
    /// Published ports.
    pub ports: Vec<ComposePort>,
    /// Volume mounts, as raw compose entries.
    pub volumes: Vec<String>,
    /// Environment variables.
    pub environment: Vec<EnvVarFact>,
    /// Service names this service depends on.
    pub depends_on: Vec<String>,
    /// Evidence for this service table.
    pub evidence: EvidenceRef,
}

/// One Docker Compose port mapping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ComposePort {
    /// Host-published port, when the mapping specifies one.
    pub published: Option<String>,
    /// Container-side port.
    pub target: String,
    /// Evidence for the ports list.
    pub evidence: EvidenceRef,
}

/// Parser-backed analyzer for Docker Compose files.
#[derive(Debug, Clone, Copy, Default)]
pub struct ComposeProfileAnalyzer;

impl ComposeProfileAnalyzer {
    /// Parses and analyzes a safe Docker Compose artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> ComposeProfile {
        if !exposable(artifact) {
            return ComposeProfile::default();
        }
        match parse_value(text, StructuredFormat::Yaml) {
            Ok(value) => build_compose_profile(artifact, &value),
            Err(error) => ComposeProfile {
                parse_error: Some(error),
                ..ComposeProfile::default()
            },
        }
    }
}

fn build_compose_profile(artifact: &Artifact, value: &Value) -> ComposeProfile {
    let Some(root) = root_obj(value) else {
        return ComposeProfile::default();
    };
    let Some(services) = map_obj(root, "services") else {
        return ComposeProfile::default();
    };

    let services = services
        .iter()
        .filter_map(|(name, service)| {
            service
                .as_object()
                .map(|service| build_compose_service(artifact, name, service))
        })
        .collect();

    ComposeProfile {
        services,
        parse_error: None,
    }
}

fn build_compose_service(
    artifact: &Artifact,
    name: &str,
    service: &Map<String, Value>,
) -> ComposeService {
    let path_prefix = format!("services.{name}");
    let build_context = map_obj(service, "build")
        .and_then(|build| map_str(build, "context"))
        .map(str::to_owned)
        .or_else(|| map_str(service, "build").map(str::to_owned));
    let build_dockerfile = map_obj(service, "build")
        .and_then(|build| map_str(build, "dockerfile"))
        .map(str::to_owned);

    ComposeService {
        name: name.to_owned(),
        image: map_str(service, "image").map(str::to_owned),
        build_context,
        build_dockerfile,
        ports: map_arr(service, "ports")
            .map(|ports| {
                ports
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|port| build_compose_port(artifact, &path_prefix, port))
                    .collect()
            })
            .unwrap_or_default(),
        volumes: map_arr(service, "volumes")
            .map(|volumes| {
                volumes
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        environment: service
            .get("environment")
            .map(|environment| {
                object_environment(artifact, &format!("{path_prefix}.environment"), environment)
            })
            .unwrap_or_default(),
        depends_on: compose_depends_on(service),
        evidence: evidence(artifact, &path_prefix),
    }
}

fn build_compose_port(artifact: &Artifact, path_prefix: &str, port: &str) -> ComposePort {
    let (published, target) = match port.rsplit_once(':') {
        Some((published, target)) => (Some(published.to_owned()), target.to_owned()),
        None => (None, port.to_owned()),
    };
    ComposePort {
        published,
        target,
        evidence: evidence(artifact, &format!("{path_prefix}.ports")),
    }
}

fn compose_depends_on(service: &Map<String, Value>) -> Vec<String> {
    match service.get("depends_on") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::Object(map)) => map.keys().cloned().collect(),
        _ => Vec::new(),
    }
}

// --- GitHub Actions workflow profile -----------------------------------------

/// GitHub Actions workflow facts extracted from a workflow file.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ActionsProfile {
    /// Workflow display name.
    pub name: Option<String>,
    /// Jobs in declaration order.
    pub jobs: Vec<ActionsJob>,
    /// Parse error when the workflow file is malformed.
    pub parse_error: Option<String>,
}

/// One GitHub Actions job.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionsJob {
    /// Job ID as declared under `jobs`.
    pub id: String,
    /// `runs-on` runner summary.
    pub runs_on: Option<String>,
    /// Steps in declaration order.
    pub steps: Vec<ActionsStep>,
    /// Evidence for this job table.
    pub evidence: EvidenceRef,
}

/// One GitHub Actions workflow step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionsStep {
    /// Step display name.
    pub name: Option<String>,
    /// Referenced reusable action, for `uses` steps.
    pub uses: Option<String>,
    /// Shell command, for `run` steps.
    pub run: Option<String>,
    /// Step-level environment variables.
    pub env: Vec<EnvVarFact>,
    /// Container image build or publish hint, when detected.
    pub hint: Option<ActionsStepHint>,
    /// Evidence for this step.
    pub evidence: EvidenceRef,
}

/// Container image build or publish hint for a step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionsStepHint {
    /// Step appears to build a container image.
    Build {
        /// Image reference, when it could be extracted from a `-t`/`--tag` flag.
        image: Option<String>,
    },
    /// Step appears to publish a container image.
    Publish {
        /// Image reference, when it could be extracted from the command.
        image: Option<String>,
    },
}

/// Parser-backed analyzer for GitHub Actions workflow files.
#[derive(Debug, Clone, Copy, Default)]
pub struct ActionsProfileAnalyzer;

impl ActionsProfileAnalyzer {
    /// Parses and analyzes a safe GitHub Actions workflow artifact.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> ActionsProfile {
        if !exposable(artifact) {
            return ActionsProfile::default();
        }
        match parse_value(text, StructuredFormat::Yaml) {
            Ok(value) => build_actions_profile(artifact, &value),
            Err(error) => ActionsProfile {
                parse_error: Some(error),
                ..ActionsProfile::default()
            },
        }
    }
}

fn build_actions_profile(artifact: &Artifact, value: &Value) -> ActionsProfile {
    let Some(root) = root_obj(value) else {
        return ActionsProfile::default();
    };

    let name = map_str(root, "name").map(str::to_owned);
    let jobs = map_obj(root, "jobs")
        .map(|jobs| {
            jobs.iter()
                .filter_map(|(id, job)| {
                    job.as_object()
                        .map(|job| build_actions_job(artifact, id, job))
                })
                .collect()
        })
        .unwrap_or_default();

    ActionsProfile {
        name,
        jobs,
        parse_error: None,
    }
}

fn build_actions_job(artifact: &Artifact, id: &str, job: &Map<String, Value>) -> ActionsJob {
    let path_prefix = format!("jobs.{id}");
    let steps = map_arr(job, "steps")
        .map(|steps| {
            steps
                .iter()
                .enumerate()
                .filter_map(|(index, step)| {
                    step.as_object()
                        .map(|step| build_actions_step(artifact, &path_prefix, index, step))
                })
                .collect()
        })
        .unwrap_or_default();

    ActionsJob {
        id: id.to_owned(),
        runs_on: runs_on_summary(job.get("runs-on")),
        steps,
        evidence: evidence(artifact, &path_prefix),
    }
}

fn runs_on_summary(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(runner)) => Some(runner.clone()),
        Some(Value::Array(items)) => {
            let runners: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            if runners.is_empty() {
                None
            } else {
                Some(runners.join(", "))
            }
        }
        _ => None,
    }
}

fn build_actions_step(
    artifact: &Artifact,
    path_prefix: &str,
    index: usize,
    step: &Map<String, Value>,
) -> ActionsStep {
    let path = format!("{path_prefix}.steps[{index}]");
    let run = map_str(step, "run").map(str::to_owned);
    let uses = map_str(step, "uses").map(str::to_owned);

    ActionsStep {
        name: map_str(step, "name").map(str::to_owned),
        uses: uses.clone(),
        run: run.clone(),
        env: step
            .get("env")
            .map(|env| object_environment(artifact, &format!("{path}.env"), env))
            .unwrap_or_default(),
        hint: actions_hint(run.as_deref(), uses.as_deref()),
        evidence: evidence(artifact, &path),
    }
}

// ponytail: heuristic whitespace tokenizing, so tags containing GitHub Actions
// `${{ expr with spaces }}` extract incompletely. Upgrade to shell-aware
// tokenizing if downstream consumers need exact image references.
fn actions_hint(run: Option<&str>, uses: Option<&str>) -> Option<ActionsStepHint> {
    if let Some(run) = run {
        if run.contains("docker push") {
            return Some(ActionsStepHint::Publish {
                image: extract_after_token(run, "push"),
            });
        }
        if run.contains("docker build") {
            return Some(ActionsStepHint::Build {
                image: extract_tag_flag(run),
            });
        }
    }
    if let Some(uses) = uses
        && uses.starts_with("docker/build-push-action")
    {
        return Some(ActionsStepHint::Publish { image: None });
    }
    None
}

fn extract_tag_flag(run: &str) -> Option<String> {
    let mut tokens = run.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == "-t" || token == "--tag" {
            return tokens.next().map(str::to_owned);
        }
    }
    None
}

fn extract_after_token(run: &str, after: &str) -> Option<String> {
    let mut tokens = run.split_whitespace();
    while let Some(token) = tokens.next() {
        if token == after {
            return tokens
                .next()
                .map(str::to_owned)
                .filter(|value| !value.starts_with('-'));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        ActionsProfileAnalyzer, ActionsStepHint, CargoDependencyKind, CargoProfileAnalyzer,
        CargoTargetKind, ComposeProfileAnalyzer, PyProjectAnalyzer, RequirementsAnalyzer,
        dependency_name,
    };
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    fn fixture_artifact(path: &str) -> Result<(Artifact, String), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions {
            include_hidden_directories: true,
            ..WalkOptions::default()
        })
        .walk(&root)?;
        let not_found = std::io::ErrorKind::NotFound;
        let artifact = artifacts
            .into_iter()
            .find(|artifact| artifact.path.as_str() == path)
            .ok_or(std::io::Error::new(not_found, path.to_owned()))?;
        let text = fs::read_to_string(root.join(path))?;
        Ok((artifact, text))
    }

    fn profile_artifact(
        path: &str,
        category: ArtifactCategory,
    ) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            category,
            SupportTier::StructuredFormat,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn cargo_profile_extracts_manifest_facts_from_fixture() -> Result<(), Box<dyn std::error::Error>>
    {
        let (artifact, text) = fixture_artifact("rust/Cargo.toml")?;
        let profile = CargoProfileAnalyzer.analyze(&artifact, &text);

        let package = profile.package.ok_or("package facts")?;
        assert_eq!(package.name.as_deref(), Some("fixture-worker"));
        assert_eq!(package.version.as_deref(), Some("0.1.0"));
        assert_eq!(package.edition.as_deref(), Some("2024"));

        let lib = profile
            .targets
            .iter()
            .find(|target| target.kind == CargoTargetKind::Lib)
            .ok_or("lib target")?;
        assert_eq!(lib.path.as_deref(), Some("src/lib.rs"));
        let bin = profile
            .targets
            .iter()
            .find(|target| target.kind == CargoTargetKind::Bin)
            .ok_or("bin target")?;
        assert_eq!(bin.name.as_deref(), Some("worker"));
        assert_eq!(bin.path.as_deref(), Some("src/bin/worker.rs"));

        let default_feature = profile
            .features
            .iter()
            .find(|feature| feature.name == "default")
            .ok_or("default feature")?;
        assert_eq!(default_feature.enables, vec!["serde-support".to_owned()]);

        let dependency = profile
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "anyhow")
            .ok_or("anyhow dependency")?;
        assert_eq!(dependency.kind, CargoDependencyKind::Normal);
        assert_eq!(dependency.requirement.as_deref(), Some("1"));

        Ok(())
    }

    #[test]
    fn cargo_profile_extracts_workspace_and_dependency_sources()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = profile_artifact("Cargo.toml", ArtifactCategory::PackageManifest)?;
        let text = "\
[workspace]
members = [\"crates/*\"]

[dev-dependencies]
tempfile = { path = \"../tempfile\" }

[build-dependencies]
cc = { git = \"https://example.test/cc.git\" }

[dependencies]
serde = { workspace = true }
";
        let profile = CargoProfileAnalyzer.analyze(&artifact, text);

        assert_eq!(profile.workspace_members[0].path, "crates/*");
        let dev = profile
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "tempfile")
            .ok_or("tempfile dependency")?;
        assert_eq!(dev.kind, CargoDependencyKind::Dev);
        assert_eq!(dev.requirement.as_deref(), Some("path:../tempfile"));
        let build = profile
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "cc")
            .ok_or("cc dependency")?;
        assert_eq!(build.kind, CargoDependencyKind::Build);
        assert_eq!(
            build.requirement.as_deref(),
            Some("git:https://example.test/cc.git")
        );
        let workspace_dep = profile
            .dependencies
            .iter()
            .find(|dependency| dependency.name == "serde")
            .ok_or("serde dependency")?;
        assert_eq!(workspace_dep.requirement.as_deref(), Some("workspace"));

        Ok(())
    }

    #[test]
    fn cargo_profile_respects_policy_and_reports_parse_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let never = profile_artifact("Cargo.toml", ArtifactCategory::PackageManifest)?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let artifact = profile_artifact("Cargo.toml", ArtifactCategory::PackageManifest)?;

        assert!(
            CargoProfileAnalyzer
                .analyze(&never, "[package]\nname = \"x\"")
                .package
                .is_none()
        );
        assert!(
            CargoProfileAnalyzer
                .analyze(&artifact, "not = [valid")
                .parse_error
                .is_some()
        );

        Ok(())
    }

    #[test]
    fn pyproject_profile_extracts_fixture_metadata() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("pyproject.toml")?;
        let profile = PyProjectAnalyzer.analyze(&artifact, &text);

        let project = profile.project.ok_or("project facts")?;
        assert_eq!(project.name.as_deref(), Some("polyglot-fixture"));
        assert_eq!(project.version.as_deref(), Some("0.1.0"));
        assert_eq!(project.requires_python.as_deref(), Some(">=3.12"));
        assert_eq!(project.dependencies[0].requirement, "pydantic>=2");

        Ok(())
    }

    #[test]
    fn pyproject_profile_reads_poetry_only_manifest() -> Result<(), Box<dyn std::error::Error>> {
        let artifact = profile_artifact("pyproject.toml", ArtifactCategory::PackageManifest)?;
        let text = "\
[tool.poetry]
name = \"app\"
version = \"0.1.0\"

[tool.poetry.dependencies]
python = \"^3.10\"
fastapi = \"^0.109.1\"
uvicorn = {extras = [\"standard\"], version = \"^0.24.0.post1\"}
shared = {path = \"../shared\"}

[tool.poetry.group.dev.dependencies]
pytest = \"^7.4.3\"
";
        let profile = PyProjectAnalyzer.analyze(&artifact, text);
        let project = profile.project.ok_or("project facts")?;

        assert_eq!(project.name.as_deref(), Some("app"));
        assert_eq!(project.version.as_deref(), Some("0.1.0"));
        let names: std::collections::BTreeSet<_> = project
            .dependencies
            .iter()
            .map(|dependency| dependency_name(&dependency.requirement))
            .collect();
        assert_eq!(
            names,
            std::collections::BTreeSet::from(["fastapi", "uvicorn", "pytest"])
        );
        assert!(
            !project
                .dependencies
                .iter()
                .any(|dependency| dependency.requirement.starts_with("python"))
        );
        assert!(
            !project
                .dependencies
                .iter()
                .any(|dependency| dependency_name(&dependency.requirement) == "shared"),
            "a path-sourced dependency names a local package, not an external one"
        );

        Ok(())
    }

    #[test]
    fn pyproject_profile_prefers_pep621_dependency_over_poetry_duplicate()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = profile_artifact("pyproject.toml", ArtifactCategory::PackageManifest)?;
        let text = "\
[project]
name = \"app\"
dependencies = [\"pydantic>=2\"]

[tool.poetry.dependencies]
python = \"^3.10\"
pydantic = \"^1.10\"
";
        let profile = PyProjectAnalyzer.analyze(&artifact, text);
        let project = profile.project.ok_or("project facts")?;

        let pydantic: Vec<_> = project
            .dependencies
            .iter()
            .filter(|dependency| dependency_name(&dependency.requirement) == "pydantic")
            .collect();
        assert_eq!(pydantic.len(), 1);
        assert_eq!(pydantic[0].requirement, "pydantic>=2");

        Ok(())
    }

    #[test]
    fn pyproject_profile_without_project_or_poetry_table_has_no_project_facts()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = profile_artifact("pyproject.toml", ArtifactCategory::PackageManifest)?;
        let profile = PyProjectAnalyzer.analyze(&artifact, "[build-system]\nrequires = []\n");
        assert!(profile.project.is_none());
        Ok(())
    }

    #[test]
    fn pyproject_profile_respects_policy_and_reports_parse_errors()
    -> Result<(), Box<dyn std::error::Error>> {
        let never = profile_artifact("pyproject.toml", ArtifactCategory::PackageManifest)?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let artifact = profile_artifact("pyproject.toml", ArtifactCategory::PackageManifest)?;

        assert!(
            PyProjectAnalyzer
                .analyze(&never, "[project]\nname = \"x\"")
                .project
                .is_none()
        );
        assert!(
            PyProjectAnalyzer
                .analyze(&artifact, "not = [valid")
                .parse_error
                .is_some()
        );

        Ok(())
    }

    #[test]
    fn requirements_profile_extracts_fixture_requirements() -> Result<(), Box<dyn std::error::Error>>
    {
        let (artifact, text) = fixture_artifact("requirements.txt")?;
        let profile = RequirementsAnalyzer.analyze(&artifact, &text);

        assert_eq!(profile.requirements[0].name, "pydantic");
        assert_eq!(profile.requirements[0].specifier.as_deref(), Some(">=2"));
        assert_eq!(profile.requirements[0].line, 1);
        assert_eq!(profile.requirements[1].name, "pytest");
        assert_eq!(profile.requirements[1].specifier.as_deref(), Some(">=8"));

        Ok(())
    }

    #[test]
    fn requirements_profile_skips_comments_blanks_and_directives_and_handles_extras()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = profile_artifact("requirements.txt", ArtifactCategory::PackageManifest)?;
        let text = "\
# a comment

-r base.txt
requests[security]==2.31.0  # pinned
click
";
        let profile = RequirementsAnalyzer.analyze(&artifact, text);

        assert_eq!(profile.requirements.len(), 2);
        assert_eq!(profile.requirements[0].name, "requests[security]");
        assert_eq!(
            profile.requirements[0].specifier.as_deref(),
            Some("==2.31.0")
        );
        assert_eq!(profile.requirements[1].name, "click");
        assert_eq!(profile.requirements[1].specifier, None);
        assert!(
            RequirementsAnalyzer
                .analyze(
                    &artifact
                        .clone()
                        .with_model_policy(ModelExposurePolicy::Never)
                        .with_text_status(TextStatus::UnsafeText, None),
                    text,
                )
                .requirements
                .is_empty()
        );

        Ok(())
    }

    #[test]
    fn compose_profile_extracts_fixture_services() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact("docker-compose.yml")?;
        let profile = ComposeProfileAnalyzer.analyze(&artifact, &text);

        let api = profile
            .services
            .iter()
            .find(|service| service.name == "api")
            .ok_or("api service")?;
        assert_eq!(api.build_context.as_deref(), Some("."));
        assert_eq!(api.build_dockerfile.as_deref(), Some("Dockerfile"));
        assert_eq!(
            api.image.as_deref(),
            Some("ghcr.io/example/route-api:${VERSION:-dev}")
        );
        assert_eq!(api.ports[0].published.as_deref(), Some("8080"));
        assert_eq!(api.ports[0].target, "8080");
        assert!(
            api.environment
                .iter()
                .any(|env| env.key == "RIDGELINE_WORKER"
                    && env.value.as_deref() == Some("/usr/local/bin/worker"))
        );
        assert_eq!(api.volumes, vec!["./data:/data:ro".to_owned()]);
        assert_eq!(api.depends_on, vec!["web".to_owned()]);

        let web = profile
            .services
            .iter()
            .find(|service| service.name == "web")
            .ok_or("web service")?;
        assert_eq!(web.image.as_deref(), Some("node:24-alpine"));
        assert!(web.build_context.is_none());

        Ok(())
    }

    #[test]
    fn compose_profile_supports_list_environment_and_map_depends_on_and_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact =
            profile_artifact("docker-compose.yml", ArtifactCategory::ContainerDefinition)?;
        let text = "\
services:
  worker:
    image: example/worker
    environment:
      - QUEUE_URL=redis://cache:6379
      - DEBUG
    depends_on:
      cache:
        condition: service_healthy
";
        let profile = ComposeProfileAnalyzer.analyze(&artifact, text);
        let worker = &profile.services[0];

        assert!(worker.environment.iter().any(
            |env| env.key == "QUEUE_URL" && env.value.as_deref() == Some("redis://cache:6379")
        ));
        assert!(
            worker
                .environment
                .iter()
                .any(|env| env.key == "DEBUG" && env.value.is_none())
        );
        assert_eq!(worker.depends_on, vec!["cache".to_owned()]);
        assert!(
            ComposeProfileAnalyzer
                .analyze(&artifact, "services: [")
                .parse_error
                .is_some()
        );

        Ok(())
    }

    #[test]
    fn actions_profile_extracts_fixture_jobs() -> Result<(), Box<dyn std::error::Error>> {
        let (artifact, text) = fixture_artifact(".github/workflows/ci.yml")?;
        let profile = ActionsProfileAnalyzer.analyze(&artifact, &text);

        assert_eq!(profile.name.as_deref(), Some("Fixture CI"));
        let job = &profile.jobs[0];
        assert_eq!(job.id, "checks");
        assert_eq!(job.runs_on.as_deref(), Some("ubuntu-latest"));
        assert!(
            job.steps
                .iter()
                .any(|step| step.uses.as_deref() == Some("actions/checkout@v4"))
        );
        let build_step = job
            .steps
            .iter()
            .find(|step| matches!(step.hint, Some(ActionsStepHint::Build { .. })))
            .ok_or("docker build step")?;
        assert!(
            build_step
                .run
                .as_deref()
                .unwrap_or_default()
                .contains("docker build")
        );

        Ok(())
    }

    #[test]
    fn actions_profile_detects_publish_hint_env_and_policy()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = profile_artifact(
            ".github/workflows/ci.yml",
            ArtifactCategory::ContinuousIntegration,
        )?;
        let text = "\
name: Release
jobs:
  publish:
    runs-on: [self-hosted, linux]
    steps:
      - name: Build
        run: docker build -t myorg/app:1.0 .
      - name: Push
        run: docker push myorg/app:1.0
        env:
          REGISTRY: ghcr.io
      - uses: docker/build-push-action@v5
";
        let profile = ActionsProfileAnalyzer.analyze(&artifact, text);
        let job = &profile.jobs[0];

        assert_eq!(job.runs_on.as_deref(), Some("self-hosted, linux"));
        assert_eq!(
            job.steps[0].hint,
            Some(ActionsStepHint::Build {
                image: Some("myorg/app:1.0".to_owned())
            })
        );
        assert_eq!(
            job.steps[1].hint,
            Some(ActionsStepHint::Publish {
                image: Some("myorg/app:1.0".to_owned())
            })
        );
        assert!(
            job.steps[1]
                .env
                .iter()
                .any(|env| env.key == "REGISTRY" && env.value.as_deref() == Some("ghcr.io"))
        );
        assert_eq!(
            job.steps[2].hint,
            Some(ActionsStepHint::Publish { image: None })
        );
        assert!(
            ActionsProfileAnalyzer
                .analyze(
                    &artifact
                        .clone()
                        .with_model_policy(ModelExposurePolicy::Never)
                        .with_text_status(TextStatus::UnsafeText, None),
                    text,
                )
                .jobs
                .is_empty()
        );
        assert!(
            ActionsProfileAnalyzer
                .analyze(&artifact, "jobs: [")
                .parse_error
                .is_some()
        );

        Ok(())
    }
}
