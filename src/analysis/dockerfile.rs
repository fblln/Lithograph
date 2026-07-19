//! Dockerfile instruction and stage analysis.

use crate::domain::{
    Artifact, ArtifactId, EvidenceRef, ModelExposurePolicy, SourceSpan, TextStatus,
};
use serde::{Deserialize, Serialize};

/// Parsed Dockerfile facts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct DockerfileAnalysis {
    /// Recognized Dockerfile instructions in source order.
    pub instructions: Vec<DockerInstruction>,
    /// Build stages introduced by `FROM`.
    pub stages: Vec<DockerStage>,
    /// `COPY` instructions with source and destination paths.
    pub copies: Vec<DockerCopy>,
    /// `ENV` assignments.
    pub env: Vec<DockerEnv>,
    /// `ARG` assignments.
    pub args: Vec<DockerEnv>,
    /// `EXPOSE` ports.
    pub ports: Vec<DockerPort>,
    /// `RUN`, `CMD`, and `ENTRYPOINT` commands.
    pub commands: Vec<DockerCommand>,
    /// `LABEL` key/value metadata.
    pub labels: Vec<DockerEnv>,
    /// `WORKDIR` values, in source order (later ones override earlier ones
    /// at the same build stage, but both are kept as evidence).
    pub workdirs: Vec<DockerSingleValue>,
    /// `USER` values.
    pub users: Vec<DockerSingleValue>,
    /// `VOLUME` mount paths, one entry per path (space-separated or JSON
    /// array form both flatten to one entry per path).
    pub volumes: Vec<DockerSingleValue>,
    /// `STOPSIGNAL` values.
    pub stop_signals: Vec<DockerSingleValue>,
}

/// Dockerfile instruction kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DockerInstructionKind {
    /// `FROM`.
    From,
    /// `COPY`.
    Copy,
    /// `RUN`.
    Run,
    /// `ENV`.
    Env,
    /// `ARG`.
    Arg,
    /// `EXPOSE`.
    Expose,
    /// `CMD`.
    Cmd,
    /// `ENTRYPOINT`.
    Entrypoint,
    /// `HEALTHCHECK`.
    HealthCheck,
    /// `LABEL`.
    Label,
    /// `USER`.
    User,
    /// `WORKDIR`.
    Workdir,
    /// `ONBUILD`.
    Onbuild,
    /// `SHELL`.
    Shell,
    /// `STOPSIGNAL`.
    StopSignal,
    /// `VOLUME`.
    Volume,
}

/// Recognized Dockerfile instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerInstruction {
    /// Instruction kind.
    pub kind: DockerInstructionKind,
    /// Raw instruction payload after the keyword.
    pub value: String,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the instruction line.
    pub evidence: EvidenceRef,
}

/// Docker build stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerStage {
    /// Zero-based stage index.
    pub index: usize,
    /// Base image reference, preserved exactly after option parsing.
    pub image: String,
    /// Optional stage alias from `AS`.
    pub alias: Option<String>,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the `FROM`.
    pub evidence: EvidenceRef,
}

/// Docker `COPY` instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerCopy {
    /// Optional `--from` stage or image reference.
    pub from: Option<String>,
    /// Source paths.
    pub sources: Vec<String>,
    /// Destination path.
    pub destination: String,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the `COPY`.
    pub evidence: EvidenceRef,
}

/// Docker `ENV` or `ARG` key/value record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerEnv {
    /// Variable name.
    pub key: String,
    /// Optional assigned value.
    pub value: Option<String>,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the assignment.
    pub evidence: EvidenceRef,
}

/// Docker exposed port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerPort {
    /// Port number or token.
    pub port: String,
    /// Optional protocol suffix such as `tcp` or `udp`.
    pub protocol: Option<String>,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the `EXPOSE`.
    pub evidence: EvidenceRef,
}

/// A single-token instruction value, e.g. one `WORKDIR`/`USER`/`VOLUME`/
/// `STOPSIGNAL` argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerSingleValue {
    /// The value, as written (JSON-array quoting stripped for `VOLUME`).
    pub value: String,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the instruction.
    pub evidence: EvidenceRef,
}

/// Docker command category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum DockerCommandKind {
    /// Build-time `RUN`.
    Run,
    /// Runtime `CMD`.
    Cmd,
    /// Runtime `ENTRYPOINT`.
    Entrypoint,
}

/// Docker command instruction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DockerCommand {
    /// Command category.
    pub kind: DockerCommandKind,
    /// Raw command payload.
    pub command: String,
    /// True when JSON exec form is used.
    pub exec_form: bool,
    /// One-based source line.
    pub line: u32,
    /// Source evidence for the command.
    pub evidence: EvidenceRef,
}

/// Dockerfile analyzer.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DockerfileAnalyzer;

#[derive(Debug, Clone)]
struct ParsedInstruction<'a> {
    kind: DockerInstructionKind,
    value: &'a str,
    line: u32,
    evidence: EvidenceRef,
}

impl DockerfileAnalyzer {
    /// Parses recognized Dockerfile instructions from safe text.
    pub(crate) fn analyze(&self, artifact: &Artifact, text: &str) -> DockerfileAnalysis {
        if artifact.text_status != TextStatus::Text
            || artifact.model_policy == ModelExposurePolicy::Never
        {
            return DockerfileAnalysis::default();
        }

        let mut analysis = DockerfileAnalysis::default();
        for (index, line) in text.lines().enumerate() {
            let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
            parse_line(&mut analysis, artifact, line, line_number);
        }
        analysis
    }
}

fn parse_line(
    analysis: &mut DockerfileAnalysis,
    artifact: &Artifact,
    line: &str,
    line_number: u32,
) {
    let Some(instruction) = parse_instruction(artifact, line, line_number) else {
        return;
    };

    analysis.instructions.push(DockerInstruction {
        kind: instruction.kind,
        value: instruction.value.to_owned(),
        line: instruction.line,
        evidence: instruction.evidence.clone(),
    });

    match instruction.kind {
        DockerInstructionKind::From => add_stage(analysis, &instruction),
        DockerInstructionKind::Copy => add_copy(analysis, &instruction),
        DockerInstructionKind::Run => {
            add_command(analysis, &instruction, DockerCommandKind::Run);
        }
        DockerInstructionKind::Cmd => {
            add_command(analysis, &instruction, DockerCommandKind::Cmd);
        }
        DockerInstructionKind::Entrypoint => {
            add_command(analysis, &instruction, DockerCommandKind::Entrypoint);
        }
        DockerInstructionKind::Env => add_assignment(&mut analysis.env, &instruction),
        DockerInstructionKind::Arg => add_assignment(&mut analysis.args, &instruction),
        DockerInstructionKind::Expose => add_ports(analysis, &instruction),
        DockerInstructionKind::Label => add_assignment(&mut analysis.labels, &instruction),
        DockerInstructionKind::Workdir => add_single_value(&mut analysis.workdirs, &instruction),
        DockerInstructionKind::User => add_single_value(&mut analysis.users, &instruction),
        DockerInstructionKind::StopSignal => {
            add_single_value(&mut analysis.stop_signals, &instruction)
        }
        DockerInstructionKind::Volume => add_volumes(analysis, &instruction),
        // `HEALTHCHECK`, `ONBUILD`, and `SHELL` have no dedicated structured
        // model: `HEALTHCHECK` embeds an arbitrary options+CMD/NONE grammar,
        // `ONBUILD` wraps an arbitrary other instruction, and `SHELL` is a
        // low-signal exec-array config directive. All three are still
        // recognized above (no longer silently dropped) and their raw value
        // is captured in `analysis.instructions`.
        DockerInstructionKind::HealthCheck
        | DockerInstructionKind::Onbuild
        | DockerInstructionKind::Shell => {}
    }
}

fn parse_instruction<'a>(
    artifact: &Artifact,
    line: &'a str,
    line_number: u32,
) -> Option<ParsedInstruction<'a>> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let keyword = parts.next().unwrap_or("").to_ascii_uppercase();
    let value = parts.next().unwrap_or("").trim();
    instruction_kind(&keyword).map(|kind| ParsedInstruction {
        kind,
        value,
        line: line_number,
        evidence: evidence(artifact, line_number),
    })
}

fn instruction_kind(keyword: &str) -> Option<DockerInstructionKind> {
    match keyword {
        "FROM" => Some(DockerInstructionKind::From),
        "COPY" => Some(DockerInstructionKind::Copy),
        "RUN" => Some(DockerInstructionKind::Run),
        "ENV" => Some(DockerInstructionKind::Env),
        "ARG" => Some(DockerInstructionKind::Arg),
        "EXPOSE" => Some(DockerInstructionKind::Expose),
        "CMD" => Some(DockerInstructionKind::Cmd),
        "ENTRYPOINT" => Some(DockerInstructionKind::Entrypoint),
        "HEALTHCHECK" => Some(DockerInstructionKind::HealthCheck),
        "LABEL" => Some(DockerInstructionKind::Label),
        "USER" => Some(DockerInstructionKind::User),
        "WORKDIR" => Some(DockerInstructionKind::Workdir),
        "ONBUILD" => Some(DockerInstructionKind::Onbuild),
        "SHELL" => Some(DockerInstructionKind::Shell),
        "STOPSIGNAL" => Some(DockerInstructionKind::StopSignal),
        "VOLUME" => Some(DockerInstructionKind::Volume),
        _ => None,
    }
}

fn add_stage(analysis: &mut DockerfileAnalysis, instruction: &ParsedInstruction<'_>) {
    let tokens = non_option_tokens(instruction.value);
    if tokens.is_empty() {
        return;
    }
    let image = tokens[0].clone();
    let alias = tokens
        .windows(2)
        .find(|window| window[0].eq_ignore_ascii_case("AS"))
        .map(|window| window[1].clone());
    analysis.stages.push(DockerStage {
        index: analysis.stages.len(),
        image,
        alias,
        line: instruction.line,
        evidence: instruction.evidence.clone(),
    });
}

fn non_option_tokens(value: &str) -> Vec<String> {
    value
        .split_whitespace()
        .filter(|token| !token.starts_with("--"))
        .map(str::to_owned)
        .collect()
}

fn add_copy(analysis: &mut DockerfileAnalysis, instruction: &ParsedInstruction<'_>) {
    let mut from = None;
    let mut paths = Vec::new();
    let mut tokens = instruction.value.split_whitespace();
    while let Some(token) = tokens.next() {
        if let Some(stage) = token.strip_prefix("--from=") {
            from = Some(stage.to_owned());
        } else if token == "--from" {
            from = tokens.next().map(str::to_owned);
        } else if !token.starts_with("--") {
            paths.push(token.to_owned());
        }
    }
    if paths.len() < 2 {
        return;
    }
    let destination = paths.pop().unwrap_or_default();
    analysis.copies.push(DockerCopy {
        from,
        sources: paths,
        destination,
        line: instruction.line,
        evidence: instruction.evidence.clone(),
    });
}

fn add_assignment(output: &mut Vec<DockerEnv>, instruction: &ParsedInstruction<'_>) {
    for assignment in split_assignments(instruction.value) {
        let (key, assigned) = assignment
            .split_once('=')
            .map_or((assignment.as_str(), None), |(key, value)| {
                (key, Some(value))
            });
        output.push(DockerEnv {
            key: key.to_owned(),
            value: assigned.map(str::to_owned),
            line: instruction.line,
            evidence: instruction.evidence.clone(),
        });
    }
}

fn split_assignments(value: &str) -> Vec<String> {
    if value.contains('=') {
        return value.split_whitespace().map(str::to_owned).collect();
    }
    let mut tokens = value.splitn(2, char::is_whitespace);
    tokens
        .next()
        .map(|key| {
            let assigned = tokens.next().map(str::trim).filter(|part| !part.is_empty());
            match assigned {
                Some(value) => format!("{key}={value}"),
                None => key.to_owned(),
            }
        })
        .into_iter()
        .collect()
}

fn add_ports(analysis: &mut DockerfileAnalysis, instruction: &ParsedInstruction<'_>) {
    for token in instruction.value.split_whitespace() {
        let (port, protocol) = token
            .split_once('/')
            .map_or((token, None), |(port, protocol)| (port, Some(protocol)));
        analysis.ports.push(DockerPort {
            port: port.to_owned(),
            protocol: protocol.map(str::to_owned),
            line: instruction.line,
            evidence: instruction.evidence.clone(),
        });
    }
}

fn add_single_value(output: &mut Vec<DockerSingleValue>, instruction: &ParsedInstruction<'_>) {
    if instruction.value.is_empty() {
        return;
    }
    output.push(DockerSingleValue {
        value: instruction.value.to_owned(),
        line: instruction.line,
        evidence: instruction.evidence.clone(),
    });
}

/// `VOLUME` accepts either a JSON array (`["/data", "/logs"]`, exec-array
/// style) or bare space-separated paths (`/data /logs`); both flatten to one
/// [`DockerSingleValue`] per path, with JSON array punctuation/quoting
/// stripped.
fn add_volumes(analysis: &mut DockerfileAnalysis, instruction: &ParsedInstruction<'_>) {
    let trimmed = instruction.value.trim();
    let is_json_array = trimmed.starts_with('[') && trimmed.ends_with(']');
    let paths: Vec<String> = if is_json_array {
        trimmed[1..trimmed.len() - 1]
            .split(',')
            .map(|token| token.trim().trim_matches('"').to_owned())
            .filter(|token| !token.is_empty())
            .collect()
    } else {
        trimmed.split_whitespace().map(str::to_owned).collect()
    };
    for path in paths {
        analysis.volumes.push(DockerSingleValue {
            value: path,
            line: instruction.line,
            evidence: instruction.evidence.clone(),
        });
    }
}

fn add_command(
    analysis: &mut DockerfileAnalysis,
    instruction: &ParsedInstruction<'_>,
    kind: DockerCommandKind,
) {
    analysis.commands.push(DockerCommand {
        kind,
        command: instruction.value.to_owned(),
        exec_form: instruction.value.trim_start().starts_with('['),
        line: instruction.line,
        evidence: instruction.evidence.clone(),
    });
}

fn evidence(artifact: &Artifact, line_number: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    match SourceSpan::new(line_number, line_number) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

#[cfg(test)]
mod tests {
    use super::{DockerCommandKind, DockerfileAnalysis, DockerfileAnalyzer};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::fs;
    use std::path::Path;

    #[test]
    fn dockerfile_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let (artifact, text) = fixture_artifact(&root, "Dockerfile")?;
        let analysis = DockerfileAnalyzer.analyze(&artifact, &text);

        assert_eq!(
            snapshot(&analysis),
            "\
stage:0:rust:1.96:rust-builder:1
stage:1:python:3.13-slim:runtime:6
copy:-:rust/->./rust/:3
copy:-:requirements.txt->.:8
copy:-:src/python_app->./python_app:10
copy:rust-builder:/workspace/rust/target/release/worker->/usr/local/bin/worker:11
env:RIDGELINE_WORKER=/usr/local/bin/worker:12
port:8080/-:13
cmd:Run:cargo build --manifest-path rust/Cargo.toml --release:false:4
cmd:Run:pip install --no-cache-dir -r requirements.txt:false:9
cmd:Cmd:[\"python\", \"-m\", \"python_app.service\"]:true:14"
        );

        Ok(())
    }

    #[test]
    fn dockerfile_extracts_args_entrypoint_and_copy_from_flag()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = docker_artifact("Dockerfile")?;
        let text = "\
ARG VERSION=1.0
FROM --platform=linux/amd64 ghcr.io/example/app:${VERSION} AS build
ENV APP_HOME /app
COPY --from build /src/bin /bin/app
EXPOSE 8080/tcp 9000/udp
ENTRYPOINT [\"/bin/app\"]
";
        let analysis = DockerfileAnalyzer.analyze(&artifact, text);

        assert_eq!(analysis.stages[0].image, "ghcr.io/example/app:${VERSION}");
        assert_eq!(analysis.stages[0].alias.as_deref(), Some("build"));
        assert_eq!(analysis.args[0].key, "VERSION");
        assert_eq!(analysis.args[0].value.as_deref(), Some("1.0"));
        assert_eq!(analysis.env[0].key, "APP_HOME");
        assert_eq!(analysis.env[0].value.as_deref(), Some("/app"));
        assert_eq!(analysis.copies[0].from.as_deref(), Some("build"));
        assert_eq!(analysis.ports[0].protocol.as_deref(), Some("tcp"));
        assert!(
            analysis
                .commands
                .iter()
                .any(|command| command.kind == DockerCommandKind::Entrypoint && command.exec_form)
        );

        Ok(())
    }

    #[test]
    fn dockerfile_respects_policy_and_ignores_incomplete_lines()
    -> Result<(), Box<dyn std::error::Error>> {
        let allowed = docker_artifact("Dockerfile")?;
        let never = docker_artifact("secret.Dockerfile")?
            .with_model_policy(ModelExposurePolicy::Never)
            .with_text_status(TextStatus::UnsafeText, None);
        let binary =
            docker_artifact("binary.Dockerfile")?.with_text_status(TextStatus::Binary, None);
        let text = "\
# comment
FROM
COPY only-one-token
LABEL org.example.stage=dev
ARG DEBUG
";

        let analysis = DockerfileAnalyzer.analyze(&allowed, text);
        assert_eq!(analysis.instructions.len(), 4);
        assert!(analysis.stages.is_empty());
        assert!(analysis.copies.is_empty());
        assert_eq!(analysis.labels[0].key, "org.example.stage");
        assert_eq!(analysis.labels[0].value.as_deref(), Some("dev"));
        assert_eq!(analysis.args[0].key, "DEBUG");
        assert_eq!(analysis.args[0].value, None);
        assert!(
            DockerfileAnalyzer
                .analyze(&never, text)
                .instructions
                .is_empty()
        );
        assert!(
            DockerfileAnalyzer
                .analyze(&binary, text)
                .instructions
                .is_empty()
        );
        let no_span = super::evidence(&allowed, 0);
        assert_eq!(no_span.span, None);

        Ok(())
    }

    #[test]
    fn dockerfile_extracts_all_newly_supported_instructions()
    -> Result<(), Box<dyn std::error::Error>> {
        let artifact = docker_artifact("Dockerfile")?;
        let text = "\
FROM alpine AS build
WORKDIR /app
USER appuser
VOLUME [\"/data\", \"/logs\"]
VOLUME /extra
STOPSIGNAL SIGTERM
SHELL [\"powershell\", \"-command\"]
HEALTHCHECK --interval=30s CMD curl -f http://localhost/ || exit 1
ONBUILD RUN echo hello
";
        let analysis = DockerfileAnalyzer.analyze(&artifact, text);

        assert_eq!(analysis.workdirs[0].value, "/app");
        assert_eq!(analysis.users[0].value, "appuser");
        assert_eq!(
            analysis
                .volumes
                .iter()
                .map(|volume| volume.value.as_str())
                .collect::<Vec<_>>(),
            vec!["/data", "/logs", "/extra"]
        );
        assert_eq!(analysis.stop_signals[0].value, "SIGTERM");

        // HEALTHCHECK, SHELL, and ONBUILD have no dedicated structured
        // field, but must no longer be silently dropped from `instructions`.
        let recognized_kinds: Vec<_> = analysis
            .instructions
            .iter()
            .map(|instruction| instruction.kind)
            .collect();
        assert!(recognized_kinds.contains(&super::DockerInstructionKind::HealthCheck));
        assert!(recognized_kinds.contains(&super::DockerInstructionKind::Shell));
        assert!(recognized_kinds.contains(&super::DockerInstructionKind::Onbuild));
        let shell = analysis
            .instructions
            .iter()
            .find(|instruction| instruction.kind == super::DockerInstructionKind::Shell)
            .ok_or("shell instruction")?;
        assert_eq!(shell.value, "[\"powershell\", \"-command\"]");

        Ok(())
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

    fn docker_artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::ContainerDefinition,
            SupportTier::StructuredFormat,
            ContentHash::new("abcdef")?,
            10,
        )
        .with_detected_format("dockerfile")
        .with_text_status(TextStatus::Text, Some(1)))
    }

    fn snapshot(analysis: &DockerfileAnalysis) -> String {
        let mut lines = Vec::new();
        lines.extend(analysis.stages.iter().map(|stage| {
            format!(
                "stage:{}:{}:{}:{}",
                stage.index,
                stage.image,
                stage.alias.as_deref().unwrap_or("-"),
                stage.line
            )
        }));
        lines.extend(analysis.copies.iter().map(|copy| {
            format!(
                "copy:{}:{}->{}:{}",
                copy.from.as_deref().unwrap_or("-"),
                copy.sources.join("+"),
                copy.destination,
                copy.line
            )
        }));
        lines.extend(analysis.env.iter().map(|env| {
            format!(
                "env:{}={}:{}",
                env.key,
                env.value.as_deref().unwrap_or("-"),
                env.line
            )
        }));
        lines.extend(analysis.ports.iter().map(|port| {
            format!(
                "port:{}/{}:{}",
                port.port,
                port.protocol.as_deref().unwrap_or("-"),
                port.line
            )
        }));
        lines.extend(analysis.commands.iter().map(|command| {
            format!(
                "cmd:{:?}:{}:{}:{}",
                command.kind, command.command, command.exec_form, command.line
            )
        }));
        lines.join("\n")
    }
}
