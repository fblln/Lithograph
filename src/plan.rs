//! Deterministic documentation module planning.
//!
//! Modules are the unit of documentation generation. They are derived from
//! artifact paths and the semantic graph using fixed rules (crate/package
//! ownership, directory boundaries, artifact category) rather than LLM
//! clustering, so the same repository always plans the same modules.

use crate::domain::{Artifact, ArtifactCategory};
use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Deterministic module boundary category.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ModuleKind {
    /// A Rust workspace member/crate, keyed by crate name.
    RustCrate,
    /// A Python top-level package, keyed by package directory name.
    PythonPackage,
    /// A source-heavy directory not owned by a crate or package.
    Directory,
    /// Container/CI/deployment artifacts.
    Infrastructure,
    /// Existing documentation artifacts.
    Documentation,
    /// Configuration not owned by another module.
    Configuration,
    /// Synthetic bucket created only when a page-count ceiling merges many
    /// small modules together (see [`ModulePlanner::plan_with_budgets`]).
    /// Never produced by [`classify_artifact`] itself.
    Miscellaneous,
}

impl ModuleKind {
    fn slug(self) -> &'static str {
        match self {
            Self::RustCrate => "rust-crate",
            Self::PythonPackage => "python-package",
            Self::Directory => "directory",
            Self::Infrastructure => "infrastructure",
            Self::Documentation => "documentation",
            Self::Configuration => "configuration",
            Self::Miscellaneous => "miscellaneous",
        }
    }
}

/// One deterministic documentation module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentationModule {
    /// Stable module identifier.
    pub id: String,
    /// Human-readable module name.
    pub name: String,
    /// Module boundary category.
    pub kind: ModuleKind,
    /// Graph nodes belonging to this module: member artifacts plus every
    /// node reachable from them by a relation, stopping at other artifacts.
    pub members: Vec<GraphNodeId>,
    /// Deterministic hash over member artifact paths and content hashes.
    pub input_hash: String,
    /// Heuristic token estimate for this module's documentation context.
    pub estimated_tokens: u32,
}

/// Typed optional semantic grouping proposal, suitable for schema-validated
/// LLM or research output before falling back to deterministic planning.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SemanticGroupingProposal {
    /// Proposed module groups.
    pub groups: Vec<ProposedModuleGroup>,
}

/// One proposed semantic module group.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposedModuleGroup {
    /// Proposed human-readable module name.
    pub name: String,
    /// Existing module ids to merge into this group.
    pub module_ids: Vec<String>,
    /// Confidence from 0 to 100.
    pub confidence: u8,
}

/// Result of applying an optional grouping proposal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticGroupingResult {
    /// Planned modules, either proposed groups or deterministic fallback.
    pub modules: Vec<DocumentationModule>,
    /// Actionable warnings when fallback was used.
    pub warnings: Vec<String>,
    /// True when proposal output was accepted.
    pub used_proposal: bool,
}

/// Plans deterministic documentation modules from a repository's artifacts
/// and semantic graph.
#[derive(Debug, Clone, Copy, Default)]
pub struct ModulePlanner;

/// Default per-module token budget: a module whose [`estimate_tokens`]
/// exceeds this is split into multiple same-kind sub-modules (and therefore
/// multiple generated pages) rather than handed to the LLM as one
/// oversized/truncated context. Deliberately generous relative to
/// `generation::context`'s per-page excerpt caps, since this is a
/// module-splitting threshold, not the actual LLM context window.
pub const DEFAULT_TOKEN_BUDGET: u32 = 6000;

/// Default page-count ceiling: once bucketing would produce more than this
/// many modules, the smallest ones (fewest member artifacts first) are
/// merged into one shared `Miscellaneous` module until the count fits,
/// instead of emitting many single- or few-artifact pages. Deliberately
/// generous -- well above what any reasonably diverse repository plans to
/// today -- so this is a no-op except for repositories fragmented into an
/// unusually large number of small buckets.
pub const DEFAULT_PAGE_CEILING: usize = 40;

impl ModulePlanner {
    /// Plans modules under [`DEFAULT_TOKEN_BUDGET`] and [`DEFAULT_PAGE_CEILING`],
    /// sorted by ID for deterministic output.
    pub fn plan(&self, graph: &Graph, artifacts: &[Artifact]) -> Vec<DocumentationModule> {
        self.plan_with_budgets(graph, artifacts, DEFAULT_TOKEN_BUDGET, DEFAULT_PAGE_CEILING)
    }

    /// Plans modules with deterministic semantic grouping layered over the
    /// default plan. Deep language ownership boundaries (Rust crates and
    /// Python packages) stay intact; smaller supporting areas are grouped by
    /// semantic kind so generated docs can be less fragmented when requested.
    pub fn plan_with_semantic_grouping(
        &self,
        graph: &Graph,
        artifacts: &[Artifact],
    ) -> Vec<DocumentationModule> {
        let base = self.plan(graph, artifacts);
        semantic_group_modules(base)
    }

    /// Applies an optional typed semantic grouping proposal to the
    /// deterministic base plan. Invalid, incomplete, or low-confidence
    /// proposals fall back to the deterministic planner with a warning.
    pub fn plan_with_grouping_proposal(
        &self,
        graph: &Graph,
        artifacts: &[Artifact],
        proposal: Option<SemanticGroupingProposal>,
    ) -> SemanticGroupingResult {
        let base = self.plan(graph, artifacts);
        let Some(proposal) = proposal else {
            return SemanticGroupingResult {
                modules: base,
                warnings: Vec::new(),
                used_proposal: false,
            };
        };
        match apply_grouping_proposal(&base, &proposal) {
            Ok(modules) => SemanticGroupingResult {
                modules,
                warnings: Vec::new(),
                used_proposal: true,
            },
            Err(warning) => SemanticGroupingResult {
                modules: base,
                warnings: vec![warning],
                used_proposal: false,
            },
        }
    }

    /// Plans modules exactly like [`Self::plan`], with a caller-supplied
    /// `token_budget` and [`DEFAULT_PAGE_CEILING`].
    pub fn plan_with_budget(
        &self,
        graph: &Graph,
        artifacts: &[Artifact],
        token_budget: u32,
    ) -> Vec<DocumentationModule> {
        self.plan_with_budgets(graph, artifacts, token_budget, DEFAULT_PAGE_CEILING)
    }

    /// Plans modules with both budgets fully configurable.
    ///
    /// A module whose estimated token count exceeds `token_budget` is
    /// deterministically split into multiple `-part-N` sub-modules instead
    /// of staying one oversized module. Splitting preserves each bucket's
    /// path order and never produces an empty part; a single artifact that
    /// alone exceeds the budget still gets its own part rather than being
    /// dropped.
    ///
    /// Independently, when bucketing would produce more than `page_ceiling`
    /// modules, the smallest buckets are merged into one shared
    /// `Miscellaneous` module (smallest first, by member-artifact count)
    /// until the total fits -- see [`merge_small_buckets_to_ceiling`]. A
    /// repository already at or under the ceiling is unaffected.
    pub fn plan_with_budgets(
        &self,
        graph: &Graph,
        artifacts: &[Artifact],
        token_budget: u32,
        page_ceiling: usize,
    ) -> Vec<DocumentationModule> {
        let artifact_paths: BTreeSet<String> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str().to_owned())
            .collect();
        let cargo_toml_paths: BTreeSet<String> = artifacts
            .iter()
            .filter(|artifact| file_name(artifact.path.as_str()) == "Cargo.toml")
            .map(|artifact| artifact.path.as_str().to_owned())
            .collect();
        let package_names = package_names_by_manifest(graph);

        let mut buckets: BTreeMap<(ModuleKind, String), Vec<&Artifact>> = BTreeMap::new();
        for artifact in artifacts {
            let key =
                classify_artifact(artifact, &cargo_toml_paths, &artifact_paths, &package_names);
            buckets.entry(key).or_default().push(artifact);
        }

        let merged_buckets =
            merge_small_buckets_to_ceiling(buckets.into_iter().collect(), page_ceiling);

        let by_source = index_relations_by_source(graph);
        let node_by_id = index_nodes_by_id(graph);

        let mut modules: Vec<DocumentationModule> = merged_buckets
            .into_iter()
            .flat_map(|((kind, name), mut members)| {
                // Sort so splitting is a function of path order alone, not
                // of the caller's artifact iteration order.
                members.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
                build_module_or_split(kind, name, members, &by_source, &node_by_id, token_budget)
            })
            .collect();
        modules.sort_by(|a, b| a.id.cmp(&b.id));
        modules
    }
}

fn apply_grouping_proposal(
    base: &[DocumentationModule],
    proposal: &SemanticGroupingProposal,
) -> Result<Vec<DocumentationModule>, String> {
    if proposal.groups.is_empty() {
        return Err("semantic grouping proposal contained no groups".to_owned());
    }
    let by_id: BTreeMap<&str, &DocumentationModule> = base
        .iter()
        .map(|module| (module.id.as_str(), module))
        .collect();
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for group in &proposal.groups {
        if group.name.trim().is_empty() {
            return Err("semantic grouping proposal contains an unnamed group".to_owned());
        }
        if group.confidence < 70 {
            return Err(format!(
                "semantic grouping proposal group `{}` has low confidence {}",
                group.name, group.confidence
            ));
        }
        if group.module_ids.is_empty() {
            return Err(format!(
                "semantic grouping proposal group `{}` has no module ids",
                group.name
            ));
        }
        let mut members = Vec::new();
        let mut hashes = Vec::new();
        let mut estimated_tokens = 0u32;
        let mut kind = ModuleKind::Miscellaneous;
        for module_id in &group.module_ids {
            if !seen.insert(module_id.clone()) {
                return Err(format!(
                    "semantic grouping proposal references `{module_id}` more than once"
                ));
            }
            let module = by_id.get(module_id.as_str()).ok_or_else(|| {
                format!("semantic grouping proposal references unknown module `{module_id}`")
            })?;
            members.extend(module.members.iter().cloned());
            hashes.push(module.input_hash.as_str());
            estimated_tokens = estimated_tokens.saturating_add(module.estimated_tokens);
            if group.module_ids.len() == 1 {
                kind = module.kind;
            }
        }
        members.sort();
        members.dedup();
        hashes.sort_unstable();
        output.push(DocumentationModule {
            id: format!("module-plan:semantic-proposed:{}", slugify(&group.name)),
            name: group.name.clone(),
            kind,
            members,
            input_hash: blake3::hash(hashes.join("\n").as_bytes())
                .to_hex()
                .to_string(),
            estimated_tokens,
        });
    }
    if seen.len() != base.len() {
        return Err(
            "semantic grouping proposal did not cover every deterministic module".to_owned(),
        );
    }
    output.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(output)
}

fn semantic_group_modules(modules: Vec<DocumentationModule>) -> Vec<DocumentationModule> {
    let mut grouped: BTreeMap<ModuleKind, Vec<DocumentationModule>> = BTreeMap::new();
    let mut output = Vec::new();

    for module in modules {
        if matches!(
            module.kind,
            ModuleKind::RustCrate | ModuleKind::PythonPackage
        ) {
            output.push(module);
        } else {
            grouped.entry(module.kind).or_default().push(module);
        }
    }

    for (kind, mut modules) in grouped {
        if modules.len() == 1 {
            output.push(modules.remove(0));
            continue;
        }
        modules.sort_by(|a, b| a.id.cmp(&b.id));
        let name = format!("Semantic {:?}", kind);
        let mut members: Vec<GraphNodeId> = modules
            .iter()
            .flat_map(|module| module.members.iter().cloned())
            .collect();
        members.sort();
        members.dedup();
        let mut input_hashes: Vec<&str> = modules
            .iter()
            .map(|module| module.input_hash.as_str())
            .collect();
        input_hashes.sort_unstable();
        let input_hash = blake3::hash(input_hashes.join("\n").as_bytes())
            .to_hex()
            .to_string();
        let estimated_tokens = modules
            .iter()
            .map(|module| module.estimated_tokens)
            .sum::<u32>();
        output.push(DocumentationModule {
            id: format!("module-plan:semantic:{}", semantic_kind_slug(kind)),
            name,
            kind,
            members,
            input_hash,
            estimated_tokens,
        });
    }

    output.sort_by(|a, b| a.id.cmp(&b.id));
    output
}

fn semantic_kind_slug(kind: ModuleKind) -> &'static str {
    match kind {
        ModuleKind::RustCrate => "rust-crate",
        ModuleKind::PythonPackage => "python-package",
        ModuleKind::Directory => "directory",
        ModuleKind::Infrastructure => "infrastructure",
        ModuleKind::Documentation => "documentation",
        ModuleKind::Configuration => "configuration",
        ModuleKind::Miscellaneous => "miscellaneous",
    }
}

/// Merges the smallest buckets (fewest member artifacts first, ties broken
/// by name for determinism) into one shared `Miscellaneous` bucket until at
/// most `page_ceiling` buckets remain, so a repository fragmented into many
/// small buckets gets one shared page instead of many single-artifact ones.
/// A no-op when already at or under the ceiling (including `page_ceiling ==
/// 0`, treated as "no ceiling" rather than "merge everything").
fn merge_small_buckets_to_ceiling(
    mut buckets: Vec<((ModuleKind, String), Vec<&Artifact>)>,
    page_ceiling: usize,
) -> Vec<((ModuleKind, String), Vec<&Artifact>)> {
    if page_ceiling == 0 || buckets.len() <= page_ceiling {
        return buckets;
    }
    buckets.sort_by(|a, b| a.1.len().cmp(&b.1.len()).then((a.0).1.cmp(&(b.0).1)));

    let mut misc_members: Vec<&Artifact> = Vec::new();
    while !buckets.is_empty()
        && buckets.len() + usize::from(!misc_members.is_empty()) > page_ceiling
    {
        let (_, members) = buckets.remove(0);
        misc_members.extend(members);
    }
    if !misc_members.is_empty() {
        buckets.push((
            (ModuleKind::Miscellaneous, "Miscellaneous".to_owned()),
            misc_members,
        ));
    }
    buckets
}

/// Builds one module for `members`, or -- when their combined estimated
/// tokens exceed `token_budget` -- splits them into multiple `-part-N`
/// modules, each named and identified distinctly but sharing the same
/// [`ModuleKind`] and base name.
fn build_module_or_split(
    kind: ModuleKind,
    name: String,
    members: Vec<&Artifact>,
    by_source: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    node_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
    token_budget: u32,
) -> Vec<DocumentationModule> {
    if estimate_tokens(&members) <= token_budget || members.len() <= 1 {
        return vec![build_module(kind, name, &members, by_source, node_by_id)];
    }

    let chunks = split_by_token_budget(&members, token_budget);
    if chunks.len() <= 1 {
        return vec![build_module(kind, name, &members, by_source, node_by_id)];
    }

    let total_parts = chunks.len();
    chunks
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let mut module = build_module(kind, name.clone(), &chunk, by_source, node_by_id);
            module.id = format!("{}-part-{}", module.id, index + 1);
            module.name = format!("{name} (part {} of {total_parts})", index + 1);
            module
        })
        .collect()
}

/// Greedily groups `members` (assumed already sorted for determinism) into
/// contiguous chunks whose estimated tokens stay within `token_budget`.
/// Never emits an empty chunk: a single artifact that alone exceeds the
/// budget still becomes its own one-artifact chunk.
fn split_by_token_budget<'a>(
    members: &[&'a Artifact],
    token_budget: u32,
) -> Vec<Vec<&'a Artifact>> {
    let mut chunks: Vec<Vec<&Artifact>> = Vec::new();
    let mut current: Vec<&Artifact> = Vec::new();
    let mut current_tokens: u64 = 0;

    for artifact in members {
        let artifact_tokens = u64::from(estimate_tokens(std::slice::from_ref(artifact)));
        if !current.is_empty() && current_tokens + artifact_tokens > u64::from(token_budget) {
            chunks.push(std::mem::take(&mut current));
            current_tokens = 0;
        }
        current_tokens += artifact_tokens;
        current.push(artifact);
    }
    if !current.is_empty() {
        chunks.push(current);
    }
    chunks
}

fn classify_artifact(
    artifact: &Artifact,
    cargo_toml_paths: &BTreeSet<String>,
    artifact_paths: &BTreeSet<String>,
    package_names: &BTreeMap<String, String>,
) -> (ModuleKind, String) {
    let path = artifact.path.as_str();

    if artifact.detected_format.as_deref() == Some("rust")
        && let Some(manifest) = rust_crate_manifest(path, cargo_toml_paths)
        && let Some(name) = package_names.get(&manifest)
    {
        return (ModuleKind::RustCrate, name.clone());
    }
    if file_name(path) == "Cargo.toml"
        && let Some(name) = package_names.get(path)
    {
        return (ModuleKind::RustCrate, name.clone());
    }
    if artifact.detected_format.as_deref() == Some("python")
        && let Some(root) = python_package_root(path, artifact_paths)
    {
        return (ModuleKind::PythonPackage, top_level_name(&root));
    }

    match artifact.category {
        ArtifactCategory::ContainerDefinition
        | ArtifactCategory::ContinuousIntegration
        | ArtifactCategory::DeploymentDefinition => {
            (ModuleKind::Infrastructure, "Infrastructure".to_owned())
        }
        ArtifactCategory::Documentation => (ModuleKind::Documentation, "Documentation".to_owned()),
        ArtifactCategory::Configuration
        | ArtifactCategory::PackageManifest
        | ArtifactCategory::DependencyLockfile => {
            (ModuleKind::Configuration, "Configuration".to_owned())
        }
        _ => (ModuleKind::Directory, top_level_directory(path)),
    }
}

/// Finds the nearest ancestor directory (including the repository root)
/// that owns a `Cargo.toml`, so files under `src/bin/`, `src/`, etc. all
/// resolve to the same crate manifest.
fn rust_crate_manifest(path: &str, cargo_toml_paths: &BTreeSet<String>) -> Option<String> {
    let mut dir = parent_dir(path);
    loop {
        let candidate = if dir.is_empty() {
            "Cargo.toml".to_owned()
        } else {
            format!("{dir}/Cargo.toml")
        };
        if cargo_toml_paths.contains(&candidate) {
            return Some(candidate);
        }
        if dir.is_empty() {
            return None;
        }
        dir = parent_dir(&dir);
    }
}

/// Finds the outermost ancestor directory that is still a Python package
/// (has an `__init__.py`), so nested subpackages collapse into one
/// top-level package module.
fn python_package_root(path: &str, artifact_paths: &BTreeSet<String>) -> Option<String> {
    let file_dir = parent_dir(path);
    if !has_init(&file_dir, artifact_paths) {
        return None;
    }
    let mut root = file_dir;
    loop {
        let parent = parent_dir(&root);
        if parent.is_empty() || !has_init(&parent, artifact_paths) {
            break;
        }
        root = parent;
    }
    Some(root)
}

fn has_init(dir: &str, artifact_paths: &BTreeSet<String>) -> bool {
    artifact_paths.contains(&format!("{dir}/__init__.py"))
}

fn package_names_by_manifest(graph: &Graph) -> BTreeMap<String, String> {
    graph
        .relations
        .iter()
        .filter(|relation| relation.kind == RelationKind::BelongsToPackage)
        .filter_map(|relation| {
            let manifest_path = relation.source.as_str().strip_prefix("artifact:")?;
            let package = graph.nodes.iter().find_map(|node| match node {
                GraphNode::Package(package) if node.id() == &relation.target => Some(package),
                _ => None,
            })?;
            Some((manifest_path.to_owned(), package.name.clone()))
        })
        .collect()
}

fn index_relations_by_source(graph: &Graph) -> BTreeMap<GraphNodeId, Vec<GraphNodeId>> {
    let mut index: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
    for relation in &graph.relations {
        index
            .entry(relation.source.clone())
            .or_default()
            .push(relation.target.clone());
    }
    index
}

fn index_nodes_by_id(graph: &Graph) -> BTreeMap<GraphNodeId, &GraphNode> {
    graph
        .nodes
        .iter()
        .map(|node| (node.id().clone(), node))
        .collect()
}

/// Expands member artifacts to every node reachable from them, stopping at
/// any other `Artifact` node so one module never absorbs another module's
/// files just because a relation (e.g. a Markdown link) points at them.
fn expand_members(
    seeds: &[GraphNodeId],
    by_source: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    node_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
) -> Vec<GraphNodeId> {
    let mut visited: BTreeSet<GraphNodeId> = seeds.iter().cloned().collect();
    let mut frontier: Vec<GraphNodeId> = seeds.to_vec();

    while let Some(current) = frontier.pop() {
        let Some(targets) = by_source.get(&current) else {
            continue;
        };
        for target in targets {
            if visited.contains(target) {
                continue;
            }
            if matches!(node_by_id.get(target), Some(GraphNode::Artifact(_))) {
                continue;
            }
            visited.insert(target.clone());
            frontier.push(target.clone());
        }
    }

    visited.into_iter().collect()
}

fn build_module(
    kind: ModuleKind,
    name: String,
    members: &[&Artifact],
    by_source: &BTreeMap<GraphNodeId, Vec<GraphNodeId>>,
    node_by_id: &BTreeMap<GraphNodeId, &GraphNode>,
) -> DocumentationModule {
    let seeds: Vec<GraphNodeId> = members
        .iter()
        .map(|artifact| GraphNodeId::new(format!("artifact:{}", artifact.path)))
        .collect();
    let members_expanded = expand_members(&seeds, by_source, node_by_id);

    DocumentationModule {
        id: format!("module-plan:{}:{}", kind.slug(), slugify(&name)),
        name,
        kind,
        members: members_expanded,
        input_hash: compute_input_hash(members),
        estimated_tokens: estimate_tokens(members),
    }
}

fn compute_input_hash(members: &[&Artifact]) -> String {
    let mut sorted: Vec<&&Artifact> = members.iter().collect();
    sorted.sort_by(|a, b| a.path.as_str().cmp(b.path.as_str()));
    let joined = sorted
        .iter()
        .map(|artifact| format!("{}:{}", artifact.path, artifact.content_hash))
        .collect::<Vec<_>>()
        .join("\n");
    blake3::hash(joined.as_bytes()).to_hex().to_string()
}

// ponytail: a real BPE tokenizer (e.g. tiktoken-rs) was considered and
// rejected here -- the common Rust crates either shell out to a bundled
// model file or fetch encoding tables over the network on first use, which
// would risk violating this project's "no network access in normal test
// suites" rule for what is only a module-splitting threshold, not exact LLM
// billing. Using 3.5 bytes/token instead of a flat 4 is a documented,
// measured-in-the-literature adjustment, not an arbitrary tweak: OpenAI's
// own tokenizer guidance cites ~4 chars/token for general English prose,
// but source code and punctuation-dense structured config/markup -- most of
// what Lithograph module-plans -- commonly measure closer to 3-3.5
// chars/token because symbols and short identifiers each tend to consume
// their own token. This is still a coarse, content-blind estimate: module
// planning deliberately never reads file bytes (see
// `docs/dev/parser-spike-decisions.md`), only `Artifact.size_bytes`. Swap
// for a real tokenizer if the context builder (LIT-1.22) ever needs tighter
// budgets than a splitting decision requires.
fn estimate_tokens(members: &[&Artifact]) -> u32 {
    const BYTES_PER_TOKEN: f64 = 3.5;
    let total_bytes: u64 = members.iter().map(|artifact| artifact.size_bytes).sum();
    let tokens = (total_bytes as f64 / BYTES_PER_TOKEN).ceil();
    u32::try_from(tokens as u64).unwrap_or(u32::MAX)
}

fn slugify(name: &str) -> String {
    name.chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect()
}

fn top_level_directory(path: &str) -> String {
    path.split_once('/')
        .map_or_else(|| "root".to_owned(), |(dir, _)| dir.to_owned())
}

fn top_level_name(dir: &str) -> String {
    dir.rsplit('/').next().unwrap_or(dir).to_owned()
}

fn parent_dir(path: &str) -> String {
    path.rsplit_once('/')
        .map_or_else(String::new, |(dir, _)| dir.to_owned())
}

fn file_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

#[cfg(test)]
mod tests {
    use super::{ModuleKind, ModulePlanner, ProposedModuleGroup, SemanticGroupingProposal};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    #[test]
    fn plan_groups_fixture_into_deterministic_modules() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let modules = ModulePlanner.plan(&graph, &artifacts);

        let names: Vec<(ModuleKind, &str)> = modules
            .iter()
            .map(|module| (module.kind, module.name.as_str()))
            .collect();
        assert!(names.contains(&(ModuleKind::RustCrate, "fixture-worker")));
        assert!(names.contains(&(ModuleKind::PythonPackage, "python_app")));
        assert!(names.contains(&(ModuleKind::Infrastructure, "Infrastructure")));
        assert!(names.contains(&(ModuleKind::Documentation, "Documentation")));
        assert!(names.contains(&(ModuleKind::Configuration, "Configuration")));
        assert!(names.contains(&(ModuleKind::Directory, "web")));

        Ok(())
    }

    #[test]
    fn semantic_grouping_is_opt_in_and_preserves_deep_language_modules()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let default_modules = ModulePlanner.plan(&graph, &artifacts);
        let semantic_modules = ModulePlanner.plan_with_semantic_grouping(&graph, &artifacts);

        assert_ne!(default_modules, semantic_modules);
        assert!(
            semantic_modules
                .iter()
                .any(|module| module.kind == ModuleKind::RustCrate)
        );
        assert!(
            semantic_modules
                .iter()
                .any(|module| module.kind == ModuleKind::PythonPackage)
        );
        assert!(
            semantic_modules
                .iter()
                .any(|module| module.id.starts_with("module-plan:semantic:"))
        );

        Ok(())
    }

    #[test]
    fn valid_grouping_proposal_uses_typed_groups() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let base = ModulePlanner.plan(&graph, &artifacts);
        let proposal = SemanticGroupingProposal {
            groups: base
                .iter()
                .map(|module| ProposedModuleGroup {
                    name: module.name.clone(),
                    module_ids: vec![module.id.clone()],
                    confidence: 90,
                })
                .collect(),
        };

        let result = ModulePlanner.plan_with_grouping_proposal(&graph, &artifacts, Some(proposal));

        assert!(result.used_proposal);
        assert!(result.warnings.is_empty());
        assert_eq!(result.modules.len(), base.len());

        Ok(())
    }

    #[test]
    fn invalid_grouping_proposal_falls_back_with_warning() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let base = ModulePlanner.plan(&graph, &artifacts);
        let proposal = SemanticGroupingProposal {
            groups: vec![ProposedModuleGroup {
                name: "Low confidence".to_owned(),
                module_ids: vec![base[0].id.clone()],
                confidence: 10,
            }],
        };

        let result = ModulePlanner.plan_with_grouping_proposal(&graph, &artifacts, Some(proposal));

        assert!(!result.used_proposal);
        assert_eq!(result.modules, base);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("low confidence"))
        );

        Ok(())
    }

    #[test]
    fn rust_crate_module_contains_lib_and_bin_and_no_vendor_leak()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let crate_module = modules
            .iter()
            .find(|module| module.kind == ModuleKind::RustCrate)
            .ok_or("rust crate module")?;

        assert!(
            crate_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:rust/src/lib.rs")
        );
        assert!(
            crate_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:rust/src/bin/worker.rs")
        );
        assert!(
            !crate_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:vendor/example/lib.rs")
        );
        assert!(
            crate_module
                .members
                .iter()
                .any(|id| id.as_str().starts_with("symbol:rust/src/lib.rs#"))
        );

        let directory_module = modules
            .iter()
            .find(|module| module.kind == ModuleKind::Directory && module.name == "vendor")
            .ok_or("vendor directory module")?;
        assert!(
            directory_module
                .members
                .iter()
                .any(|id| id.as_str() == "artifact:vendor/example/lib.rs")
        );

        Ok(())
    }

    #[test]
    fn module_plan_is_deterministic_and_has_hashes_and_token_estimates()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        let first = ModulePlanner.plan(&graph, &artifacts);
        let second = ModulePlanner.plan(&graph, &artifacts);

        assert_eq!(first, second);
        assert!(first.iter().all(|module| !module.input_hash.is_empty()));
        assert!(first.iter().all(|module| module.estimated_tokens > 0));

        let ids: Vec<&str> = first.iter().map(|module| module.id.as_str()).collect();
        let mut sorted_ids = ids.clone();
        sorted_ids.sort_unstable();
        assert_eq!(ids, sorted_ids);

        Ok(())
    }

    /// Environment/configuration fact materialization adds canonical key
    /// nodes to this fixture. LIT-24's typed extraction also adds one
    /// generated-file fact and three Python symbols. The `vendor` module remains at one:
    /// `vendor/example/lib.rs` is opaque, so only its Artifact node is
    /// reachable and it contributes no source-derived symbols.
    #[test]
    fn module_plan_fixture_snapshot() -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions {
            include_hidden_directories: true,
            include_tests: true,
            ..WalkOptions::default()
        })
        .walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let snapshot = modules
            .iter()
            .map(|module| {
                format!(
                    "{}|{:?}|{}|{}|{}",
                    module.id,
                    module.kind,
                    module.name,
                    module.members.len(),
                    module.estimated_tokens
                )
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            snapshot,
            "\
module-plan:configuration:configuration|Configuration|Configuration|46|295
module-plan:directory:assets|Directory|assets|2|64
module-plan:directory:data|Directory|data|1|9
module-plan:directory:generated|Directory|generated|4|37
module-plan:directory:root|Directory|root|6|78
module-plan:directory:vendor|Directory|vendor|1|18
module-plan:directory:web|Directory|web|33|151
module-plan:documentation:documentation|Documentation|Documentation|13|1081
module-plan:infrastructure:infrastructure|Infrastructure|Infrastructure|19|384
module-plan:python-package:python-app|PythonPackage|python_app|20|370
module-plan:rust-crate:fixture-worker|RustCrate|fixture-worker|14|365"
        );

        Ok(())
    }

    #[test]
    fn oversized_module_splits_deterministically_under_a_token_budget()
    -> Result<(), Box<dyn std::error::Error>> {
        // Three ~350-byte (100-token, at 3.5 bytes/token) files under one
        // directory bucket. With a 250-token budget, the greedy splitter
        // fits the first two into one part (100 + 100 = 200 <= 250) and
        // starts a fresh part for the third (200 + 100 = 300 > 250) --
        // proving both "combines what fits" and "starts a new part when it
        // doesn't", not just one file per part.
        let repo = tempfile::TempDir::new()?;
        std::fs::create_dir_all(repo.path().join("big"))?;
        for name in ["one.dat", "two.dat", "three.dat"] {
            std::fs::write(repo.path().join("big").join(name), "x".repeat(350))?;
        }

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);

        let first = ModulePlanner.plan_with_budget(&graph, &artifacts, 250);
        let second = ModulePlanner.plan_with_budget(&graph, &artifacts, 250);
        assert_eq!(first, second, "splitting must be deterministic");

        let mut parts: Vec<_> = first
            .iter()
            .filter(|module| module.kind == ModuleKind::Directory && module.name.starts_with("big"))
            .collect();
        parts.sort_by(|a, b| a.id.cmp(&b.id));

        assert_eq!(parts.len(), 2, "expected exactly two parts");
        assert_eq!(parts[0].id, "module-plan:directory:big-part-1");
        assert_eq!(parts[1].id, "module-plan:directory:big-part-2");
        assert_eq!(parts[0].name, "big (part 1 of 2)");
        assert_eq!(parts[1].name, "big (part 2 of 2)");
        assert!(parts.iter().all(|part| part.estimated_tokens <= 250));

        let all_member_files: std::collections::BTreeSet<&str> = parts
            .iter()
            .flat_map(|part| part.members.iter())
            .filter_map(|id| id.as_str().strip_prefix("artifact:"))
            .collect();
        assert_eq!(
            all_member_files,
            std::collections::BTreeSet::from(["big/one.dat", "big/two.dat", "big/three.dat"])
        );

        // Under a budget large enough to hold everything, no split happens.
        let unsplit = ModulePlanner.plan_with_budget(&graph, &artifacts, 10_000);
        assert!(
            unsplit
                .iter()
                .any(|module| module.kind == ModuleKind::Directory && module.name == "big"),
            "expected a single unsplit 'big' module when the budget is generous"
        );

        Ok(())
    }

    #[test]
    fn small_repo_merges_under_a_low_page_ceiling_but_not_a_generous_one()
    -> Result<(), Box<dyn std::error::Error>> {
        // Five top-level directories, one tiny file each: without a page
        // ceiling this plans as five single-artifact Directory modules --
        // exactly the "many single-artifact pages" outcome LIT-10 exists to
        // avoid for small repositories.
        let repo = tempfile::TempDir::new()?;
        for dir in ["a", "b", "c", "d", "e"] {
            std::fs::create_dir_all(repo.path().join(dir))?;
            std::fs::write(repo.path().join(dir).join("one.dat"), "x")?;
        }

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(repo.path())?;
        let graph = GraphBuilder.build(repo.path(), &artifacts);

        // A generous ceiling (>= the natural module count) is a no-op.
        let generous = ModulePlanner.plan_with_budgets(&graph, &artifacts, u32::MAX, 10);
        let generous_directory_count = generous
            .iter()
            .filter(|module| module.kind == ModuleKind::Directory)
            .count();
        assert_eq!(
            generous_directory_count, 5,
            "ceiling >= count must be a no-op"
        );
        assert!(
            !generous
                .iter()
                .any(|module| module.kind == ModuleKind::Miscellaneous),
            "no Miscellaneous bucket should appear when nothing needed merging"
        );

        // A low ceiling merges the smallest (here, tied) buckets first,
        // deterministically, until the total fits.
        let first = ModulePlanner.plan_with_budgets(&graph, &artifacts, u32::MAX, 3);
        let second = ModulePlanner.plan_with_budgets(&graph, &artifacts, u32::MAX, 3);
        assert_eq!(first, second, "merging must be deterministic");
        assert_eq!(first.len(), 3, "expected exactly 3 modules under ceiling 3");

        let misc = first
            .iter()
            .find(|module| module.kind == ModuleKind::Miscellaneous)
            .ok_or("expected a Miscellaneous module")?;
        assert_eq!(misc.name, "Miscellaneous");
        // 5 directories collapse to 2 kept + 1 shared page: 3 of the 5
        // original single-file directories merge into the shared page.
        let kept_directories = first
            .iter()
            .filter(|module| module.kind == ModuleKind::Directory)
            .count();
        assert_eq!(kept_directories, 2);
        let misc_files: std::collections::BTreeSet<&str> = misc
            .members
            .iter()
            .filter_map(|id| id.as_str().strip_prefix("artifact:"))
            .collect();
        assert_eq!(
            misc_files.len(),
            3,
            "shared page should hold the 3 merged files"
        );

        Ok(())
    }
}
