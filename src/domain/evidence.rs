//! Source evidence references.

use crate::domain::ids::{ArtifactId, RepoPath};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// One-based inclusive source span.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceSpan {
    /// First covered line, one-based.
    pub start_line: u32,
    /// Last covered line, one-based and inclusive.
    pub end_line: u32,
}

impl SourceSpan {
    /// Creates a validated source span.
    pub fn new(start_line: u32, end_line: u32) -> Result<Self, SourceSpanError> {
        if start_line == 0 {
            return Err(SourceSpanError::ZeroStart);
        }
        if end_line < start_line {
            return Err(SourceSpanError::EndBeforeStart {
                start_line,
                end_line,
            });
        }

        Ok(Self {
            start_line,
            end_line,
        })
    }
}

impl Display for SourceSpan {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}-{}", self.start_line, self.end_line)
    }
}

/// Error returned when a source span cannot be represented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceSpanError {
    /// Source spans are one-based.
    ZeroStart,
    /// End line must not precede start line.
    EndBeforeStart {
        /// Requested start line.
        start_line: u32,
        /// Requested end line.
        end_line: u32,
    },
}

impl Display for SourceSpanError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ZeroStart => formatter.write_str("source span start line must be at least 1"),
            Self::EndBeforeStart {
                start_line,
                end_line,
            } => write!(
                formatter,
                "source span end line {end_line} precedes start line {start_line}"
            ),
        }
    }
}

impl std::error::Error for SourceSpanError {}

/// Evidence attached to graph nodes, relations, and generated documentation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRef {
    /// Artifact that contains the evidence.
    pub artifact_id: ArtifactId,
    /// Repository-relative evidence path.
    pub path: RepoPath,
    /// Optional line span for text artifacts.
    pub span: Option<SourceSpan>,
    /// Optional structured path such as a JSON pointer or YAML/TOML path.
    pub structured_path: Option<String>,
}

impl EvidenceRef {
    /// Creates file-level evidence for an artifact.
    pub fn file(artifact_id: ArtifactId, path: RepoPath) -> Self {
        Self {
            artifact_id,
            path,
            span: None,
            structured_path: None,
        }
    }

    /// Adds a source span.
    pub fn with_span(mut self, span: SourceSpan) -> Self {
        self.span = Some(span);
        self
    }

    /// Adds a structured path.
    pub fn with_structured_path(mut self, path: impl Into<String>) -> Self {
        self.structured_path = Some(path.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::{EvidenceRef, SourceSpan, SourceSpanError};
    use crate::domain::ids::{ArtifactId, RepoPath};

    #[test]
    fn source_span_validates_one_based_inclusive_ranges() -> Result<(), Box<dyn std::error::Error>>
    {
        let span = SourceSpan::new(3, 7)?;

        assert_eq!(span.to_string(), "3-7");
        assert_eq!(
            SourceSpan::new(0, 1).err(),
            Some(SourceSpanError::ZeroStart)
        );
        assert!(matches!(
            SourceSpan::new(4, 3),
            Err(SourceSpanError::EndBeforeStart {
                start_line: 4,
                end_line: 3
            })
        ));

        Ok(())
    }

    #[test]
    fn evidence_ref_can_target_lines_and_structured_paths() -> Result<(), Box<dyn std::error::Error>>
    {
        let path = RepoPath::new("config/settings.yaml")?;
        let artifact_id = ArtifactId::from_path(&path);
        let evidence = EvidenceRef::file(artifact_id, path)
            .with_span(SourceSpan::new(1, 5)?)
            .with_structured_path("service.image");

        assert_eq!(evidence.span, Some(SourceSpan::new(1, 5)?));
        assert_eq!(evidence.structured_path.as_deref(), Some("service.image"));

        Ok(())
    }

    #[test]
    fn source_span_errors_have_actionable_display_messages() {
        assert_eq!(
            SourceSpanError::ZeroStart.to_string(),
            "source span start line must be at least 1"
        );
        assert_eq!(
            SourceSpanError::EndBeforeStart {
                start_line: 10,
                end_line: 4,
            }
            .to_string(),
            "source span end line 4 precedes start line 10"
        );
    }

    #[test]
    fn evidence_ref_serializes_deterministically() -> Result<(), Box<dyn std::error::Error>> {
        let path = RepoPath::new("README.md")?;
        let artifact_id = ArtifactId::from_path(&path);
        let evidence = EvidenceRef::file(artifact_id, path).with_span(SourceSpan::new(2, 4)?);

        let json = serde_json::to_string_pretty(&evidence)?;
        let round_tripped: EvidenceRef = serde_json::from_str(&json)?;

        assert_eq!(round_tripped, evidence);
        assert!(json.contains("\"path\": \"README.md\""));
        assert!(json.contains("\"start_line\": 2"));

        Ok(())
    }
}
