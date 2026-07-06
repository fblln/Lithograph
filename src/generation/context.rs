//! Bounded, evidence-backed model context and prompt construction for
//! module, architecture, and quickstart pages.

use crate::domain::{Artifact, ModelExposurePolicy, TextStatus};
use crate::generation::llm::ModelRequest;
use crate::graph::{Graph, GraphNode, GraphNodeId};
use crate::inventory::SafetyPolicy;
use crate::manifest::TaskKind;
use crate::plan::DocumentationModule;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Maximum lines of a single artifact's content included as an excerpt.
const MAX_EXCERPT_LINES: usize = 120;
/// Maximum relation lines listed per section, so a highly-connected module
/// can't blow out the prompt budget.
const MAX_RELATION_LINES: usize = 40;

/// One artifact excerpt actually sent to the model, recorded so tests (and
/// later the evidence validator) can prove what evidence was available.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextExcerpt {
    /// Repository-relative artifact path.
    pub artifact_path: String,
    /// Model exposure policy applied when selecting this excerpt.
    pub policy: ModelExposurePolicy,
    /// Lines actually included.
    pub included_lines: usize,
    /// True when the artifact had more content than `included_lines`.
    pub truncated: bool,
}

/// A fully assembled, bounded model context for one page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelContext {
    /// System/instruction prompt (schema + evidence rules).
    pub system_prompt: String,
    /// User/context prompt: members, summaries, excerpts, relations.
    pub user_prompt: String,
    /// Every artifact excerpt actually included in `user_prompt`.
    pub excerpts: Vec<ContextExcerpt>,
    /// Hash over the context's inputs.
    pub input_hash: String,
    /// Page category this context is for.
    pub task_kind: TaskKind,
}

impl ModelContext {
    /// Converts this context into a [`ModelRequest`] for a specific model/prompt version.
    pub fn into_request(
        self,
        model: impl Into<String>,
        prompt_version: impl Into<String>,
    ) -> ModelRequest {
        ModelRequest {
            model: model.into(),
            prompt_version: prompt_version.into(),
            task_kind: self.task_kind,
            input_hash: self.input_hash,
            system_prompt: self.system_prompt,
            user_prompt: self.user_prompt,
        }
    }
}

/// Builds bounded, evidence-backed contexts for documentation generation.
#[derive(Debug, Clone, Copy, Default)]
pub struct ContextBuilder;

impl ContextBuilder {
    /// Builds the leaf context for one module: members, summaries,
    /// source/config/doc excerpts, inbound/outbound relations, unresolved
    /// references, and existing docs evidence.
    pub fn build_module_context(
        &self,
        module: &DocumentationModule,
        graph: &Graph,
        artifacts: &[Artifact],
        repo_root: &Path,
    ) -> ModelContext {
        let artifact_by_path: BTreeMap<&str, &Artifact> = artifacts
            .iter()
            .map(|artifact| (artifact.path.as_str(), artifact))
            .collect();
        let node_by_id: BTreeMap<&GraphNodeId, &GraphNode> =
            graph.nodes.iter().map(|node| (node.id(), node)).collect();
        let member_set: std::collections::BTreeSet<&GraphNodeId> = module.members.iter().collect();

        let mut sections = Vec::new();
        sections.push(format!("## Module: {} ({:?})\n", module.name, module.kind));

        let mut excerpts = Vec::new();
        let mut symbol_lines = Vec::new();
        let mut summary_lines = Vec::new();
        let mut unresolved_lines = Vec::new();

        for member_id in &module.members {
            let Some(node) = node_by_id.get(member_id) else {
                continue;
            };
            match node {
                GraphNode::Artifact(artifact_node) => {
                    if let Some(artifact) = artifact_by_path.get(artifact_node.path.as_str()) {
                        if let Some(excerpt) = read_excerpt(artifact, repo_root) {
                            sections.push(format!(
                                "### {} ({:?})\nEVIDENCE: {}\n```\n{}\n```\n",
                                artifact.path, artifact.category, artifact.path, excerpt.text
                            ));
                            excerpts.push(excerpt.record);
                        } else {
                            summary_lines.push(format!(
                                "- {} (no content included: {:?})",
                                artifact.path, artifact.model_policy
                            ));
                        }
                    }
                }
                GraphNode::Symbol(symbol) => {
                    let doc = symbol.doc.as_deref().unwrap_or("(no docstring)");
                    symbol_lines.push(format!(
                        "- {:?} `{}`: {}",
                        symbol.kind, symbol.qualified_name, doc
                    ));
                }
                GraphNode::Config(config) => {
                    summary_lines.push(format!("- config {:?} `{}`", config.kind, config.name));
                }
                GraphNode::Documentation(doc) => {
                    summary_lines.push(format!("- doc heading `{}`", doc.title));
                }
                GraphNode::Container(image) => {
                    summary_lines.push(format!("- container image `{}`", image.reference));
                }
                GraphNode::Command(command) => {
                    summary_lines.push(format!("- command `{}`", command.text));
                }
                GraphNode::EnvVar(env) => {
                    summary_lines.push(format!("- env var `{}`", env.name));
                }
                GraphNode::Package(package) => {
                    summary_lines.push(format!(
                        "- package `{}`{}",
                        package.name,
                        if package.is_external {
                            " (external)"
                        } else {
                            ""
                        }
                    ));
                }
                GraphNode::Unresolved(unresolved) => {
                    unresolved_lines.push(format!("- `{}`", unresolved.value));
                }
                GraphNode::Module(_) => {}
            }
        }

        if !symbol_lines.is_empty() {
            sections.push(format!("### Symbols\n{}\n", symbol_lines.join("\n")));
        }
        if !summary_lines.is_empty() {
            sections.push(format!("### Other members\n{}\n", summary_lines.join("\n")));
        }

        let relation_lines = relation_lines(graph, &member_set, MAX_RELATION_LINES);
        if !relation_lines.is_empty() {
            sections.push(format!(
                "### Relations (inbound and outbound)\n{}\n",
                relation_lines.join("\n")
            ));
        }
        if !unresolved_lines.is_empty() {
            sections.push(format!(
                "### Unresolved references\n{}\n",
                unresolved_lines.join("\n")
            ));
        }

        ModelContext {
            system_prompt: system_prompt(TaskKind::ModulePage),
            user_prompt: sections.join("\n"),
            excerpts,
            input_hash: module.input_hash.clone(),
            task_kind: TaskKind::ModulePage,
        }
    }

    /// Builds the repository-wide summary context shared by the quickstart
    /// and architecture pages: child module summaries and cross-module
    /// relations, never full member source.
    pub fn build_summary_context(
        &self,
        task_kind: TaskKind,
        modules: &[DocumentationModule],
        graph: &Graph,
    ) -> ModelContext {
        let owner_by_member: BTreeMap<&GraphNodeId, &str> = modules
            .iter()
            .flat_map(|module| {
                module
                    .members
                    .iter()
                    .map(move |member| (member, module.id.as_str()))
            })
            .collect();

        let mut sections = vec!["## Repository modules\n".to_owned()];
        for module in modules {
            sections.push(format!(
                "- `{}` ({:?}, {} members, ~{} tokens)",
                module.name,
                module.kind,
                module.members.len(),
                module.estimated_tokens
            ));
        }

        let mut cross_module_lines = Vec::new();
        for relation in &graph.relations {
            let Some(source_owner) = owner_by_member.get(&relation.source) else {
                continue;
            };
            let Some(target_owner) = owner_by_member.get(&relation.target) else {
                continue;
            };
            if source_owner == target_owner {
                continue;
            }
            cross_module_lines.push(format!(
                "- {source_owner} -[{:?}]-> {target_owner} ({:?})",
                relation.kind, relation.confidence
            ));
            if cross_module_lines.len() >= MAX_RELATION_LINES {
                cross_module_lines.push("- ... (truncated)".to_owned());
                break;
            }
        }
        if !cross_module_lines.is_empty() {
            sections.push(format!(
                "\n## Cross-module relations\n{}",
                cross_module_lines.join("\n")
            ));
        }

        let input_hash = summary_input_hash(modules);
        ModelContext {
            system_prompt: system_prompt(task_kind),
            user_prompt: sections.join("\n"),
            excerpts: Vec::new(),
            input_hash,
            task_kind,
        }
    }
}

struct ExcerptResult {
    text: String,
    record: ContextExcerpt,
}

fn read_excerpt(artifact: &Artifact, repo_root: &Path) -> Option<ExcerptResult> {
    if artifact.model_policy == ModelExposurePolicy::Never
        || artifact.text_status != TextStatus::Text
    {
        return None;
    }
    let content = std::fs::read_to_string(repo_root.join(artifact.path.as_str())).ok()?;
    let content = if artifact.model_policy == ModelExposurePolicy::Redacted {
        SafetyPolicy.redact_text(&content)
    } else {
        content
    };

    let total_lines = content.lines().count();
    let take = MAX_EXCERPT_LINES.min(total_lines);
    let text: String = content.lines().take(take).collect::<Vec<_>>().join("\n");
    let truncated = total_lines > take;

    Some(ExcerptResult {
        text,
        record: ContextExcerpt {
            artifact_path: artifact.path.as_str().to_owned(),
            policy: artifact.model_policy,
            included_lines: take,
            truncated,
        },
    })
}

fn relation_lines(
    graph: &Graph,
    member_set: &std::collections::BTreeSet<&GraphNodeId>,
    limit: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    for relation in &graph.relations {
        if !member_set.contains(&relation.source) && !member_set.contains(&relation.target) {
            continue;
        }
        lines.push(format!(
            "- {} -[{:?}]-> {} ({:?})",
            display_id(&relation.source),
            relation.kind,
            display_id(&relation.target),
            relation.confidence
        ));
        if lines.len() >= limit {
            lines.push("- ... (truncated)".to_owned());
            break;
        }
    }
    lines
}

fn display_id(id: &GraphNodeId) -> &str {
    id.as_str()
        .split_once(':')
        .map_or(id.as_str(), |(_, rest)| rest)
}

fn summary_input_hash(modules: &[DocumentationModule]) -> String {
    let mut hashes: Vec<&str> = modules
        .iter()
        .map(|module| module.input_hash.as_str())
        .collect();
    hashes.sort_unstable();
    blake3::hash(hashes.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

fn system_prompt(task_kind: TaskKind) -> String {
    let role = match task_kind {
        TaskKind::ModulePage => {
            "You write one Lithograph module documentation page from the module context below."
        }
        TaskKind::Architecture => {
            "You write the Lithograph repository architecture overview from the module summaries below."
        }
        TaskKind::Quickstart => {
            "You write the Lithograph repository quickstart page from the module summaries below."
        }
    };
    format!(
        "{role}\n\n\
         Respond with a single JSON object matching this schema exactly:\n\
         {{\"title\": string, \"summary\": string, \"evidence_refs\": string[], \
         \"unresolved_questions\": string[], \"body\": string (Markdown)}}\n\n\
         Rules:\n\
         - Only list a path in `evidence_refs` if it appears after an `EVIDENCE:` \
         line in the context below.\n\
         - Never invent file paths, line numbers, or facts not present in the context.\n\
         - If something is unclear from the context, add it to `unresolved_questions` \
         instead of guessing.\n\
         - `body` must be self-contained Markdown."
    )
}

#[cfg(test)]
mod tests {
    use super::ContextBuilder;
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, ModelExposurePolicy, RepoPath, SupportTier,
        TextStatus,
    };
    use crate::graph::GraphBuilder;
    use crate::graph::{
        ArtifactNode, EnvVarNode, Graph, GraphNode, GraphNodeId, Relation, RelationKind,
        UnresolvedNode,
    };
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use crate::manifest::TaskKind;
    use crate::plan::{DocumentationModule, ModuleKind, ModulePlanner};
    use std::path::Path;

    fn fixture_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot")
    }

    #[test]
    fn module_context_includes_excerpts_symbols_relations_and_unresolved()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);
        let python_module = modules
            .iter()
            .find(|module| module.kind == ModuleKind::PythonPackage)
            .ok_or("python module")?;

        let context = ContextBuilder.build_module_context(python_module, &graph, &artifacts, &root);

        assert!(
            context
                .user_prompt
                .contains("EVIDENCE: src/python_app/service.py")
        );
        assert!(context.user_prompt.contains("class RouteService"));
        assert!(context.user_prompt.contains("### Symbols"));
        assert!(context.user_prompt.contains("### Relations"));
        assert!(!context.excerpts.is_empty());
        assert_eq!(context.input_hash, python_module.input_hash);
        assert_eq!(context.task_kind, TaskKind::ModulePage);

        Ok(())
    }

    #[test]
    fn module_context_excludes_never_policy_artifacts_and_records_excerpts()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path().to_path_buf();
        std::fs::write(root.join("safe.txt"), "hello world\n")?;
        std::fs::write(
            root.join("README.md"),
            "sk-super-secret-token-do-not-leak\n",
        )?;
        let safe = Artifact::new(
            RepoPath::new("safe.txt")?,
            ArtifactCategory::Documentation,
            SupportTier::GenericText,
            ContentHash::new("abcdef")?,
            5,
        )
        .with_text_status(TextStatus::Text, Some(1));
        let secret = Artifact::new(
            RepoPath::new("README.md")?,
            ArtifactCategory::Configuration,
            SupportTier::GenericText,
            ContentHash::new("abcdef")?,
            5,
        )
        .with_text_status(TextStatus::UnsafeText, None)
        .with_model_policy(ModelExposurePolicy::Never);
        let artifacts = vec![safe.clone(), secret.clone()];

        let safe_id = GraphNodeId::new("artifact:safe.txt");
        let secret_id = GraphNodeId::new("artifact:README.md");
        let env_id = GraphNodeId::new("env:SECRET_TOKEN");
        let graph = Graph {
            nodes: vec![
                GraphNode::Artifact(ArtifactNode {
                    id: safe_id.clone(),
                    path: "safe.txt".to_owned(),
                    category: ArtifactCategory::Documentation,
                    evidence: file_evidence(&safe),
                }),
                GraphNode::Artifact(ArtifactNode {
                    id: secret_id.clone(),
                    path: "README.md".to_owned(),
                    category: ArtifactCategory::Configuration,
                    evidence: file_evidence(&secret),
                }),
                GraphNode::EnvVar(EnvVarNode {
                    id: env_id.clone(),
                    name: "SECRET_TOKEN".to_owned(),
                }),
                GraphNode::Unresolved(UnresolvedNode {
                    id: GraphNodeId::new("unresolved:mystery"),
                    value: "mystery".to_owned(),
                }),
            ],
            relations: vec![Relation {
                id: "relation:1".to_owned(),
                source: secret_id.clone(),
                target: env_id.clone(),
                kind: RelationKind::ReadsEnv,
                confidence: crate::domain::Confidence::High,
                evidence: vec![file_evidence(&secret)],
            }],
        };
        let module = DocumentationModule {
            id: "module-plan:directory:root".to_owned(),
            name: "root".to_owned(),
            kind: ModuleKind::Directory,
            members: vec![
                safe_id,
                secret_id,
                env_id,
                GraphNodeId::new("unresolved:mystery"),
            ],
            input_hash: "hash".to_owned(),
            estimated_tokens: 10,
        };

        let context = ContextBuilder.build_module_context(&module, &graph, &artifacts, &root);

        assert!(context.user_prompt.contains("hello world"));
        assert!(
            !context
                .user_prompt
                .contains("sk-super-secret-token-do-not-leak")
        );
        assert!(
            context
                .excerpts
                .iter()
                .all(|excerpt| excerpt.artifact_path != "README.md")
        );
        assert!(context.user_prompt.contains("### Unresolved references"));
        assert!(context.user_prompt.contains("mystery"));

        Ok(())
    }

    #[test]
    fn summary_context_lists_modules_and_cross_module_relations_without_source()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        let modules = ModulePlanner.plan(&graph, &artifacts);

        let context = ContextBuilder.build_summary_context(TaskKind::Quickstart, &modules, &graph);

        assert!(context.user_prompt.contains("## Repository modules"));
        assert!(context.excerpts.is_empty());
        assert!(!context.user_prompt.contains("```"));
        assert_eq!(context.task_kind, TaskKind::Quickstart);

        Ok(())
    }

    fn file_evidence(artifact: &Artifact) -> crate::domain::EvidenceRef {
        crate::domain::EvidenceRef::file(
            crate::domain::ArtifactId::from_path(&artifact.path),
            artifact.path.clone(),
        )
    }
}
