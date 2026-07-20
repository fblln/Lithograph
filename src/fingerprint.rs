//! Pipeline logic fingerprints for cache invalidation (LIT-86.7).
//!
//! Every cacheable transformation ("pass") is identified by a [`Fingerprint`]:
//! a stable pass id, a declared logic version, a canonical input hash, the
//! hashes of its declared dependencies, and its output schema version. Two
//! runs reuse a cached result only when the whole fingerprint matches; when it
//! does not, [`Fingerprint::diff`] names the first differing field so a
//! diagnostic can explain *why* a cache hit or miss happened.
//!
//! The logic version is a declared constant, bumped when a pass's semantics
//! change (generalizing the single `GRAPH_BUILD_PIPELINE_VERSION`). This is the
//! explicit override the design calls for where automatic source inspection is
//! unavailable, and it means a whitespace/comment-only edit -- which does not
//! bump the constant -- never invalidates a cache. Serialization is
//! deterministic and excludes absolute paths, timestamps, and run ids, so a
//! fingerprint is identical across processes and machines.

// ponytail: the fingerprint framework lands here; LIT-86.8 wires it into the
// analyzer cache, resolver, page-context, and index stages one at a time. Drop
// this allow as those consumers land.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The identity of one cacheable transformation and every input that must
/// invalidate it when changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fingerprint {
    /// Stable pass/function identity (e.g. `"graph.build"`, `"analyze.python"`).
    pub pass_id: String,
    /// Declared logic version; bump on a semantic change to this pass.
    pub logic_version: u32,
    /// Canonical hash of this pass's own inputs (config, prompts, model, ...).
    pub input_hash: String,
    /// Hashes of declared dependencies, keyed by a stable dependency name and
    /// sorted for determinism.
    pub dependency_hashes: BTreeMap<String, String>,
    /// Output schema version; bump when the persisted output shape changes.
    pub output_schema_version: u32,
    /// Optional explicit override token folded into the digest, so a caller can
    /// force invalidation without editing a version constant (AC#5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub override_token: Option<String>,
}

/// The first field at which two fingerprints differ (AC#7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FingerprintField {
    /// Different pass identity.
    Pass,
    /// Different logic version.
    LogicVersion,
    /// Different canonical input hash.
    InputHash,
    /// A declared dependency was added, removed, or changed.
    Dependency(String),
    /// Different output schema version.
    OutputSchema,
    /// Different override token.
    OverrideToken,
}

/// A human-readable explanation of the first difference between two
/// fingerprints (AC#7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FingerprintDiff {
    /// The differing field.
    pub field: FingerprintField,
    /// A one-line reason suitable for a cache-miss diagnostic.
    pub reason: String,
}

impl Fingerprint {
    /// Builds a fingerprint. Prefer [`FingerprintBuilder`] for readable call
    /// sites; this exists for direct construction in tests.
    pub fn new(
        pass_id: impl Into<String>,
        logic_version: u32,
        input_hash: impl Into<String>,
        output_schema_version: u32,
    ) -> Self {
        Self {
            pass_id: pass_id.into(),
            logic_version,
            input_hash: input_hash.into(),
            dependency_hashes: BTreeMap::new(),
            output_schema_version,
            override_token: None,
        }
    }

    /// Adds a declared dependency hash.
    #[must_use]
    pub fn with_dependency(mut self, name: impl Into<String>, hash: impl Into<String>) -> Self {
        self.dependency_hashes.insert(name.into(), hash.into());
        self
    }

    /// Sets an explicit override token (AC#5).
    #[must_use]
    pub fn with_override(mut self, token: impl Into<String>) -> Self {
        self.override_token = Some(token.into());
        self
    }

    /// Deterministic digest over every field, in a canonical, path/time-free
    /// form (AC#6). Two equal fingerprints always produce the same digest on
    /// any process or machine.
    pub fn digest(&self) -> String {
        // A fixed field order with unit-separated key/value pairs; the
        // dependency map is already sorted (BTreeMap), so the string is a pure
        // function of the fingerprint's value.
        let mut canonical = format!(
            "pass={}\u{1f}logic={}\u{1f}input={}\u{1f}schema={}\u{1f}override={}",
            self.pass_id,
            self.logic_version,
            self.input_hash,
            self.output_schema_version,
            self.override_token.as_deref().unwrap_or(""),
        );
        for (name, hash) in &self.dependency_hashes {
            canonical.push_str(&format!("\u{1e}dep:{name}={hash}"));
        }
        blake3::hash(canonical.as_bytes()).to_hex().to_string()
    }

    /// True when a cached result produced under `self` can be reused now under
    /// `current` (identical digests).
    pub fn is_compatible_with(&self, current: &Self) -> bool {
        self.digest() == current.digest()
    }

    /// The first field at which `self` (the cached fingerprint) differs from
    /// `current`, or `None` when they are identical. Field order is fixed so
    /// the "first" difference is deterministic.
    pub fn diff(&self, current: &Self) -> Option<FingerprintDiff> {
        if self.pass_id != current.pass_id {
            return Some(FingerprintDiff {
                field: FingerprintField::Pass,
                reason: format!("pass id {} != {}", self.pass_id, current.pass_id),
            });
        }
        if self.logic_version != current.logic_version {
            return Some(FingerprintDiff {
                field: FingerprintField::LogicVersion,
                reason: format!(
                    "logic version {} != {}",
                    self.logic_version, current.logic_version
                ),
            });
        }
        if self.output_schema_version != current.output_schema_version {
            return Some(FingerprintDiff {
                field: FingerprintField::OutputSchema,
                reason: format!(
                    "output schema {} != {}",
                    self.output_schema_version, current.output_schema_version
                ),
            });
        }
        if self.override_token != current.override_token {
            return Some(FingerprintDiff {
                field: FingerprintField::OverrideToken,
                reason: "override token changed".to_owned(),
            });
        }
        if self.input_hash != current.input_hash {
            return Some(FingerprintDiff {
                field: FingerprintField::InputHash,
                reason: "canonical input hash changed".to_owned(),
            });
        }
        // Dependency differences: report the first added, removed, or changed
        // dependency by sorted name for determinism.
        for (name, hash) in &current.dependency_hashes {
            match self.dependency_hashes.get(name) {
                Some(previous) if previous == hash => {}
                Some(_) => {
                    return Some(FingerprintDiff {
                        field: FingerprintField::Dependency(name.clone()),
                        reason: format!("dependency `{name}` changed"),
                    });
                }
                None => {
                    return Some(FingerprintDiff {
                        field: FingerprintField::Dependency(name.clone()),
                        reason: format!("dependency `{name}` added"),
                    });
                }
            }
        }
        for name in self.dependency_hashes.keys() {
            if !current.dependency_hashes.contains_key(name) {
                return Some(FingerprintDiff {
                    field: FingerprintField::Dependency(name.clone()),
                    reason: format!("dependency `{name}` removed"),
                });
            }
        }
        None
    }
}

/// Accumulates named input parts into one canonical input hash, so a pass can
/// fold prompt text, context schema, model/provider/config identity, language
/// registry version, feature flags, external tool versions, and relevant
/// environment-derived configuration into a single fingerprint input (AC#4).
///
/// Parts are keyed and sorted, so declaration order does not affect the hash,
/// and only the supplied values contribute -- never a path or a clock.
#[derive(Debug, Clone, Default)]
pub struct InputHasher {
    parts: BTreeMap<String, String>,
}

impl InputHasher {
    /// Creates an empty hasher.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces a named input part.
    #[must_use]
    pub fn with(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.parts.insert(key.into(), value.into());
        self
    }

    /// Finalizes the canonical input hash.
    pub fn finish(&self) -> String {
        let mut canonical = String::new();
        for (key, value) in &self.parts {
            canonical.push_str(&format!("{key}={value}\u{1f}"));
        }
        blake3::hash(canonical.as_bytes()).to_hex().to_string()
    }
}

/// Convenience builder for a [`Fingerprint`].
#[derive(Debug, Clone)]
pub struct FingerprintBuilder {
    fingerprint: Fingerprint,
}

impl FingerprintBuilder {
    /// Starts a fingerprint for `pass_id` at `logic_version`/`output_schema_version`.
    pub fn new(pass_id: impl Into<String>, logic_version: u32, output_schema_version: u32) -> Self {
        Self {
            fingerprint: Fingerprint::new(
                pass_id,
                logic_version,
                String::new(),
                output_schema_version,
            ),
        }
    }

    /// Sets the canonical input hash from an [`InputHasher`].
    #[must_use]
    pub fn inputs(mut self, hasher: &InputHasher) -> Self {
        self.fingerprint.input_hash = hasher.finish();
        self
    }

    /// Adds a declared dependency.
    #[must_use]
    pub fn dependency(mut self, name: impl Into<String>, hash: impl Into<String>) -> Self {
        self.fingerprint = self.fingerprint.with_dependency(name, hash);
        self
    }

    /// Sets an explicit override token.
    #[must_use]
    pub fn override_token(mut self, token: impl Into<String>) -> Self {
        self.fingerprint = self.fingerprint.with_override(token);
        self
    }

    /// Finalizes the fingerprint.
    pub fn build(self) -> Fingerprint {
        self.fingerprint
    }
}

#[cfg(test)]
mod tests {
    use super::{Fingerprint, FingerprintBuilder, FingerprintField, InputHasher};

    fn base() -> Fingerprint {
        FingerprintBuilder::new("graph.build", 1, 1)
            .inputs(
                &InputHasher::new()
                    .with("model", "mock")
                    .with("prompt", "v1"),
            )
            .dependency("analyze.python", "hash-a")
            .build()
    }

    /// AC#6: the digest is deterministic across repeated computation and does
    /// not depend on dependency insertion order.
    #[test]
    fn digest_is_deterministic_and_order_independent() {
        let a = FingerprintBuilder::new("p", 1, 1)
            .dependency("z", "1")
            .dependency("a", "2")
            .build();
        let b = FingerprintBuilder::new("p", 1, 1)
            .dependency("a", "2")
            .dependency("z", "1")
            .build();
        assert_eq!(a.digest(), b.digest());
        assert_eq!(a.digest(), a.digest());
    }

    /// AC#2: a change that does not bump the logic version (e.g. whitespace,
    /// modeled as "same inputs, same version") stays compatible -- a cache hit.
    #[test]
    fn unchanged_logic_and_inputs_is_a_hit() {
        assert!(base().is_compatible_with(&base()));
        assert_eq!(base().diff(&base()), None);
    }

    /// AC#8 code-only change: bumping the logic version invalidates.
    #[test]
    fn logic_version_bump_invalidates() {
        let current = FingerprintBuilder::new("graph.build", 2, 1)
            .inputs(
                &InputHasher::new()
                    .with("model", "mock")
                    .with("prompt", "v1"),
            )
            .dependency("analyze.python", "hash-a")
            .build();
        assert!(!base().is_compatible_with(&current));
        assert_eq!(
            base().diff(&current).map(|diff| diff.field),
            Some(FingerprintField::LogicVersion)
        );
    }

    /// AC#8 input-only change: a changed model identity invalidates via the
    /// input hash even with the same logic version.
    #[test]
    fn input_change_invalidates() {
        let current = FingerprintBuilder::new("graph.build", 1, 1)
            .inputs(
                &InputHasher::new()
                    .with("model", "real")
                    .with("prompt", "v1"),
            )
            .dependency("analyze.python", "hash-a")
            .build();
        assert!(!base().is_compatible_with(&current));
        assert_eq!(
            base().diff(&current).map(|diff| diff.field),
            Some(FingerprintField::InputHash)
        );
    }

    /// AC#8 transitive helper change: a changed declared dependency hash
    /// invalidates and names the dependency.
    #[test]
    fn dependency_change_invalidates_and_is_named() {
        let current = FingerprintBuilder::new("graph.build", 1, 1)
            .inputs(
                &InputHasher::new()
                    .with("model", "mock")
                    .with("prompt", "v1"),
            )
            .dependency("analyze.python", "hash-b")
            .build();
        assert_eq!(
            base().diff(&current).map(|diff| diff.field),
            Some(FingerprintField::Dependency("analyze.python".to_owned()))
        );
    }

    /// AC#8 conditional/declared dependency: adding a dependency invalidates
    /// and is reported as added.
    #[test]
    fn added_dependency_is_reported() -> Result<(), Box<dyn std::error::Error>> {
        let current = FingerprintBuilder::new("graph.build", 1, 1)
            .inputs(
                &InputHasher::new()
                    .with("model", "mock")
                    .with("prompt", "v1"),
            )
            .dependency("analyze.python", "hash-a")
            .dependency("analyze.rust", "hash-c")
            .build();
        let diff = base().diff(&current).ok_or("expected a difference")?;
        assert_eq!(
            diff.field,
            FingerprintField::Dependency("analyze.rust".to_owned())
        );
        assert!(diff.reason.contains("added"));
        Ok(())
    }

    /// AC#5 version override: an override token forces a miss without touching
    /// the logic version or inputs.
    #[test]
    fn override_token_forces_invalidation() {
        let overridden = FingerprintBuilder::new("graph.build", 1, 1)
            .inputs(
                &InputHasher::new()
                    .with("model", "mock")
                    .with("prompt", "v1"),
            )
            .dependency("analyze.python", "hash-a")
            .override_token("hotfix-2026-07-20")
            .build();
        assert!(!base().is_compatible_with(&overridden));
        assert_eq!(
            base().diff(&overridden).map(|diff| diff.field),
            Some(FingerprintField::OverrideToken)
        );
    }

    /// AC#8 hidden dependency: two fingerprints that fail to declare a
    /// dependency look identical (a false hit), documenting why declaring it
    /// matters -- and that once declared, a change is caught.
    #[test]
    fn undeclared_dependency_is_a_false_hit_until_declared() {
        // Neither declares the hidden dependency: identical, so a (wrong) hit.
        let a = FingerprintBuilder::new("p", 1, 1).build();
        let b = FingerprintBuilder::new("p", 1, 1).build();
        assert!(a.is_compatible_with(&b));
        // Declaring the dependency with differing hashes now catches the change.
        let a = a.with_dependency("hidden", "old");
        let b = b.with_dependency("hidden", "new");
        assert!(!a.is_compatible_with(&b));
    }

    /// AC#8 unrelated-stage reuse: different pass ids never collide, so an
    /// unrelated stage stays reusable when another changes.
    #[test]
    fn unrelated_passes_are_independent() {
        let graph = FingerprintBuilder::new("graph.build", 1, 1).build();
        let docs = FingerprintBuilder::new("docs.render", 1, 1).build();
        assert_ne!(graph.digest(), docs.digest());
        // Bumping docs does not change graph's digest.
        let docs_v2 = FingerprintBuilder::new("docs.render", 2, 1).build();
        assert_ne!(docs.digest(), docs_v2.digest());
        assert_eq!(
            graph.digest(),
            FingerprintBuilder::new("graph.build", 1, 1)
                .build()
                .digest()
        );
    }
}
