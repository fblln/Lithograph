//! Opt-in per-target MCP server integration for coding agents (LIT-22.8.3).
//! Codex, Claude, Gemini, and Zed each get a project-scoped MCP server
//! registration pointing at `lithograph mcp-server <repo_root>`, merged
//! into whatever config that target already has rather than overwriting
//! it. Aider has no native MCP server support as of this writing, so it is
//! reported as an unsupported target with an actionable alternative rather
//! than silently skipped or given a fabricated config format (AC3).
//!
//! Nothing here ever writes without an explicit [`apply`] call (AC1);
//! [`preview`] renders the exact proposed file content without touching
//! disk (AC2), and both use the same rendering path so a preview always
//! matches what `apply` would actually write.

use serde::Serialize;
use std::fmt::{Display, Formatter};
use std::path::{Path, PathBuf};

const SERVER_NAME: &str = "lithograph";

/// A coding agent Lithograph can register its MCP server with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum AgentTarget {
    /// OpenAI Codex CLI, via project-scoped `.codex/config.toml`.
    Codex,
    /// Claude Code, via project-scoped `.mcp.json`.
    Claude,
    /// Gemini CLI, via project-scoped `.gemini/settings.json`.
    Gemini,
    /// Zed editor, via project-scoped `.zed/settings.json`.
    Zed,
    /// Aider: no native MCP support (see module docs).
    Aider,
}

/// Every known target, in a stable order.
pub(crate) const ALL_TARGETS: [AgentTarget; 5] = [
    AgentTarget::Codex,
    AgentTarget::Claude,
    AgentTarget::Gemini,
    AgentTarget::Zed,
    AgentTarget::Aider,
];

impl AgentTarget {
    /// Stable lowercase identifier, used for `--target` and detection output.
    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Gemini => "gemini",
            Self::Zed => "zed",
            Self::Aider => "aider",
        }
    }

    /// Parses a `--target` value, case-insensitively.
    pub(crate) fn parse(id: &str) -> Option<Self> {
        ALL_TARGETS
            .into_iter()
            .find(|target| target.id().eq_ignore_ascii_case(id))
    }

    /// Repository-relative MCP config path this target reads, or `None`
    /// when the target has no MCP integration Lithograph can offer.
    pub(crate) fn config_path(self) -> Option<&'static str> {
        match self {
            Self::Codex => Some(".codex/config.toml"),
            Self::Claude => Some(".mcp.json"),
            Self::Gemini => Some(".gemini/settings.json"),
            Self::Zed => Some(".zed/settings.json"),
            Self::Aider => None,
        }
    }

    /// Actionable explanation for a target with no MCP config path.
    fn unsupported_reason(self) -> Option<&'static str> {
        match self {
            Self::Aider => Some(
                "aider has no native MCP server support; run `lithograph integrate-agents` \
                 for repository-level instructions instead, or point .aider.conf.yml's `read:` \
                 list at the generated docs/lithograph output manually",
            ),
            _ => None,
        }
    }
}

impl Display for AgentTarget {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.id())
    }
}

/// Detection status for one target (AC1): read-only, never writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct TargetDetection {
    /// The target inspected.
    pub target: AgentTarget,
    /// `true` when Lithograph can integrate MCP for this target at all.
    pub supported: bool,
    /// Where this target's config would be read/written, when supported.
    pub config_path: Option<PathBuf>,
    /// `true` when that config file already exists.
    pub config_exists: bool,
    /// `true` when that config file already has a Lithograph MCP entry.
    pub already_integrated: bool,
    /// Actionable explanation when `supported` is `false`.
    pub reason: Option<String>,
}

/// Detects every known target's status under `repo_root` without writing
/// anything (AC1).
pub(crate) fn detect(repo_root: &Path) -> Vec<TargetDetection> {
    ALL_TARGETS
        .into_iter()
        .map(|target| detect_one(repo_root, target))
        .collect()
}

fn detect_one(repo_root: &Path, target: AgentTarget) -> TargetDetection {
    let Some(relative) = target.config_path() else {
        return TargetDetection {
            target,
            supported: false,
            config_path: None,
            config_exists: false,
            already_integrated: false,
            reason: target.unsupported_reason().map(str::to_owned),
        };
    };
    let config_path = repo_root.join(relative);
    let config_exists = config_path.is_file();
    let already_integrated = config_exists
        && std::fs::read_to_string(&config_path)
            .map(|content| content.contains(SERVER_NAME))
            .unwrap_or(false);
    TargetDetection {
        target,
        supported: true,
        config_path: Some(config_path),
        config_exists,
        already_integrated,
        reason: None,
    }
}

/// Why a preview or apply could not complete.
#[derive(Debug)]
pub(crate) enum IntegrationError {
    /// The target has no MCP integration Lithograph can offer.
    Unsupported {
        /// The target requested.
        target: AgentTarget,
        /// Actionable explanation.
        reason: String,
    },
    /// The existing config file could not be parsed or merged into.
    InvalidExistingConfig(String),
    /// The merged config could not be serialized.
    Serialize(String),
    /// A filesystem operation failed.
    Io(std::io::Error),
}

impl Display for IntegrationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unsupported { target, reason } => {
                write!(
                    formatter,
                    "{target} does not support MCP integration: {reason}"
                )
            }
            Self::InvalidExistingConfig(reason) => {
                write!(formatter, "cannot merge into existing config: {reason}")
            }
            Self::Serialize(reason) => write!(formatter, "failed to render config: {reason}"),
            Self::Io(source) => write!(formatter, "filesystem error: {source}"),
        }
    }
}

impl std::error::Error for IntegrationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
            _ => None,
        }
    }
}

impl From<std::io::Error> for IntegrationError {
    fn from(source: std::io::Error) -> Self {
        Self::Io(source)
    }
}

/// A previewed or applied integration for one target (AC2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub(crate) struct IntegrationOutcome {
    /// The target integrated.
    pub target: AgentTarget,
    /// Config path written to (or that would be written to).
    pub config_path: PathBuf,
    /// Full proposed file content.
    pub content: String,
    /// `true` when `content` differs from what's currently on disk (or the
    /// file doesn't exist yet). A second call after `apply` always reports
    /// `false` here, proving idempotency.
    pub changed: bool,
}

/// Renders the exact content [`apply`] would write, without touching disk.
pub(crate) fn preview(
    repo_root: &Path,
    target: AgentTarget,
) -> Result<IntegrationOutcome, IntegrationError> {
    build_outcome(repo_root, target)
}

/// Merges Lithograph's MCP server entry into `target`'s config file,
/// writing only when the rendered content actually differs from what's on
/// disk (idempotent: a second call is always a no-op).
pub(crate) fn apply(
    repo_root: &Path,
    target: AgentTarget,
) -> Result<IntegrationOutcome, IntegrationError> {
    let outcome = build_outcome(repo_root, target)?;
    if outcome.changed {
        if let Some(parent) = outcome.config_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&outcome.config_path, &outcome.content)?;
    }
    Ok(outcome)
}

fn build_outcome(
    repo_root: &Path,
    target: AgentTarget,
) -> Result<IntegrationOutcome, IntegrationError> {
    let Some(relative) = target.config_path() else {
        return Err(IntegrationError::Unsupported {
            target,
            reason: target
                .unsupported_reason()
                .unwrap_or("no MCP config path is defined for this target")
                .to_owned(),
        });
    };
    let config_path = repo_root.join(relative);
    let existing = if config_path.is_file() {
        Some(std::fs::read_to_string(&config_path)?)
    } else {
        None
    };

    let content = match target {
        AgentTarget::Claude | AgentTarget::Gemini => {
            render_json(existing.as_deref(), "mcpServers", stdio_entry(repo_root))?
        }
        AgentTarget::Zed => {
            render_json(existing.as_deref(), "context_servers", zed_entry(repo_root))?
        }
        AgentTarget::Codex => render_toml(existing.as_deref(), repo_root)?,
        AgentTarget::Aider => unreachable!("Aider has no config_path; handled above"),
    };
    let changed = existing.as_deref() != Some(content.as_str());

    Ok(IntegrationOutcome {
        target,
        config_path,
        content,
        changed,
    })
}

fn mcp_server_args(repo_root: &Path) -> Vec<String> {
    vec!["mcp-server".to_owned(), repo_root.display().to_string()]
}

fn stdio_entry(repo_root: &Path) -> serde_json::Value {
    serde_json::json!({
        "command": "lithograph",
        "args": mcp_server_args(repo_root),
    })
}

fn zed_entry(repo_root: &Path) -> serde_json::Value {
    serde_json::json!({
        "source": "custom",
        "command": "lithograph",
        "args": mcp_server_args(repo_root),
        "env": {},
    })
}

fn render_json(
    existing: Option<&str>,
    group_key: &str,
    entry: serde_json::Value,
) -> Result<String, IntegrationError> {
    let mut root: serde_json::Value = match existing.map(str::trim) {
        Some(text) if !text.is_empty() => serde_json::from_str(text)
            .map_err(|source| IntegrationError::InvalidExistingConfig(source.to_string()))?,
        _ => serde_json::json!({}),
    };
    let object = root.as_object_mut().ok_or_else(|| {
        IntegrationError::InvalidExistingConfig(
            "existing config root is not a JSON object".to_owned(),
        )
    })?;
    let group = object
        .entry(group_key.to_owned())
        .or_insert_with(|| serde_json::json!({}));
    let group_object = group.as_object_mut().ok_or_else(|| {
        IntegrationError::InvalidExistingConfig(format!(
            "existing \"{group_key}\" is not a JSON object"
        ))
    })?;
    group_object.insert(SERVER_NAME.to_owned(), entry);

    let mut rendered = serde_json::to_string_pretty(&root)
        .map_err(|source| IntegrationError::Serialize(source.to_string()))?;
    rendered.push('\n');
    Ok(rendered)
}

fn render_toml(existing: Option<&str>, repo_root: &Path) -> Result<String, IntegrationError> {
    let mut root: toml::Value = match existing.map(str::trim) {
        Some(text) if !text.is_empty() => toml::from_str(text)
            .map_err(|source| IntegrationError::InvalidExistingConfig(source.to_string()))?,
        _ => toml::Value::Table(toml::value::Table::new()),
    };
    let table = root.as_table_mut().ok_or_else(|| {
        IntegrationError::InvalidExistingConfig(
            "existing config root is not a TOML table".to_owned(),
        )
    })?;
    let servers = table
        .entry("mcp_servers")
        .or_insert_with(|| toml::Value::Table(toml::value::Table::new()));
    let servers_table = servers.as_table_mut().ok_or_else(|| {
        IntegrationError::InvalidExistingConfig(
            "existing \"mcp_servers\" is not a TOML table".to_owned(),
        )
    })?;

    let mut entry = toml::value::Table::new();
    entry.insert(
        "command".to_owned(),
        toml::Value::String("lithograph".to_owned()),
    );
    entry.insert(
        "args".to_owned(),
        toml::Value::Array(
            mcp_server_args(repo_root)
                .into_iter()
                .map(toml::Value::String)
                .collect(),
        ),
    );
    servers_table.insert(SERVER_NAME.to_owned(), toml::Value::Table(entry));

    toml::to_string_pretty(&root).map_err(|source| IntegrationError::Serialize(source.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{AgentTarget, apply, detect, preview};

    #[test]
    fn parse_round_trips_every_target_id() {
        for target in super::ALL_TARGETS {
            assert_eq!(AgentTarget::parse(target.id()), Some(target));
        }
        assert_eq!(AgentTarget::parse("not-a-target"), None);
    }

    #[test]
    fn detect_never_writes_and_reports_every_target() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        let detections = detect(temp.path());

        assert_eq!(detections.len(), 5);
        let aider = detections
            .iter()
            .find(|detection| detection.target == AgentTarget::Aider)
            .ok_or("expected an aider detection entry")?;
        assert!(!aider.supported);
        assert!(
            aider
                .reason
                .as_deref()
                .is_some_and(|reason| reason.contains("no native MCP"))
        );
        let claude = detections
            .iter()
            .find(|detection| detection.target == AgentTarget::Claude)
            .ok_or("expected a claude detection entry")?;
        assert!(claude.supported);
        assert!(!claude.config_exists);
        assert!(!claude.already_integrated);
        assert!(std::fs::read_dir(temp.path())?.next().is_none());

        Ok(())
    }

    #[test]
    fn preview_never_writes_to_disk() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        let outcome = preview(temp.path(), AgentTarget::Claude)?;

        assert!(outcome.changed);
        assert!(outcome.content.contains("mcpServers"));
        assert!(outcome.content.contains("mcp-server"));
        assert!(!outcome.config_path.exists());

        Ok(())
    }

    #[test]
    fn apply_writes_claude_mcp_json_and_preserves_other_servers()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join(".mcp.json"),
            r#"{"mcpServers":{"other":{"command":"other-tool","args":[]}}}"#,
        )?;

        let outcome = apply(temp.path(), AgentTarget::Claude)?;

        assert!(outcome.changed);
        let written = std::fs::read_to_string(&outcome.config_path)?;
        let value: serde_json::Value = serde_json::from_str(&written)?;
        assert_eq!(value["mcpServers"]["other"]["command"], "other-tool");
        assert_eq!(value["mcpServers"]["lithograph"]["command"], "lithograph");
        assert_eq!(value["mcpServers"]["lithograph"]["args"][0], "mcp-server");

        Ok(())
    }

    #[test]
    fn apply_is_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        let first = apply(temp.path(), AgentTarget::Zed)?;
        let after_first = std::fs::read_to_string(&first.config_path)?;
        let second = apply(temp.path(), AgentTarget::Zed)?;
        let after_second = std::fs::read_to_string(&second.config_path)?;

        assert!(first.changed);
        assert!(!second.changed);
        assert_eq!(after_first, after_second);

        Ok(())
    }

    #[test]
    fn apply_writes_codex_config_toml() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        let outcome = apply(temp.path(), AgentTarget::Codex)?;

        let written = std::fs::read_to_string(&outcome.config_path)?;
        let value: toml::Value = toml::from_str(&written)?;
        assert_eq!(
            value["mcp_servers"]["lithograph"]["command"].as_str(),
            Some("lithograph")
        );

        Ok(())
    }

    #[test]
    fn aider_is_reported_as_unsupported_with_an_actionable_message()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;

        match preview(temp.path(), AgentTarget::Aider) {
            Ok(_) => Err("expected an unsupported-target error".into()),
            Err(error) => {
                let message = error.to_string();
                assert!(message.contains("aider"));
                assert!(message.contains("no native MCP"));
                assert!(!temp.path().join(".aider.conf.yml").exists());
                Ok(())
            }
        }
    }
}
