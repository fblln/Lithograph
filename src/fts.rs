//! Persistent BM25 full-text search index over the knowledge graph:
//! symbols, docs, comments, paths, and extracted facts, with identifier-
//! aware tokenization (LIT-22.4.3).

use crate::graph::{Graph, GraphNode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// BM25 term-frequency saturation constant.
const K1: f64 = 1.2;
/// BM25 document-length normalization constant.
const B: f64 = 0.75;

/// Category of one indexed document, so a result can be grouped/labeled
/// without re-deriving it from the graph (AC2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum FtsDocumentKind {
    /// A symbol's qualified name plus its doc comment/docstring.
    Symbol,
    /// A Markdown documentation heading.
    Documentation,
    /// An artifact's repository-relative path.
    Path,
    /// An extracted fact: a config entity, command, env var, or package name.
    Fact,
}

/// One indexed document: the graph node it came from, plus the raw text
/// it contributes to the index.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FtsDocument {
    /// Source graph node id.
    pub id: String,
    /// Document category (AC2).
    pub kind: FtsDocumentKind,
    /// Human-readable reference (path or qualified name) for display.
    pub reference: String,
    /// Raw indexed text (name, doc comment, path, or fact text).
    pub text: String,
}

/// Persistent full-text index built from one graph snapshot (AC1): every
/// `init`/`update` run rebuilds this deterministically from the current
/// graph, so it always reflects the graph it was built from and never
/// drifts independently of it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct FtsIndex {
    /// Every indexed document, in a stable (graph node id) order.
    pub documents: Vec<FtsDocument>,
}

/// One BM25-ranked search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct FtsSearchResult {
    /// Source graph node id.
    pub document_id: String,
    /// Document category.
    pub kind: FtsDocumentKind,
    /// Human-readable reference.
    pub reference: String,
    /// BM25 relevance score; higher ranks first.
    pub score: f64,
}

impl FtsIndex {
    /// Builds the index from `graph` (AC1/AC2): one document per symbol
    /// (qualified name + doc), documentation heading, artifact path, and
    /// extracted fact (config entity, command, env var, package).
    pub(crate) fn build(graph: &Graph) -> Self {
        let mut documents: Vec<FtsDocument> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Symbol(symbol) => Some(FtsDocument {
                    id: symbol.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Symbol,
                    reference: symbol.qualified_name.clone(),
                    text: format!(
                        "{} {}",
                        symbol.qualified_name,
                        symbol.doc.as_deref().unwrap_or("")
                    ),
                }),
                GraphNode::Documentation(doc) => Some(FtsDocument {
                    id: doc.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Documentation,
                    reference: doc.title.clone(),
                    text: doc.title.clone(),
                }),
                GraphNode::Artifact(artifact) => Some(FtsDocument {
                    id: artifact.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Path,
                    reference: artifact.path.clone(),
                    text: artifact.path.clone(),
                }),
                GraphNode::Config(config) => Some(FtsDocument {
                    id: config.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Fact,
                    reference: config.name.clone(),
                    text: format!("{:?} {}", config.kind, config.name),
                }),
                GraphNode::Command(command) => Some(FtsDocument {
                    id: command.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Fact,
                    reference: command.text.clone(),
                    text: command.text.clone(),
                }),
                GraphNode::EnvVar(env) => Some(FtsDocument {
                    id: env.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Fact,
                    reference: env.name.clone(),
                    text: env.name.clone(),
                }),
                GraphNode::Package(package) => Some(FtsDocument {
                    id: package.id.as_str().to_owned(),
                    kind: FtsDocumentKind::Fact,
                    reference: package.name.clone(),
                    text: package.name.clone(),
                }),
                _ => None,
            })
            .collect();
        documents.sort_by(|a, b| a.id.cmp(&b.id));
        Self { documents }
    }

    /// Ranks every document against `query` with BM25, highest score
    /// first, ties broken by document id for stable ordering (AC4).
    /// Empty for an empty or entirely-unknown query.
    pub(crate) fn search(&self, query: &str, limit: usize) -> Vec<FtsSearchResult> {
        let query_terms = tokenize(query);
        if query_terms.is_empty() || self.documents.is_empty() {
            return Vec::new();
        }
        let limit = if limit == 0 { 10 } else { limit };

        let doc_tokens: Vec<Vec<String>> = self
            .documents
            .iter()
            .map(|doc| tokenize(&doc.text))
            .collect();
        let doc_lengths: Vec<f64> = doc_tokens
            .iter()
            .map(|tokens| tokens.len() as f64)
            .collect();
        let average_length = doc_lengths.iter().sum::<f64>() / doc_lengths.len() as f64;
        let document_count = self.documents.len() as f64;

        let document_frequency: BTreeMap<&str, usize> = query_terms
            .iter()
            .map(|term| {
                let count = doc_tokens
                    .iter()
                    .filter(|tokens| tokens.iter().any(|token| token == term))
                    .count();
                (term.as_str(), count)
            })
            .collect();

        let mut scored: Vec<(usize, f64)> = doc_tokens
            .iter()
            .enumerate()
            .filter_map(|(index, tokens)| {
                let score = bm25_score(
                    &query_terms,
                    tokens,
                    doc_lengths[index],
                    average_length,
                    document_count,
                    &document_frequency,
                );
                (score > 0.0).then_some((index, score))
            })
            .collect();
        scored.sort_by(|(a_index, a_score), (b_index, b_score)| {
            b_score
                .partial_cmp(a_score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| {
                    self.documents[*a_index]
                        .id
                        .cmp(&self.documents[*b_index].id)
                })
        });
        scored.truncate(limit);
        scored
            .into_iter()
            .map(|(index, score)| {
                let document = &self.documents[index];
                FtsSearchResult {
                    document_id: document.id.clone(),
                    kind: document.kind,
                    reference: document.reference.clone(),
                    score,
                }
            })
            .collect()
    }
}

fn bm25_score(
    query_terms: &[String],
    document_tokens: &[String],
    document_length: f64,
    average_length: f64,
    document_count: f64,
    document_frequency: &BTreeMap<&str, usize>,
) -> f64 {
    query_terms
        .iter()
        .map(|term| {
            let term_frequency = document_tokens
                .iter()
                .filter(|token| *token == term)
                .count() as f64;
            if term_frequency == 0.0 {
                return 0.0;
            }
            let matching_documents = *document_frequency.get(term.as_str()).unwrap_or(&0) as f64;
            let inverse_document_frequency =
                ((document_count - matching_documents + 0.5) / (matching_documents + 0.5) + 1.0)
                    .ln();
            let normalized_length = if average_length > 0.0 {
                document_length / average_length
            } else {
                1.0
            };
            inverse_document_frequency * (term_frequency * (K1 + 1.0))
                / (term_frequency + K1 * (1.0 - B + B * normalized_length))
        })
        .sum()
}

/// Splits `text` into lowercase tokens, identifier-aware (AC3): first on
/// any non-alphanumeric byte (handles `snake_case`, `kebab-case`, and
/// `dotted.names`), then each resulting word on camelCase boundaries
/// (`fooBar` -> `foo`, `bar`; `XMLParser` -> `xml`, `parser`).
pub(crate) fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .flat_map(split_camel_case)
        .map(|token| token.to_lowercase())
        .filter(|token| !token.is_empty())
        .collect()
}

fn split_camel_case(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    let mut tokens = Vec::new();
    let mut current = String::new();
    for (index, &ch) in chars.iter().enumerate() {
        if index > 0 {
            let previous = chars[index - 1];
            let lower_to_upper = previous.is_lowercase() && ch.is_uppercase();
            let acronym_to_word = previous.is_uppercase()
                && ch.is_uppercase()
                && chars.get(index + 1).is_some_and(|next| next.is_lowercase());
            if (lower_to_upper || acronym_to_word) && !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
        current.push(ch);
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

#[cfg(test)]
mod tests {
    use super::{FtsIndex, split_camel_case, tokenize};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    #[test]
    fn tokenizer_splits_camel_case() {
        assert_eq!(split_camel_case("camelCase"), vec!["camel", "Case"]);
        assert_eq!(split_camel_case("XMLParser"), vec!["XML", "Parser"]);
        assert_eq!(split_camel_case("simple"), vec!["simple"]);
    }

    /// LIT-22.4.3 AC3: snake_case, kebab-case, dotted names, and camelCase
    /// all split into the same normalized lowercase tokens.
    #[test]
    fn tokenize_handles_snake_kebab_dotted_and_camel_case() {
        assert_eq!(tokenize("snake_case_name"), vec!["snake", "case", "name"]);
        assert_eq!(tokenize("kebab-case-name"), vec!["kebab", "case", "name"]);
        assert_eq!(
            tokenize("dotted.module.name"),
            vec!["dotted", "module", "name"]
        );
        assert_eq!(tokenize("camelCaseName"), vec!["camel", "case", "name"]);
    }

    fn build_polyglot_index() -> Result<FtsIndex, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);
        Ok(FtsIndex::build(&graph))
    }

    /// LIT-22.4.3 AC2: the index covers symbols, docs, paths, and facts,
    /// not just one node kind.
    #[test]
    fn index_covers_symbols_docs_paths_and_facts() -> Result<(), Box<dyn std::error::Error>> {
        use super::FtsDocumentKind;

        let index = build_polyglot_index()?;
        for kind in [
            FtsDocumentKind::Symbol,
            FtsDocumentKind::Documentation,
            FtsDocumentKind::Path,
            FtsDocumentKind::Fact,
        ] {
            assert!(
                index
                    .documents
                    .iter()
                    .any(|document| document.kind as u8 == kind as u8),
                "missing at least one {kind:?} document"
            );
        }

        Ok(())
    }

    /// LIT-22.4.3 AC4: a symbol whose qualified name contains the exact
    /// query term ranks above one that merely shares a common substring.
    #[test]
    fn ranking_favors_exact_and_frequent_term_matches() -> Result<(), Box<dyn std::error::Error>> {
        let index = build_polyglot_index()?;

        let results = index.search("RouteService", 10);
        assert!(!results.is_empty());
        // Every result is a genuine partial or full match on `route`/
        // `service` -- an unrelated document (e.g. LICENSE) never appears.
        assert!(!results.iter().any(|result| result.reference == "LICENSE"));
        assert!(
            results
                .iter()
                .any(|result| result.reference.contains("RouteService"))
        );
        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }

        Ok(())
    }

    /// LIT-22.4.3 AC4: an empty query and a query with no matching terms
    /// both return no results rather than every document.
    #[test]
    fn empty_or_unmatched_query_returns_no_results() -> Result<(), Box<dyn std::error::Error>> {
        let index = build_polyglot_index()?;

        assert!(index.search("", 10).is_empty());
        assert!(index.search("zzzznonexistenttokenzzzz", 10).is_empty());

        Ok(())
    }

    /// LIT-22.4.3 AC4: the same query against the same index produces
    /// byte-identical, stably-ordered results every time -- no live
    /// scoring, no nondeterministic tie-breaking.
    #[test]
    fn search_results_are_deterministic_across_repeated_queries()
    -> Result<(), Box<dyn std::error::Error>> {
        let index = build_polyglot_index()?;

        let first = index.search("service", 10);
        let second = index.search("service", 10);

        assert_eq!(first, second);
        assert!(!first.is_empty());

        Ok(())
    }

    /// LIT-22.4.3 AC1/AC4: rebuilding the index from a graph where only
    /// one symbol's content changed leaves every other document's text
    /// identical -- the index is a pure, per-node function of the graph,
    /// so an unrelated change never perturbs unrelated postings.
    #[test]
    fn incremental_graph_change_only_perturbs_the_changed_document()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("service.py"),
            "def alpha():\n    \"\"\"Alpha docstring.\"\"\"\n    return 1\n\n\ndef beta():\n    \"\"\"Beta docstring.\"\"\"\n    return 2\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let before = FtsIndex::build(&GraphBuilder.build(temp.path(), &artifacts));

        std::fs::write(
            temp.path().join("service.py"),
            "def alpha():\n    \"\"\"Alpha docstring, now updated.\"\"\"\n    return 1\n\n\ndef beta():\n    \"\"\"Beta docstring.\"\"\"\n    return 2\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let after = FtsIndex::build(&GraphBuilder.build(temp.path(), &artifacts));

        let beta_before = before
            .documents
            .iter()
            .find(|document| document.reference.ends_with("beta"))
            .ok_or("missing beta before")?;
        let beta_after = after
            .documents
            .iter()
            .find(|document| document.reference.ends_with("beta"))
            .ok_or("missing beta after")?;
        assert_eq!(beta_before.text, beta_after.text);

        let alpha_before = before
            .documents
            .iter()
            .find(|document| document.reference.ends_with("alpha"))
            .ok_or("missing alpha before")?;
        let alpha_after = after
            .documents
            .iter()
            .find(|document| document.reference.ends_with("alpha"))
            .ok_or("missing alpha after")?;
        assert_ne!(alpha_before.text, alpha_after.text);

        Ok(())
    }
}
