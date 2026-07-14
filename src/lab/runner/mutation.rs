//! Mutation-style diagnostics: expectation-preserving repository scenarios
//! (rename/replace/move/rewrite operations replayed against a cached fixture
//! to assert incremental equivalence) and bounded generated-Python fuzzing of
//! the parser/graph builder with automatic failure shrinking.

use super::{Lab, LabError, evaluate};
use crate::graph::{CommunityScope, GraphValidator, leiden_communities};
use crate::inventory::{RepositoryWalker, WalkOptions};
use crate::lab::model::{AssertionResult, CorpusCase, ExpectationSet, ScenarioOperation};
use std::path::Path;

impl Lab {
    /// Replays every scenario for a case against a fresh copy of its
    /// repository, asserting the mutation either preserves expectations (or,
    /// for identity-changing scenarios, still yields a valid graph) and that
    /// an incrementally cached rebuild equals a clean one.
    pub(super) fn run_scenarios(
        &self,
        case: &CorpusCase,
        source: &Path,
        expectations: &ExpectationSet,
    ) -> Result<Vec<AssertionResult>, LabError> {
        let mut results = Vec::new();
        for scenario in &expectations.scenarios {
            let root = self
                .root
                .join("work/scenarios")
                .join(&case.id)
                .join(&scenario.id);
            if root.exists() {
                std::fs::remove_dir_all(&root)?;
            }
            copy_repository(source, &root)?;
            apply_scenario(&root, &scenario.operation)?;
            let artifacts = RepositoryWalker::new(WalkOptions {
                exclude_globs: case.exclude.clone(),
                ..WalkOptions::default()
            })
            .walk(&root)?;
            let graph = crate::graph::GraphBuilder.build(&root, &artifacts);
            let cache = crate::analysis::AnalysisCache::new(
                self.root
                    .join("work/scenario-cache")
                    .join(&case.id)
                    .join(&scenario.id),
            );
            let source_artifacts = RepositoryWalker::new(WalkOptions {
                exclude_globs: case.exclude.clone(),
                ..WalkOptions::default()
            })
            .walk(source)?;
            let _seed = crate::graph::GraphBuilder.build_with_cache(
                source,
                &source_artifacts,
                Some(&cache),
            );
            let incremental =
                crate::graph::GraphBuilder.build_with_cache(&root, &artifacts, Some(&cache));
            let communities = leiden_communities(&graph, &CommunityScope::Combined);
            let scenario_results = if scenario.preserve_expectations {
                evaluate(&expectations.expectations, &artifacts, &graph, &communities)
            } else {
                let issues = GraphValidator.validate(&graph, &artifacts);
                vec![AssertionResult {
                    id: "transformed-graph-valid".to_owned(),
                    passed: issues.is_empty(),
                    stage: "finalize".to_owned(),
                    detail: format!(
                        "identity-changing scenario requires a valid transformed graph; issues={issues:?}"
                    ),
                    expected_failure: None,
                }]
            };
            results.extend(scenario_results.into_iter().map(|result| AssertionResult {
                id: format!("scenario:{}:{}", scenario.id, result.id),
                passed: result.passed,
                stage: result.stage,
                detail: format!("scenario `{}`: {}", scenario.id, result.detail),
                expected_failure: result.expected_failure,
            }));
            results.push(AssertionResult {
                id: format!("scenario:{}:incremental-equivalence", scenario.id),
                passed: graph == incremental,
                stage: "finalize".to_owned(),
                detail: format!(
                    "scenario `{}` expected incremental cached build to equal a clean rebuild; equivalent={}, cache_hits={}, cache_misses={}",
                    scenario.id,
                    graph == incremental,
                    cache.hits(),
                    cache.misses()
                ),
                expected_failure: None,
            });
        }
        Ok(results)
    }

    /// Generates 32 deterministic bounded Python inputs and asserts the
    /// parser/graph builder never raises a validation issue on any of them;
    /// on failure, shrinks the failing input to a minimal reproduction.
    pub(super) fn generated_parser_robustness(&self) -> Result<AssertionResult, LabError> {
        let root = self.root.join("work/generated-parser-inputs");
        if root.exists() {
            std::fs::remove_dir_all(&root)?;
        }
        std::fs::create_dir_all(&root)?;
        for seed in 0..32u32 {
            let content = generated_python(seed);
            let path = root.join(format!("case_{seed}.py"));
            std::fs::write(&path, &content)?;
            let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
            let graph = crate::graph::GraphBuilder.build(&root, &artifacts);
            let issues = GraphValidator.validate(&graph, &artifacts);
            if !issues.is_empty() {
                let minimized = shrink_generated_python(&root, &content, &issues)?;
                return Ok(AssertionResult {
                    id: "generated-parser-robustness".to_owned(),
                    passed: false,
                    stage: "definitions_and_imports".to_owned(),
                    detail: format!(
                        "bounded generated input failed; seed={seed}; minimized_input={minimized:?}; issues={issues:?}"
                    ),
                    expected_failure: None,
                });
            }
            std::fs::remove_file(path)?;
        }
        Ok(AssertionResult {
            id: "generated-parser-robustness".to_owned(),
            passed: true,
            stage: "definitions_and_imports".to_owned(),
            detail: "32 deterministic bounded Python inputs passed; failure reports retain seed and minimized input"
                .to_owned(),
            expected_failure: None,
        })
    }
}

fn copy_repository(source: &Path, destination: &Path) -> Result<(), LabError> {
    std::fs::create_dir_all(destination)?;
    let mut directories = vec![source.to_path_buf()];
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let entry = entry?;
            let path = entry.path();
            if entry.file_name() == ".git" || entry.file_name() == ".lithograph" {
                continue;
            }
            let target = destination.join(path.strip_prefix(source).map_err(|error| {
                LabError::Invalid(format!("cannot copy scenario repository: {error}"))
            })?);
            if path.is_dir() {
                std::fs::create_dir_all(&target)?;
                directories.push(path);
            } else {
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(path, target)?;
            }
        }
    }
    Ok(())
}

fn apply_scenario(root: &Path, operation: &ScenarioOperation) -> Result<(), LabError> {
    match operation {
        ScenarioOperation::AppendComment { path, text } => {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(root.join(path))?;
            writeln!(file, "{text}")?;
        }
        ScenarioOperation::AddFile { path, content } => {
            let target = root.join(path);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(target, content)?;
        }
        ScenarioOperation::RenameFile { from, to } => {
            let target = root.join(to);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(root.join(from), target)?;
        }
        ScenarioOperation::ReplaceText { path, from, to } => {
            replace_fixture_text(&root.join(path), from, to)?;
        }
        ScenarioOperation::MoveFileAndReplace {
            from,
            to,
            update_path,
            old,
            new,
        } => {
            let target = root.join(to);
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::rename(root.join(from), target)?;
            replace_fixture_text(&root.join(update_path), old, new)?;
        }
        ScenarioOperation::PrependText { path, text } => {
            let target = root.join(path);
            let current = std::fs::read_to_string(&target)?;
            std::fs::write(target, format!("{text}{current}"))?;
        }
        ScenarioOperation::RewriteFile { path, content } => {
            std::fs::write(root.join(path), content)?;
        }
    }
    Ok(())
}

fn generated_python(seed: u32) -> String {
    let decorators = if seed.is_multiple_of(3) {
        "@staticmethod\n    "
    } else {
        ""
    };
    let annotation = if seed.is_multiple_of(2) {
        " -> int"
    } else {
        ""
    };
    let body = match seed % 4 {
        0 => "return value + 1",
        1 => "return sum(item for item in value if item)",
        2 => "match value:\n            case 0: return 0\n            case _: return 1",
        _ => {
            "try:\n            return value[0]\n        except (IndexError, TypeError):\n            return None"
        }
    };
    format!(
        "class Generated{seed}:\n    {decorators}def evaluate(value){annotation}:\n        {body}\n"
    )
}

fn shrink_generated_python(
    root: &Path,
    content: &str,
    original: &[crate::graph::GraphIssue],
) -> Result<String, LabError> {
    let signature = original.iter().map(|issue| issue.kind).collect::<Vec<_>>();
    let mut lines = content.lines().map(str::to_owned).collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let mut candidate = lines.clone();
        candidate.remove(index);
        let candidate_text = format!("{}\n", candidate.join("\n"));
        let path = root.join("shrink.py");
        std::fs::write(&path, &candidate_text)?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let graph = crate::graph::GraphBuilder.build(root, &artifacts);
        let observed = GraphValidator
            .validate(&graph, &artifacts)
            .iter()
            .map(|issue| issue.kind)
            .collect::<Vec<_>>();
        std::fs::remove_file(path)?;
        if observed == signature {
            lines = candidate;
        } else {
            index += 1;
        }
    }
    Ok(format!("{}\n", lines.join("\n")))
}

fn replace_fixture_text(path: &Path, from: &str, to: &str) -> Result<(), LabError> {
    let current = std::fs::read_to_string(path)?;
    if !current.contains(from) {
        return Err(LabError::Invalid(format!(
            "scenario replacement `{from}` was not found in {}",
            path.display()
        )));
    }
    std::fs::write(path, current.replace(from, to))?;
    Ok(())
}
