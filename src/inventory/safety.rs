//! Safety policy for model exposure and secret redaction.

use crate::domain::{AnalyzerSelection, ModelExposurePolicy, SupportTier, TextStatus};
use crate::inventory::Classification;

const REDACTED: &str = "[REDACTED]";

/// Path and content safety policy for inventory artifacts.
#[derive(Debug, Clone, Copy, Default)]
pub struct SafetyPolicy;

/// Safety decision for a discovered artifact path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SafetyDecision {
    /// Text status after applying safety rules.
    pub text_status: TextStatus,
    /// Whether the artifact content must stay out of model/log context.
    pub metadata_only: bool,
}

impl SafetyPolicy {
    /// Returns the safe text status and exposure decision for a path.
    pub fn decide(&self, path: &str, detected_text_status: TextStatus) -> SafetyDecision {
        if detected_text_status == TextStatus::Binary {
            return SafetyDecision {
                text_status: TextStatus::Binary,
                metadata_only: true,
            };
        }

        let metadata_only = is_unsafe_path(path);
        SafetyDecision {
            text_status: if metadata_only {
                TextStatus::UnsafeText
            } else {
                detected_text_status
            },
            metadata_only,
        }
    }

    /// Applies a safety decision to a classifier result.
    pub fn apply(
        &self,
        mut classification: Classification,
        decision: SafetyDecision,
    ) -> Classification {
        if decision.metadata_only {
            classification.support_tier = SupportTier::Opaque;
            classification.model_policy = ModelExposurePolicy::Never;
            classification.analyzer = AnalyzerSelection::Opaque;
        }
        classification
    }

    /// Redacts likely secret values while preserving surrounding structure.
    pub fn redact_text(&self, text: &str) -> String {
        let mut redacted = Vec::new();
        let mut in_private_key = false;

        for line in text.lines() {
            if is_private_key_begin(line) {
                in_private_key = true;
                redacted.push(line.to_owned());
                continue;
            }
            if is_private_key_end(line) {
                in_private_key = false;
                redacted.push(line.to_owned());
                continue;
            }
            if in_private_key {
                redacted.push(REDACTED.to_owned());
                continue;
            }
            redacted.push(redact_line(line));
        }

        if text.ends_with('\n') {
            redacted.push(String::new());
        }
        redacted.join("\n")
    }
}

fn is_unsafe_path(path: &str) -> bool {
    let filename = file_name(path);
    let lower_path = path.to_ascii_lowercase();
    let lower_filename = filename.to_ascii_lowercase();

    lower_filename == ".env"
        || lower_filename.starts_with(".env.")
        || lower_filename == "id_rsa"
        || lower_filename == "id_dsa"
        || lower_filename == "id_ecdsa"
        || lower_filename == "id_ed25519"
        || lower_filename == ".npmrc"
        || lower_filename == ".pypirc"
        || lower_filename == "credentials"
        || lower_filename == "credentials.json"
        || lower_filename == "secrets.yml"
        || lower_filename == "secrets.yaml"
        || lower_filename.ends_with(".pem")
        || lower_filename.ends_with(".key")
        || lower_filename.ends_with(".p12")
        || lower_filename.ends_with(".pfx")
        || lower_path.contains("/.aws/credentials")
        || lower_path.contains("/.config/gcloud/")
}

fn redact_line(line: &str) -> String {
    if let Some((index, separator)) = first_separator(line) {
        let key = line[..index].trim().trim_matches('"').trim_matches('\'');
        if is_secret_key(key) {
            return format!("{}{} {}", &line[..index], separator, REDACTED);
        }
    }
    line.to_owned()
}

fn first_separator(line: &str) -> Option<(usize, char)> {
    let equals = line.find('=');
    let colon = line.find(':');
    match (equals, colon) {
        (Some(left), Some(right)) if left < right => Some((left, '=')),
        (Some(_), Some(right)) => Some((right, ':')),
        (Some(index), None) => Some((index, '=')),
        (None, Some(index)) => Some((index, ':')),
        (None, None) => None,
    }
}

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password")
        || key.contains("passwd")
        || key.contains("secret")
        || key.contains("token")
        || key.contains("api_key")
        || key.contains("apikey")
        || key.contains("access_key")
        || key.contains("private_key")
        || key.contains("credential")
}

fn is_private_key_begin(line: &str) -> bool {
    line.contains("-----BEGIN ") && line.contains("PRIVATE KEY-----")
}

fn is_private_key_end(line: &str) -> bool {
    line.contains("-----END ") && line.contains("PRIVATE KEY-----")
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::{SafetyPolicy, is_unsafe_path};
    use crate::domain::{
        AnalyzerSelection, ArtifactCategory, ModelExposurePolicy, SupportTier, TextStatus,
    };
    use crate::inventory::Classification;

    #[test]
    fn unsafe_paths_are_metadata_only() {
        let policy = SafetyPolicy;
        let unsafe_paths = [
            ".env",
            ".env.local",
            "config/secrets.yaml",
            "keys/private.pem",
            "keys/service.key",
            ".aws/credentials",
            "id_ed25519",
            ".npmrc",
        ];

        for path in unsafe_paths {
            let decision = policy.decide(path, TextStatus::Text);
            assert!(is_unsafe_path(path));
            assert_eq!(decision.text_status, TextStatus::UnsafeText);
            assert!(decision.metadata_only);
        }
    }

    #[test]
    fn binary_is_metadata_only_without_becoming_unsafe_text() {
        let decision = SafetyPolicy.decide("assets/logo.png", TextStatus::Binary);

        assert_eq!(decision.text_status, TextStatus::Binary);
        assert!(decision.metadata_only);
    }

    #[test]
    fn safety_override_makes_classification_opaque() {
        let classification = Classification {
            category: ArtifactCategory::Configuration,
            detected_format: Some("yaml".to_owned()),
            support_tier: SupportTier::StructuredFormat,
            model_policy: ModelExposurePolicy::Allowed,
            analyzer: AnalyzerSelection::Structured("yaml".to_owned()),
            generated_score: 0,
            vendored_score: 0,
        };
        let decision = SafetyPolicy.decide("config/secrets.yaml", TextStatus::Text);
        let classification = SafetyPolicy.apply(classification, decision);

        assert_eq!(classification.support_tier, SupportTier::Opaque);
        assert_eq!(classification.model_policy, ModelExposurePolicy::Never);
        assert_eq!(classification.analyzer, AnalyzerSelection::Opaque);
    }

    #[test]
    fn redact_text_masks_secret_values_and_private_key_bodies() {
        let input = "\
name: lithograph
URL=https://example.test
image: ghcr.io/example/app=latest
plain
password: hunter2
GITHUB_TOKEN=${{ secrets.GITHUB_TOKEN }}
-----BEGIN PRIVATE KEY-----
abc123
-----END PRIVATE KEY-----
";
        let redacted = SafetyPolicy.redact_text(input);

        assert!(redacted.contains("name: lithograph"));
        assert!(redacted.contains("URL=https://example.test"));
        assert!(redacted.contains("image: ghcr.io/example/app=latest"));
        assert!(redacted.contains("plain"));
        assert!(redacted.contains("password: [REDACTED]"));
        assert!(redacted.contains("GITHUB_TOKEN= [REDACTED]"));
        assert!(redacted.contains("-----BEGIN PRIVATE KEY-----\n[REDACTED]"));
        assert!(!redacted.contains("hunter2"));
        assert!(!redacted.contains("abc123"));
    }
}
