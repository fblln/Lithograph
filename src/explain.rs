//! Deterministic invalidation explain plans (LIT-86.17).
//!
//! An [`ExplainPlan`] is a versioned, machine-readable description of what a
//! run *would* do and *why*: one [`ExplainEntry`] per component/target with a
//! stable component path, key, planned [`Action`], source-state compatibility,
//! a [`ReasonCode`], the differing fingerprint category, and the relevant
//! dependency path. It is a pure function of the compared fingerprints, so a
//! dry run can produce it without writing any repository state, and a lab can
//! assert bounded recomputation from the reason/action codes without parsing
//! human prose.
//!
//! Diagnostics carry only stable ids, reason codes, and safe repository-
//! relative keys -- never source text, prompts, secrets, or absolute paths
//! (AC#8): the entry shape has no field that could hold a payload.

// ponytail: the explain-plan model lands here; wiring `update --dry-run
// --explain` (CLI) and the MCP explain tool are thin surfaces on top, a
// follow-on. Drop this allow when they land.
#![allow(dead_code)]

use crate::fingerprint::{Fingerprint, FingerprintField};
use serde::{Deserialize, Serialize};

/// Version of the explain-plan schema.
pub(crate) const EXPLAIN_PLAN_VERSION: u32 = 1;

/// Why a component/target was kept or recomputed (AC#2). Ordered so that, when
/// several differences exist, the *first* (most causal) is reported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ReasonCode {
    /// No relevant input changed; the cached result is reused.
    UnchangedReuse,
    /// The canonical input changed.
    InputChanged,
    /// The pass's logic version changed.
    LogicChanged,
    /// A semantic configuration input (prompt, feature flag) changed.
    SemanticConfigChanged,
    /// The output schema version changed.
    SchemaChanged,
    /// The provider/model identity changed.
    ProviderModelChanged,
    /// A declared transitive dependency changed.
    TransitiveDependency,
    /// No prior state existed for this target.
    MissingState,
    /// Prior state existed but was corrupt/unreadable.
    CorruptState,
    /// The target's last owner disappeared.
    OwnershipRemoved,
    /// An explicit override forced recomputation.
    ExplicitRebuild,
}

/// The action a run would take for one component/target (AC#1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Action {
    /// Reuse the existing result.
    Reuse,
    /// Create a new result.
    Insert,
    /// Recompute an existing result.
    Update,
    /// Remove an orphaned result.
    Delete,
}

/// One planned action with its cause.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExplainEntry {
    /// Stable component path (e.g. `"graph.fragment:src/main.rs"`).
    pub component_path: String,
    /// Target or attachment key.
    pub key: String,
    /// The planned action.
    pub action: Action,
    /// Whether the prior source state was compatible.
    pub compatible: bool,
    /// Why the action was chosen.
    pub reason: ReasonCode,
    /// The differing fingerprint category, when a fingerprint drove the
    /// decision (e.g. `"logic_version"`, `"dependency:analyze.python"`).
    pub differing_field: Option<String>,
    /// The relevant dependency path from the change to this target, when known.
    pub dependency_path: Vec<String>,
}

impl ExplainEntry {
    /// Builds an entry by diffing a `cached` fingerprint against the `current`
    /// one, mapping the first differing field to a reason code and action. A
    /// clean match is a reuse; any difference is an update.
    pub(crate) fn from_fingerprints(
        component_path: impl Into<String>,
        key: impl Into<String>,
        cached: &Fingerprint,
        current: &Fingerprint,
    ) -> Self {
        let component_path = component_path.into();
        let key = key.into();
        match cached.diff(current) {
            None => Self {
                component_path,
                key,
                action: Action::Reuse,
                compatible: true,
                reason: ReasonCode::UnchangedReuse,
                differing_field: None,
                dependency_path: Vec::new(),
            },
            Some(diff) => {
                let (reason, field) = match &diff.field {
                    FingerprintField::LogicVersion => {
                        (ReasonCode::LogicChanged, "logic_version".to_owned())
                    }
                    FingerprintField::InputHash => {
                        (ReasonCode::InputChanged, "input_hash".to_owned())
                    }
                    FingerprintField::OutputSchema => (
                        ReasonCode::SchemaChanged,
                        "output_schema_version".to_owned(),
                    ),
                    FingerprintField::OverrideToken => {
                        (ReasonCode::ExplicitRebuild, "override_token".to_owned())
                    }
                    FingerprintField::Dependency(name) => (
                        ReasonCode::TransitiveDependency,
                        format!("dependency:{name}"),
                    ),
                    FingerprintField::Pass => (ReasonCode::InputChanged, "pass_id".to_owned()),
                };
                Self {
                    component_path,
                    key,
                    action: Action::Update,
                    compatible: false,
                    reason,
                    differing_field: Some(field),
                    dependency_path: Vec::new(),
                }
            }
        }
    }

    /// A target whose last owner disappeared: a deterministic delete (AC#2).
    pub(crate) fn orphan_delete(component_path: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            component_path: component_path.into(),
            key: key.into(),
            action: Action::Delete,
            compatible: false,
            reason: ReasonCode::OwnershipRemoved,
            differing_field: None,
            dependency_path: Vec::new(),
        }
    }

    /// A target with no prior state: a fresh insert.
    pub(crate) fn missing_state(component_path: impl Into<String>, key: impl Into<String>) -> Self {
        Self {
            component_path: component_path.into(),
            key: key.into(),
            action: Action::Insert,
            compatible: false,
            reason: ReasonCode::MissingState,
            differing_field: None,
            dependency_path: Vec::new(),
        }
    }
}

/// A full plan: versioned and deterministically ordered (AC#1/#3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExplainPlan {
    /// Schema version.
    pub version: u32,
    /// Entries, sorted by `(component_path, key)` for a deterministic plan that
    /// never depends on hash-map or execution order.
    pub entries: Vec<ExplainEntry>,
}

impl ExplainPlan {
    /// Assembles a plan, sorting entries into canonical order.
    pub(crate) fn new(mut entries: Vec<ExplainEntry>) -> Self {
        entries.sort_by(|a, b| {
            a.component_path
                .cmp(&b.component_path)
                .then_with(|| a.key.cmp(&b.key))
        });
        Self {
            version: EXPLAIN_PLAN_VERSION,
            entries,
        }
    }

    /// Counts of each action, for run metrics.
    pub(crate) fn action_counts(&self) -> ActionCounts {
        let mut counts = ActionCounts::default();
        for entry in &self.entries {
            match entry.action {
                Action::Reuse => counts.reuse += 1,
                Action::Insert => counts.insert += 1,
                Action::Update => counts.update += 1,
                Action::Delete => counts.delete += 1,
            }
        }
        counts
    }

    /// Compares this preview against the `actual` post-run plan and returns the
    /// entries whose action or reason diverged (AC#7). Empty means the run did
    /// exactly what was previewed.
    pub(crate) fn divergence(&self, actual: &Self) -> Vec<String> {
        let mut divergences = Vec::new();
        let key = |entry: &ExplainEntry| (entry.component_path.clone(), entry.key.clone());
        let actual_by_key: std::collections::BTreeMap<_, _> = actual
            .entries
            .iter()
            .map(|entry| (key(entry), entry))
            .collect();
        for planned in &self.entries {
            match actual_by_key.get(&key(planned)) {
                Some(done) if done.action == planned.action && done.reason == planned.reason => {}
                Some(done) => divergences.push(format!(
                    "{}/{}: planned {:?}/{:?}, actual {:?}/{:?}",
                    planned.component_path,
                    planned.key,
                    planned.action,
                    planned.reason,
                    done.action,
                    done.reason
                )),
                None => divergences.push(format!(
                    "{}/{}: planned but not performed",
                    planned.component_path, planned.key
                )),
            }
        }
        divergences
    }
}

/// Per-action counts for run metrics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ActionCounts {
    /// Reused targets.
    pub reuse: usize,
    /// Inserted targets.
    pub insert: usize,
    /// Updated targets.
    pub update: usize,
    /// Deleted targets.
    pub delete: usize,
}

#[cfg(test)]
mod tests {
    use super::{Action, ExplainEntry, ExplainPlan, ReasonCode};
    use crate::fingerprint::FingerprintBuilder;

    fn fp(logic: u32, schema: u32) -> crate::fingerprint::Fingerprint {
        FingerprintBuilder::new("graph.build", logic, schema)
            .dependency("analyze.python", "hash-a")
            .build()
    }

    /// AC#9 no-op: identical fingerprints yield a reuse with no differing field.
    #[test]
    fn unchanged_is_reuse() {
        let entry = ExplainEntry::from_fingerprints("graph.build", "k", &fp(1, 1), &fp(1, 1));
        assert_eq!(entry.action, Action::Reuse);
        assert_eq!(entry.reason, ReasonCode::UnchangedReuse);
        assert!(entry.differing_field.is_none());
    }

    /// AC#2/#9 logic change maps to LogicChanged/Update.
    #[test]
    fn logic_change_is_reported() {
        let entry = ExplainEntry::from_fingerprints("graph.build", "k", &fp(1, 1), &fp(2, 1));
        assert_eq!(entry.action, Action::Update);
        assert_eq!(entry.reason, ReasonCode::LogicChanged);
        assert_eq!(entry.differing_field.as_deref(), Some("logic_version"));
    }

    /// AC#2/#9 schema migration maps to SchemaChanged.
    #[test]
    fn schema_change_is_reported() {
        let entry = ExplainEntry::from_fingerprints("graph.build", "k", &fp(1, 1), &fp(1, 2));
        assert_eq!(entry.reason, ReasonCode::SchemaChanged);
    }

    /// AC#2/#9 transitive dependency change names the dependency.
    #[test]
    fn dependency_change_is_transitive() {
        let cached = fp(1, 1);
        let current = FingerprintBuilder::new("graph.build", 1, 1)
            .dependency("analyze.python", "hash-CHANGED")
            .build();
        let entry = ExplainEntry::from_fingerprints("graph.build", "k", &cached, &current);
        assert_eq!(entry.reason, ReasonCode::TransitiveDependency);
        assert_eq!(
            entry.differing_field.as_deref(),
            Some("dependency:analyze.python")
        );
    }

    /// AC#2 orphan cleanup and missing state are their own reason codes.
    #[test]
    fn orphan_and_missing_reasons() {
        assert_eq!(
            ExplainEntry::orphan_delete("c", "k").reason,
            ReasonCode::OwnershipRemoved
        );
        assert_eq!(
            ExplainEntry::missing_state("c", "k").reason,
            ReasonCode::MissingState
        );
    }

    /// AC#1/#3: the plan is deterministically ordered regardless of input order.
    #[test]
    fn plan_is_deterministically_ordered() {
        let a = ExplainEntry::missing_state("z.comp", "k1");
        let b = ExplainEntry::missing_state("a.comp", "k2");
        let c = ExplainEntry::missing_state("a.comp", "k1");
        let plan = ExplainPlan::new(vec![a, b, c]);
        let order: Vec<(&str, &str)> = plan
            .entries
            .iter()
            .map(|entry| (entry.component_path.as_str(), entry.key.as_str()))
            .collect();
        assert_eq!(
            order,
            [("a.comp", "k1"), ("a.comp", "k2"), ("z.comp", "k1")]
        );
    }

    /// AC#7: a preview that matches the post-run plan reports no divergence; a
    /// mismatch is reported.
    #[test]
    fn preview_versus_actual_divergence() {
        let preview = ExplainPlan::new(vec![ExplainEntry::from_fingerprints(
            "graph.build",
            "k",
            &fp(1, 1),
            &fp(2, 1),
        )]);
        // Actual did exactly the planned update.
        assert!(preview.divergence(&preview.clone()).is_empty());

        // Actual instead reused it: divergence reported.
        let actual = ExplainPlan::new(vec![ExplainEntry::from_fingerprints(
            "graph.build",
            "k",
            &fp(1, 1),
            &fp(1, 1),
        )]);
        assert!(!preview.divergence(&actual).is_empty());
    }

    /// AC#8: entries hold only stable ids/reason codes -- serializing a plan
    /// never leaks source text (there is no field that could carry it).
    #[test]
    fn plan_serialization_is_redaction_safe() -> Result<(), Box<dyn std::error::Error>> {
        let plan = ExplainPlan::new(vec![ExplainEntry::from_fingerprints(
            "graph.fragment:src/secret_handler.rs",
            "chunk:src/secret_handler.rs#0",
            &fp(1, 1),
            &fp(2, 1),
        )]);
        let json = serde_json::to_string(&plan)?;
        // Only the stable path/key/reason appear; no payload fields exist.
        assert!(json.contains("logic_version"));
        assert!(json.contains("LogicChanged"));
        assert!(!json.contains("prompt"));
        assert!(!json.contains("text"));
        Ok(())
    }
}
