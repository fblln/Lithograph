//! Component-owned target-state reconciliation foundation (LIT-86.9).
//!
//! A *component* (repository, artifact, analyzer, graph pass, module, page
//! kind, search index, or any nested child) is identified by a stable
//! [`ComponentPath`] and declares the [`TargetRecord`]s it owns. The
//! [`reconcile`] function diffs those desired records against the persisted
//! [`OwnershipState`] and produces deterministic insert/update/no-op/delete
//! [`Action`]s plus the next state -- without mutating anything, so computation
//! (processing) is cleanly separated from commit.
//!
//! Records may be shared: ownership is a *set* of components, and a record is
//! deleted only when its last owner disappears, so one owner going away never
//! removes state another owner still justifies. When a component disappears,
//! all and only records exclusively owned by it (and its nested descendants)
//! are removed. Serialization is canonical (sorted maps/sets, no paths or
//! clocks), and commit is staged so a crash mid-write rolls back to the last
//! committed state.

// ponytail: the reconciliation foundation lands here; graph-fragment reconcile
// (LIT-86.10) and BM25 records (LIT-86.11) build on it. Drop this allow as
// those consumers land.
#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;

/// On-disk schema version for the ownership state. A bump discards old tracking
/// state and reconciles from empty (schema migration).
pub(crate) const OWNERSHIP_STATE_VERSION: u32 = 1;

/// A stable, hierarchical component identity, e.g.
/// `["artifact", "src/main.rs"]` or `["page-kind", "overview"]`. Nesting is
/// expressed by segment prefixes, so deleting an ancestor deletes descendants.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct ComponentPath(pub Vec<String>);

impl ComponentPath {
    /// Builds a component path from segments.
    pub(crate) fn new<I, S>(segments: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self(segments.into_iter().map(Into::into).collect())
    }

    /// True when `self` is `other` or an ancestor of it (a prefix of its
    /// segments), so clearing `self` from ownership also clears `other`.
    pub(crate) fn covers(&self, other: &Self) -> bool {
        other.0.len() >= self.0.len() && other.0[..self.0.len()] == self.0[..]
    }
}

/// One persisted record and the set of components that own it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnedRecord {
    /// Content hash of the record's payload.
    pub content_hash: String,
    /// Owning components, sorted for determinism.
    pub owners: BTreeSet<ComponentPath>,
}

/// The persisted ownership tracking state: which records exist, their content,
/// and who owns them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct OwnershipState {
    /// Schema version.
    pub version: u32,
    /// Records keyed by stable record key, sorted for determinism.
    pub records: BTreeMap<String, OwnedRecord>,
}

impl Default for OwnershipState {
    fn default() -> Self {
        Self {
            version: OWNERSHIP_STATE_VERSION,
            records: BTreeMap::new(),
        }
    }
}

impl OwnershipState {
    /// Loads state from `path`, returning an empty state when the file is
    /// missing, corrupt, or a different schema version (schema migration).
    /// Any leftover `*.pending` file from an interrupted commit is removed and
    /// ignored -- the committed file is authoritative (crash rollback, AC#6).
    pub(crate) fn load(path: &Path) -> Self {
        let pending = path.with_extension("pending");
        if pending.exists() {
            let _ = std::fs::remove_file(&pending);
        }
        let parsed = std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Self>(&text).ok());
        match parsed {
            Some(state) if state.version == OWNERSHIP_STATE_VERSION => state,
            _ => Self::default(),
        }
    }

    /// Commits state to `path`: staged to `*.pending`, then atomically renamed.
    /// A crash before the rename leaves the previous committed file intact.
    pub(crate) fn commit(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut json = serde_json::to_string(self).map_err(io::Error::other)?;
        json.push('\n');
        let pending = path.with_extension("pending");
        std::fs::write(&pending, &json)?;
        std::fs::rename(&pending, path)
    }
}

/// A record a component wants to exist after this reconcile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TargetRecord {
    /// Stable record key.
    pub key: String,
    /// Content hash of the payload.
    pub content_hash: String,
    /// The component declaring this record.
    pub owner: ComponentPath,
}

/// What a reconcile decided to do with one record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum ActionKind {
    /// The record is new.
    Insert,
    /// The record existed but its content changed.
    Update,
    /// The record is unchanged.
    NoOp,
    /// The record's last owner disappeared; remove it.
    Delete,
}

/// One decided action, keyed by record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Action {
    /// Record key.
    pub key: String,
    /// Decided action.
    pub kind: ActionKind,
}

/// The full outcome of a reconcile: deterministic actions, the next state to
/// commit, and any ownership conflicts detected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Reconciliation {
    /// Actions in canonical (key-sorted) order.
    pub actions: Vec<Action>,
    /// The state to commit if the caller proceeds.
    pub next: OwnershipState,
    /// Conflicts where two owners declared the same key with different content
    /// (AC#4): the contract is that shared records must agree; the
    /// lexicographically-first owner's content wins deterministically and the
    /// conflict is reported rather than silently resolved.
    pub conflicts: Vec<String>,
}

/// Diffs `desired` (declared by the in-`scope` components) against `previous`
/// and returns the actions plus next state. Pure: nothing is written, so a
/// caller can preview (AC#8) and only [`OwnershipState::commit`] on success
/// (processing/commit separation, AC#5).
///
/// `scope` is the set of components being recomputed this run. Their prior
/// ownership is cleared before `desired` re-adds it, so a component (or nested
/// descendant) that no longer declares a record releases it; a record with no
/// remaining owners is deleted. Components outside `scope` keep their ownership
/// untouched.
pub(crate) fn reconcile(
    previous: &OwnershipState,
    scope: &BTreeSet<ComponentPath>,
    desired: &[TargetRecord],
) -> Reconciliation {
    let covered = |owner: &ComponentPath| scope.iter().any(|path| path.covers(owner));

    // Carry forward records whose owners are (at least partly) outside the
    // scope, keeping their previous content.
    let mut next_records: BTreeMap<String, OwnedRecord> = BTreeMap::new();
    for (key, record) in &previous.records {
        let surviving: BTreeSet<ComponentPath> = record
            .owners
            .iter()
            .filter(|owner| !covered(owner))
            .cloned()
            .collect();
        if !surviving.is_empty() {
            next_records.insert(
                key.clone(),
                OwnedRecord {
                    content_hash: record.content_hash.clone(),
                    owners: surviving,
                },
            );
        }
    }

    // Apply desired records: add ownership and adopt the recomputed content.
    // Deterministic order (sorted by key then owner) makes conflict resolution
    // and the "first owner wins" rule reproducible.
    let mut sorted_desired: Vec<&TargetRecord> = desired.iter().collect();
    sorted_desired.sort_by(|a, b| a.key.cmp(&b.key).then_with(|| a.owner.cmp(&b.owner)));

    let mut conflicts = Vec::new();
    let mut content_claim: BTreeMap<String, String> = BTreeMap::new();
    for target in sorted_desired {
        let entry = next_records
            .entry(target.key.clone())
            .or_insert_with(|| OwnedRecord {
                content_hash: target.content_hash.clone(),
                owners: BTreeSet::new(),
            });
        entry.owners.insert(target.owner.clone());
        match content_claim.get(&target.key) {
            // First claimant this run sets the content.
            None => {
                content_claim.insert(target.key.clone(), target.content_hash.clone());
                entry.content_hash = target.content_hash.clone();
            }
            // A later claimant with different content is a conflict; the first
            // (already applied) wins.
            Some(first) if first != &target.content_hash => {
                conflicts.push(format!(
                    "record `{}` declared with conflicting content by multiple owners",
                    target.key
                ));
            }
            Some(_) => {}
        }
    }

    // Decide actions by comparing next vs previous.
    let mut keys: BTreeSet<&String> = previous.records.keys().collect();
    keys.extend(next_records.keys());
    let actions = keys
        .into_iter()
        .filter_map(|key| {
            let kind = match (previous.records.get(key), next_records.get(key)) {
                (None, Some(_)) => ActionKind::Insert,
                (Some(_), None) => ActionKind::Delete,
                (Some(prev), Some(next)) if prev.content_hash != next.content_hash => {
                    ActionKind::Update
                }
                (Some(_), Some(_)) => ActionKind::NoOp,
                (None, None) => return None,
            };
            Some(Action {
                key: key.clone(),
                kind,
            })
        })
        .collect();

    Reconciliation {
        actions,
        next: OwnershipState {
            version: OWNERSHIP_STATE_VERSION,
            records: next_records,
        },
        conflicts,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ActionKind, ComponentPath, OWNERSHIP_STATE_VERSION, OwnedRecord, OwnershipState,
        TargetRecord, reconcile,
    };
    use std::collections::{BTreeMap, BTreeSet};

    fn path(segments: &[&str]) -> ComponentPath {
        ComponentPath::new(segments.iter().copied())
    }

    fn scope(paths: &[ComponentPath]) -> BTreeSet<ComponentPath> {
        paths.iter().cloned().collect()
    }

    fn target(key: &str, content: &str, owner: ComponentPath) -> TargetRecord {
        TargetRecord {
            key: key.to_owned(),
            content_hash: content.to_owned(),
            owner,
        }
    }

    fn action_kind(rec: &super::Reconciliation, key: &str) -> Option<ActionKind> {
        rec.actions
            .iter()
            .find(|action| action.key == key)
            .map(|action| action.kind)
    }

    /// AC#2/#9 add, then no-op, then edit, then delete.
    #[test]
    fn add_noop_edit_delete_lifecycle() {
        let owner = path(&["artifact", "a.rs"]);
        // Add.
        let added = reconcile(
            &OwnershipState::default(),
            &scope(std::slice::from_ref(&owner)),
            &[target("chunk:a#0", "h1", owner.clone())],
        );
        assert_eq!(action_kind(&added, "chunk:a#0"), Some(ActionKind::Insert));

        // No-op: same content.
        let noop = reconcile(
            &added.next,
            &scope(std::slice::from_ref(&owner)),
            &[target("chunk:a#0", "h1", owner.clone())],
        );
        assert_eq!(action_kind(&noop, "chunk:a#0"), Some(ActionKind::NoOp));

        // Edit: content changed.
        let edited = reconcile(
            &added.next,
            &scope(std::slice::from_ref(&owner)),
            &[target("chunk:a#0", "h2", owner.clone())],
        );
        assert_eq!(action_kind(&edited, "chunk:a#0"), Some(ActionKind::Update));

        // Delete: owner declares nothing this run.
        let deleted = reconcile(&added.next, &scope(std::slice::from_ref(&owner)), &[]);
        assert_eq!(action_kind(&deleted, "chunk:a#0"), Some(ActionKind::Delete));
        assert!(deleted.next.records.is_empty());
    }

    /// AC#9 rename: an artifact's records move to a new key; old is deleted,
    /// new is inserted.
    #[test]
    fn rename_deletes_old_and_inserts_new() {
        let old_owner = path(&["artifact", "old.rs"]);
        let seeded = reconcile(
            &OwnershipState::default(),
            &scope(std::slice::from_ref(&old_owner)),
            &[target("chunk:old.rs#0", "h1", old_owner.clone())],
        );
        let new_owner = path(&["artifact", "new.rs"]);
        // Both artifacts are in scope: old declares nothing, new declares the
        // moved record (same content).
        let renamed = reconcile(
            &seeded.next,
            &scope(&[old_owner, new_owner.clone()]),
            &[target("chunk:new.rs#0", "h1", new_owner)],
        );
        assert_eq!(
            action_kind(&renamed, "chunk:old.rs#0"),
            Some(ActionKind::Delete)
        );
        assert_eq!(
            action_kind(&renamed, "chunk:new.rs#0"),
            Some(ActionKind::Insert)
        );
    }

    /// AC#3 nested deletion: clearing an ancestor path removes all descendant
    /// components' exclusively-owned records.
    #[test]
    fn ancestor_scope_deletes_nested_descendants() {
        let parent = path(&["module", "svc"]);
        let child = path(&["module", "svc", "handler"]);
        let seeded = reconcile(
            &OwnershipState::default(),
            &scope(std::slice::from_ref(&child)),
            &[target("rec:handler", "h1", child)],
        );
        // Reconciling the ancestor with nothing removes the child's record.
        let removed = reconcile(&seeded.next, &scope(&[parent]), &[]);
        assert_eq!(
            action_kind(&removed, "rec:handler"),
            Some(ActionKind::Delete)
        );
    }

    /// AC#4/#9 shared ownership: two owners hold one record; one leaving does
    /// not delete it.
    #[test]
    fn shared_record_survives_one_owner_leaving() {
        let a = path(&["analyzer", "python"]);
        let b = path(&["analyzer", "rust"]);
        let seeded = reconcile(
            &OwnershipState::default(),
            &scope(&[a.clone(), b.clone()]),
            &[
                target("shared", "h1", a.clone()),
                target("shared", "h1", b.clone()),
            ],
        );
        assert_eq!(seeded.next.records["shared"].owners.len(), 2);

        // Owner `a` disappears (declares nothing); `b` still owns `shared`.
        let after = reconcile(&seeded.next, &scope(&[a]), &[]);
        assert_eq!(action_kind(&after, "shared"), Some(ActionKind::NoOp));
        assert!(after.next.records.contains_key("shared"));
        assert_eq!(after.next.records["shared"].owners.len(), 1);
    }

    /// AC#4 conflicting ownership: two owners declare the same key with
    /// different content; the conflict is reported and resolved deterministically.
    #[test]
    fn conflicting_ownership_is_reported() {
        let a = path(&["a"]);
        let b = path(&["b"]);
        let result = reconcile(
            &OwnershipState::default(),
            &scope(&[a.clone(), b.clone()]),
            &[
                target("shared", "content-b", b),
                target("shared", "content-a", a),
            ],
        );
        assert!(!result.conflicts.is_empty(), "conflict is reported");
        // Deterministic: owner `a` sorts first, so its content wins.
        assert_eq!(result.next.records["shared"].content_hash, "content-a");
    }

    /// AC#5/#8 processing/commit separation and preview: reconcile mutates
    /// nothing on disk, so a computed plan can be inspected and discarded.
    #[test]
    fn reconcile_is_pure_preview() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path_on_disk = temp.path().join("ownership.json");
        let owner = path(&["artifact", "a.rs"]);
        let plan = reconcile(
            &OwnershipState::default(),
            &scope(std::slice::from_ref(&owner)),
            &[target("k", "h1", owner)],
        );
        // Previewed an Insert but committed nothing.
        assert_eq!(action_kind(&plan, "k"), Some(ActionKind::Insert));
        assert!(!path_on_disk.exists());
        Ok(())
    }

    /// AC#6 interrupted commit: a leftover `*.pending` from a crash is ignored,
    /// and the last committed state is what loads (rollback).
    #[test]
    fn interrupted_commit_rolls_back_to_committed_state() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        let path_on_disk = temp.path().join("ownership.json");
        let owner = path(&["artifact", "a.rs"]);
        let committed = reconcile(
            &OwnershipState::default(),
            &scope(std::slice::from_ref(&owner)),
            &[target("k", "h1", owner)],
        );
        committed.next.commit(&path_on_disk)?;

        // Simulate a crash mid-commit of a newer state: a stray `.pending`.
        std::fs::write(path_on_disk.with_extension("pending"), "garbage-incomplete")?;

        let loaded = OwnershipState::load(&path_on_disk);
        assert_eq!(loaded, committed.next, "rolled back to committed state");
        assert!(
            !path_on_disk.with_extension("pending").exists(),
            "stray pending is cleaned up"
        );
        Ok(())
    }

    /// AC#9 corrupt tracking state loads as empty.
    #[test]
    fn corrupt_state_loads_empty() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path_on_disk = temp.path().join("ownership.json");
        std::fs::write(&path_on_disk, "{ not json")?;
        assert!(OwnershipState::load(&path_on_disk).records.is_empty());
        Ok(())
    }

    /// AC#9 schema migration: a version mismatch reconciles from empty.
    #[test]
    fn schema_version_mismatch_resets() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let path_on_disk = temp.path().join("ownership.json");
        let stale = OwnershipState {
            version: OWNERSHIP_STATE_VERSION + 1,
            records: BTreeMap::from([(
                "k".to_owned(),
                OwnedRecord {
                    content_hash: "h".to_owned(),
                    owners: BTreeSet::new(),
                },
            )]),
        };
        let mut json = serde_json::to_string(&stale)?;
        json.push('\n');
        std::fs::write(&path_on_disk, json)?;
        assert!(OwnershipState::load(&path_on_disk).records.is_empty());
        Ok(())
    }

    /// AC#7 canonical serialization: state round-trips and is byte-identical
    /// regardless of insertion order (BTree ordering).
    #[test]
    fn state_serialization_is_canonical() -> Result<(), Box<dyn std::error::Error>> {
        let owner_z = path(&["z"]);
        let owner_a = path(&["a"]);
        let one = reconcile(
            &OwnershipState::default(),
            &scope(&[owner_z.clone(), owner_a.clone()]),
            &[
                target("k2", "h2", owner_z.clone()),
                target("k1", "h1", owner_a.clone()),
            ],
        );
        let two = reconcile(
            &OwnershipState::default(),
            &scope(&[owner_a.clone(), owner_z.clone()]),
            &[target("k1", "h1", owner_a), target("k2", "h2", owner_z)],
        );
        assert_eq!(
            serde_json::to_string(&one.next)?,
            serde_json::to_string(&two.next)?
        );
        Ok(())
    }
}
