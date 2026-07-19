//! Architecture layer detection (LIT-22.5.2): classifies each artifact into
//! a coarse architecture layer (UI, API, domain, data, infra, test, or
//! unknown) from path keywords, declared artifact category, and graph role
//! (e.g. an attached HTTP/RPC route), each with a human-readable reason and
//! an honest confidence -- a classification with no real evidence stays
//! `Unknown` at `Confidence::Low` rather than guessing.

use crate::domain::{ArtifactCategory, Confidence};
use crate::graph::{ConfigNodeKind, Graph, GraphNode, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Coarse architecture layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LayerKind {
    /// User-facing presentation code: components, views, templates.
    Ui,
    /// API surface: routes, controllers, handlers, RPC/GraphQL schemas.
    Api,
    /// Core business/domain logic: services, models, entities.
    Domain,
    /// Data access: repositories, schemas, migrations.
    Data,
    /// Build, deploy, and operational tooling: CI, containers, IaC.
    Infra,
    /// Tests and test fixtures.
    Test,
    /// No path, category, or graph-role evidence matched a known layer.
    Unknown,
}

/// One artifact's layer classification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArchitectureLayer {
    /// Repository-relative artifact path.
    pub artifact_path: String,
    /// Classified layer.
    pub layer: LayerKind,
    /// Confidence in this classification (AC3: `Unknown` and graph-role-only
    /// matches are `Low`; path- and category-based matches are `High`).
    pub confidence: Confidence,
    /// Human-readable reason for the classification.
    pub reason: String,
}

/// Deterministic, evidence-backed architecture layer detector.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct LayerDetector;

/// Path components checked in priority order (AC4: ambiguous cases --
/// a path matching more than one layer's keywords, e.g. `tests/api/`,
/// resolves to whichever layer is checked first here). Test-ness is the
/// most specific, least-ambiguous signal a path can carry, so it's checked
/// first; `Domain` is the vaguest catch-all bucket, so it's checked last
/// before falling back to `Unknown`.
const PATH_LAYER_RULES: &[(LayerKind, &[&str])] = &[
    (
        LayerKind::Test,
        &[
            "test",
            "tests",
            "__tests__",
            "spec",
            "specs",
            "fixtures",
            "testdata",
        ],
    ),
    (
        LayerKind::Infra,
        &[
            "infra",
            "infrastructure",
            "deploy",
            "deployment",
            "docker",
            "k8s",
            "kubernetes",
            "terraform",
            "ci",
            ".github",
            "scripts",
        ],
    ),
    (
        LayerKind::Data,
        &[
            "data",
            "db",
            "database",
            "repositories",
            "repository",
            "migrations",
            "dao",
            "schema",
            "schemas",
        ],
    ),
    (
        LayerKind::Api,
        &[
            "api",
            "routes",
            "route",
            "controllers",
            "controller",
            "handlers",
            "handler",
            "endpoints",
            "rpc",
            "graphql",
        ],
    ),
    (
        LayerKind::Ui,
        &[
            "ui",
            "components",
            "component",
            "views",
            "view",
            "pages",
            "page",
            "frontend",
            "web",
            "templates",
            "template",
        ],
    ),
    (
        LayerKind::Domain,
        &[
            "domain", "models", "model", "entities", "entity", "services", "service", "core",
        ],
    ),
];

impl LayerDetector {
    /// Classifies every `Artifact` node in `graph`.
    pub(crate) fn detect(&self, graph: &Graph) -> Vec<ArchitectureLayer> {
        let routed_artifacts = artifacts_with_routes(graph);
        let mut layers: Vec<ArchitectureLayer> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Artifact(artifact) => Some(classify(artifact, &routed_artifacts)),
                _ => None,
            })
            .collect();
        layers.sort_by(|a, b| a.artifact_path.cmp(&b.artifact_path));
        layers
    }
}

fn classify(
    artifact: &crate::graph::ArtifactNode,
    routed_artifacts: &BTreeSet<&str>,
) -> ArchitectureLayer {
    let path_lower = artifact.path.to_lowercase();
    if let Some((layer, keyword)) = PATH_LAYER_RULES.iter().find_map(|(layer, keywords)| {
        keywords
            .iter()
            .find(|keyword| has_path_component(&path_lower, keyword))
            .map(|keyword| (*layer, *keyword))
    }) {
        return ArchitectureLayer {
            artifact_path: artifact.path.clone(),
            layer,
            confidence: Confidence::High,
            reason: format!("path contains `{keyword}`"),
        };
    }

    if let Some(layer) = layer_for_category(artifact.category) {
        return ArchitectureLayer {
            artifact_path: artifact.path.clone(),
            layer,
            confidence: Confidence::High,
            reason: format!("artifact category is {:?}", artifact.category),
        };
    }

    if routed_artifacts.contains(artifact.path.as_str()) {
        return ArchitectureLayer {
            artifact_path: artifact.path.clone(),
            layer: LayerKind::Api,
            confidence: Confidence::Low,
            reason: "declares an HTTP/RPC/GraphQL route in the graph".to_owned(),
        };
    }

    ArchitectureLayer {
        artifact_path: artifact.path.clone(),
        layer: LayerKind::Unknown,
        confidence: Confidence::Low,
        reason: "no path, category, or graph-role evidence matched a known layer".to_owned(),
    }
}

/// Matches `keyword` as a whole path component (`/`-delimited segment or
/// filename), not an arbitrary substring -- so `"apiary.py"` doesn't match
/// `"api"`. Directory segments (`visual-tests`) and file stems
/// (`graph.spec.ts`, post extension-strip) share the same affix rule.
fn has_path_component(path_lower: &str, keyword: &str) -> bool {
    path_lower.split('/').any(|segment| {
        matches_component(segment, keyword)
            || segment
                .strip_suffix(".py")
                .or_else(|| segment.strip_suffix(".rs"))
                .or_else(|| segment.strip_suffix(".ts"))
                .or_else(|| segment.strip_suffix(".tsx"))
                .or_else(|| segment.strip_suffix(".js"))
                .or_else(|| segment.strip_suffix(".go"))
                .is_some_and(|stem| matches_component(stem, keyword))
    })
}

/// Matches `component` (a directory segment or an extension-stripped file
/// stem) against `keyword` exactly, or with `keyword` set off by a
/// `_`/`-`/`.` delimiter -- so `visual-tests`, `e2e_tests`, and
/// `graph.spec` (from `graph.spec.ts`) all match `tests`/`spec`, while a
/// bare substring like `contests` or `webpack` never matches `test`/`web`.
fn matches_component(component: &str, keyword: &str) -> bool {
    component == keyword
        || ["_", "-", "."].iter().any(|delimiter| {
            component.starts_with(&format!("{keyword}{delimiter}"))
                || component.ends_with(&format!("{delimiter}{keyword}"))
        })
}

fn layer_for_category(category: ArtifactCategory) -> Option<LayerKind> {
    match category {
        ArtifactCategory::ContainerDefinition
        | ArtifactCategory::DeploymentDefinition
        | ArtifactCategory::ContinuousIntegration
        | ArtifactCategory::BuildDefinition
        | ArtifactCategory::PackageManifest
        | ArtifactCategory::DependencyLockfile => Some(LayerKind::Infra),
        ArtifactCategory::DatabaseSchema | ArtifactCategory::DatabaseMigration => {
            Some(LayerKind::Data)
        }
        ArtifactCategory::Template => Some(LayerKind::Ui),
        ArtifactCategory::TestData => Some(LayerKind::Test),
        _ => None,
    }
}

/// Repository-relative paths of every artifact with a `Route` config node
/// attached via a `Contains` relation (LIT-22.3.4).
fn artifacts_with_routes(graph: &Graph) -> BTreeSet<&str> {
    let route_node_ids: BTreeSet<&crate::graph::GraphNodeId> = graph
        .nodes
        .iter()
        .filter_map(|node| match node {
            GraphNode::Config(config) if config.kind == ConfigNodeKind::Route => Some(node.id()),
            _ => None,
        })
        .collect();
    graph
        .relations
        .iter()
        .filter(|relation| {
            relation.kind == RelationKind::Contains && route_node_ids.contains(&relation.target)
        })
        .filter_map(|relation| relation.source.as_str().strip_prefix("artifact:"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{LayerDetector, LayerKind};
    use crate::domain::Confidence;
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    #[test]
    fn path_based_classification_covers_every_layer_family()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        for (path, contents) in [
            (
                "src/components/Button.tsx",
                "export const Button = () => null;\n",
            ),
            ("src/api/routes.ts", "export const routes = [];\n"),
            ("src/domain/models/user.py", "class User:\n    pass\n"),
            (
                "src/data/repositories/user_repo.py",
                "class UserRepo:\n    pass\n",
            ),
            (".github/workflows/ci.yml", "name: CI\non: push\njobs: {}\n"),
            ("tests/test_user.py", "def test_user():\n    pass\n"),
            ("README.md", "# Hello\n"),
        ] {
            let full = temp.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(full, contents)?;
        }

        let artifacts = RepositoryWalker::new(WalkOptions {
            include_hidden_directories: true,
            include_tests: true,
            ..WalkOptions::default()
        })
        .walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let layers = LayerDetector.detect(&graph);

        let layer_of = |path: &str| {
            layers
                .iter()
                .find(|layer| layer.artifact_path == path)
                .map(|layer| layer.layer)
        };
        assert_eq!(layer_of("src/components/Button.tsx"), Some(LayerKind::Ui));
        assert_eq!(layer_of("src/api/routes.ts"), Some(LayerKind::Api));
        assert_eq!(
            layer_of("src/domain/models/user.py"),
            Some(LayerKind::Domain)
        );
        assert_eq!(
            layer_of("src/data/repositories/user_repo.py"),
            Some(LayerKind::Data)
        );
        assert_eq!(layer_of(".github/workflows/ci.yml"), Some(LayerKind::Infra));
        assert_eq!(layer_of("tests/test_user.py"), Some(LayerKind::Test));
        assert_eq!(layer_of("README.md"), Some(LayerKind::Unknown));

        for layer in &layers {
            if layer.layer == LayerKind::Unknown {
                assert_eq!(layer.confidence, Confidence::Low);
            } else {
                assert_eq!(layer.confidence, Confidence::High);
            }
            assert!(!layer.reason.is_empty());
        }

        Ok(())
    }

    /// LIT-24.50: a hyphenated test-suite directory and a dot-delimited
    /// spec filename both classify as `Test` even though neither exactly
    /// equals a keyword, while a plain `ui`-rooted component path and
    /// adversarial substring-only lookalikes (`contests/`, `apiary.py`,
    /// `webpack.config.js`) are unaffected.
    #[test]
    fn affix_delimited_paths_classify_without_over_matching()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        for (path, contents) in [
            ("ui/visual-tests/graph-readability.spec.ts", "export {}\n"),
            (
                "ui/src/testdata/graphFixtures.ts",
                "export const fixtures = [];\n",
            ),
            (
                "ui/src/components/Button.tsx",
                "export const Button = () => null;\n",
            ),
            ("contests/leaderboard.py", "leaderboard = []\n"),
            ("apiary.py", "print('not an api')\n"),
            ("webpack.config.js", "module.exports = {};\n"),
        ] {
            let full = temp.path().join(path);
            if let Some(parent) = full.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(full, contents)?;
        }

        let artifacts = RepositoryWalker::new(WalkOptions {
            include_tests: true,
            ..WalkOptions::default()
        })
        .walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let layers = LayerDetector.detect(&graph);

        let layer_of = |path: &str| {
            layers
                .iter()
                .find(|layer| layer.artifact_path == path)
                .map(|layer| layer.layer)
        };
        assert_eq!(
            layer_of("ui/visual-tests/graph-readability.spec.ts"),
            Some(LayerKind::Test)
        );
        assert_eq!(
            layer_of("ui/src/testdata/graphFixtures.ts"),
            Some(LayerKind::Test)
        );
        assert_eq!(
            layer_of("ui/src/components/Button.tsx"),
            Some(LayerKind::Ui)
        );
        assert_ne!(layer_of("contests/leaderboard.py"), Some(LayerKind::Test));
        assert_ne!(layer_of("apiary.py"), Some(LayerKind::Api));
        assert_ne!(layer_of("webpack.config.js"), Some(LayerKind::Ui));

        Ok(())
    }

    /// LIT-22.5.2 AC4: an ambiguous path (matches both `tests` and `api`
    /// keywords) resolves to `Test`, the documented priority order.
    #[test]
    fn ambiguous_path_resolves_by_documented_priority_order()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("tests/api"))?;
        std::fs::write(
            temp.path().join("tests/api/test_routes.py"),
            "def test_routes():\n    pass\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions {
            include_tests: true,
            ..WalkOptions::default()
        })
        .walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let layers = LayerDetector.detect(&graph);

        let layer = layers
            .iter()
            .find(|layer| layer.artifact_path == "tests/api/test_routes.py")
            .ok_or("missing layer for ambiguous path")?;
        assert_eq!(layer.layer, LayerKind::Test);

        Ok(())
    }

    /// LIT-22.5.2 AC1: a Python HTTP route decorator with no path/category
    /// evidence is still classified `Api` from graph role alone, at `Low`
    /// confidence (AC3).
    #[test]
    fn graph_role_alone_classifies_a_route_handler_as_api_at_low_confidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("main.py"),
            "@app.get(\"/users/{id}\")\ndef get_user(id):\n    return None\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let layers = LayerDetector.detect(&graph);

        let layer = layers
            .iter()
            .find(|layer| layer.artifact_path == "main.py")
            .ok_or("missing layer for main.py")?;
        assert_eq!(layer.layer, LayerKind::Api);
        assert_eq!(layer.confidence, Confidence::Low);

        Ok(())
    }
}
