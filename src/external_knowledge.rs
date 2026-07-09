//! Extracts architecture decision facts from existing documentation and
//! ADRs, categorized so research and editor agents can request exactly
//! the decision categories they need (LIT-22.5.5).

use crate::adr::AdrStore;
use crate::domain::{Artifact, ArtifactCategory, Confidence, EvidenceRef};
use crate::graph::Graph;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One category of external architecture knowledge (AC1). A single
/// document can match more than one category (e.g. a combined
/// "database-api.md" page); each match becomes its own fact rather than
/// forcing one label per document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ExternalKnowledgeCategory {
    /// Architecture/design documentation.
    Architecture,
    /// Deployment/infrastructure/operations documentation.
    Deployment,
    /// Database/schema/migration documentation.
    Database,
    /// API/route/endpoint documentation.
    Api,
    /// A persisted architecture decision record.
    Adr,
}

/// One extracted external-knowledge fact (AC2): its category, text,
/// evidence, and confidence -- keyword-categorized documentation is
/// `Low` confidence (a heuristic guess at relevance), a real ADR record
/// is `High` (structured, deliberately authored).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalKnowledgeFact {
    /// Decision category this fact belongs to.
    pub category: ExternalKnowledgeCategory,
    /// Human-readable fact text.
    pub text: String,
    /// Evidence backing this fact.
    pub evidence: EvidenceRef,
    /// Confidence in the categorization.
    pub confidence: Confidence,
}

/// Extracts and categorizes external knowledge from documentation
/// artifacts and ADRs.
#[derive(Debug, Clone, Copy, Default)]
pub struct ExternalKnowledgeExtractor;

impl ExternalKnowledgeExtractor {
    /// Extracts every fact from `artifacts` (documentation, keyword-
    /// categorized by path) and every ADR persisted at `repo_root`
    /// (AC1/AC2). `graph` is accepted for symmetry with other research
    /// passes and future graph-derived signals; the current heuristic is
    /// path-based and doesn't need it yet.
    pub fn extract(
        &self,
        artifacts: &[Artifact],
        _graph: &Graph,
        repo_root: &Path,
    ) -> Vec<ExternalKnowledgeFact> {
        let mut facts: Vec<ExternalKnowledgeFact> = artifacts
            .iter()
            .filter(|artifact| artifact.category == ArtifactCategory::Documentation)
            .flat_map(|artifact| {
                categories_for_path(artifact.path.as_str())
                    .into_iter()
                    .map(move |category| ExternalKnowledgeFact {
                        category,
                        text: format!("documentation: {}", artifact.path),
                        evidence: EvidenceRef::file(
                            crate::domain::ArtifactId::from_path(&artifact.path),
                            artifact.path.clone(),
                        ),
                        confidence: Confidence::Low,
                    })
            })
            .collect();

        for summary in AdrStore::new(repo_root).list() {
            let Ok(relative_path) =
                crate::domain::RepoPath::new(format!(".lithograph/adrs/{}.json", summary.id))
            else {
                continue;
            };
            facts.push(ExternalKnowledgeFact {
                category: ExternalKnowledgeCategory::Adr,
                text: format!(
                    "ADR {} [{:?}]: {}",
                    summary.id, summary.status, summary.title
                ),
                evidence: EvidenceRef::file(
                    crate::domain::ArtifactId::from_path(&relative_path),
                    relative_path,
                ),
                confidence: Confidence::High,
            });
        }

        facts.sort_by(|a, b| {
            a.category
                .cmp(&b.category)
                .then_with(|| a.text.cmp(&b.text))
        });
        facts
    }

    /// Filters `facts` down to exactly the requested `categories` (AC3):
    /// the basic "request decision categories" hook a data-source router
    /// can call. Empty `categories` returns nothing, matching the same
    /// "never fabricate resolved facts" default used across every other
    /// research pass -- a caller must ask for something to get anything.
    pub fn request<'a>(
        &self,
        facts: &'a [ExternalKnowledgeFact],
        categories: &[ExternalKnowledgeCategory],
    ) -> Vec<&'a ExternalKnowledgeFact> {
        facts
            .iter()
            .filter(|fact| categories.contains(&fact.category))
            .collect()
    }
}

fn categories_for_path(path: &str) -> Vec<ExternalKnowledgeCategory> {
    let lower = path.to_lowercase();
    let mut categories = Vec::new();
    if lower.contains("architecture") || lower.contains("design") {
        categories.push(ExternalKnowledgeCategory::Architecture);
    }
    if lower.contains("deploy") || lower.contains("infra") || lower.contains("ops") {
        categories.push(ExternalKnowledgeCategory::Deployment);
    }
    if lower.contains("database") || lower.contains("schema") || lower.contains("migration") {
        categories.push(ExternalKnowledgeCategory::Database);
    }
    if lower.contains("api") || lower.contains("route") || lower.contains("endpoint") {
        categories.push(ExternalKnowledgeCategory::Api);
    }
    if lower.contains("adr") || lower.contains("decision") {
        categories.push(ExternalKnowledgeCategory::Adr);
    }
    categories
}

#[cfg(test)]
mod tests {
    use super::{ExternalKnowledgeCategory, ExternalKnowledgeExtractor};
    use crate::domain::Confidence;
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-22.5.5 AC1/AC2/AC4: architecture/deployment/database/API/ADR
    /// documentation is categorized, with real evidence and appropriate
    /// confidence.
    #[test]
    fn categorizes_documentation_by_path_with_evidence_and_confidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("docs"))?;
        std::fs::write(temp.path().join("docs/architecture.md"), "# Architecture\n")?;
        std::fs::write(temp.path().join("docs/deployment.md"), "# Deployment\n")?;
        std::fs::write(temp.path().join("docs/database-schema.md"), "# Schema\n")?;
        std::fs::write(temp.path().join("docs/api-routes.md"), "# API\n")?;
        std::fs::write(temp.path().join("docs/adr-0001.md"), "# ADR 1\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let facts = ExternalKnowledgeExtractor.extract(&artifacts, &graph, temp.path());

        for category in [
            ExternalKnowledgeCategory::Architecture,
            ExternalKnowledgeCategory::Deployment,
            ExternalKnowledgeCategory::Database,
            ExternalKnowledgeCategory::Api,
            ExternalKnowledgeCategory::Adr,
        ] {
            assert!(
                facts.iter().any(|fact| fact.category == category),
                "missing category {category:?}"
            );
        }
        assert!(facts.iter().all(|fact| fact.confidence == Confidence::Low));
        assert!(
            facts
                .iter()
                .all(|fact| !fact.evidence.path.as_str().is_empty())
        );

        Ok(())
    }

    /// LIT-22.5.5 AC1/AC2/AC4: a real persisted ADR extracts as a
    /// High-confidence Adr-category fact.
    #[test]
    fn extracts_real_adrs_as_high_confidence_facts() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        crate::adr::AdrStore::new(temp.path()).create(
            "Use blake3 for content hashing",
            "Need a fast, stable content hash.",
            "Adopt blake3.",
            None,
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let facts = ExternalKnowledgeExtractor.extract(&artifacts, &graph, temp.path());

        let adr_fact = facts
            .iter()
            .find(|fact| fact.category == ExternalKnowledgeCategory::Adr)
            .ok_or("expected an ADR fact")?;
        assert_eq!(adr_fact.confidence, Confidence::High);
        assert!(adr_fact.text.contains("Use blake3 for content hashing"));

        Ok(())
    }

    /// LIT-22.5.5 AC3/AC4: `request` filters facts down to exactly the
    /// requested categories, and an empty category list returns nothing.
    #[test]
    fn request_filters_to_requested_categories() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("docs"))?;
        std::fs::write(temp.path().join("docs/architecture.md"), "# Architecture\n")?;
        std::fs::write(temp.path().join("docs/database-schema.md"), "# Schema\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let facts = ExternalKnowledgeExtractor.extract(&artifacts, &graph, temp.path());

        let database_only =
            ExternalKnowledgeExtractor.request(&facts, &[ExternalKnowledgeCategory::Database]);
        assert!(!database_only.is_empty());
        assert!(
            database_only
                .iter()
                .all(|fact| fact.category == ExternalKnowledgeCategory::Database)
        );

        let none = ExternalKnowledgeExtractor.request(&facts, &[]);
        assert!(none.is_empty());

        Ok(())
    }

    /// LIT-22.5.5 AC4: a repository with no documentation and no ADRs
    /// extracts to an empty fact list, never fabricated categories.
    #[test]
    fn no_documentation_or_adrs_extracts_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(temp.path().join("lib.rs"), "pub fn noop() {}\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let facts = ExternalKnowledgeExtractor.extract(&artifacts, &graph, temp.path());

        assert!(facts.is_empty());

        Ok(())
    }
}
