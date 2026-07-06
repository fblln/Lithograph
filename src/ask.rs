//! Local wiki retrieval and MCP-style export over generated Lithograph docs.

use crate::manifest::PageManifest;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// One generated wiki page loaded from disk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiPage {
    /// Manifest page identifier.
    pub id: String,
    /// Repository-relative documentation path.
    pub path: String,
    /// First Markdown heading or manifest id.
    pub title: String,
    /// Markdown content.
    pub body: String,
}

/// One query hit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskHit {
    /// Page id.
    pub page_id: String,
    /// Repository-relative page path.
    pub path: String,
    /// Score from deterministic lexical matching.
    pub score: usize,
    /// Matching excerpt.
    pub excerpt: String,
}

/// Deterministic answer assembled from generated docs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskAnswer {
    /// Original question.
    pub question: String,
    /// Grounded answer text.
    pub answer: String,
    /// Supporting hits.
    pub hits: Vec<AskHit>,
}

/// MCP-style static export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpExport {
    /// Available tool names mirrored from DeepWiki-style access patterns.
    pub tools: Vec<String>,
    /// Wiki structure.
    pub structure: Vec<WikiPageSummary>,
    /// Full wiki page contents.
    pub contents: Vec<WikiPage>,
    /// Optional answer when a question was supplied.
    pub answer: Option<AskAnswer>,
}

/// Summary entry for one wiki page.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WikiPageSummary {
    /// Page id.
    pub id: String,
    /// Repository-relative page path.
    pub path: String,
    /// Page title.
    pub title: String,
}

/// Loads generated docs and performs local deterministic retrieval.
#[derive(Debug, Clone, Copy, Default)]
pub struct WikiSearch;

impl WikiSearch {
    /// Reads every page referenced by `.lithograph/manifest.json`.
    pub fn load_pages(
        &self,
        repo_root: &Path,
    ) -> Result<Vec<WikiPage>, Box<dyn std::error::Error>> {
        let manifest_path = repo_root.join(".lithograph/manifest.json");
        let manifest = PageManifest::from_json(&std::fs::read_to_string(manifest_path)?)?;
        let mut pages = Vec::new();
        for page in manifest.pages {
            let path = PathBuf::from(&page.path);
            let body = std::fs::read_to_string(repo_root.join(&path)).unwrap_or_default();
            pages.push(WikiPage {
                id: page.id,
                path: page.path,
                title: first_heading(&body).unwrap_or_else(|| path.display().to_string()),
                body,
            });
        }
        pages.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(pages)
    }

    /// Answers a question from local generated docs.
    pub fn ask(
        &self,
        repo_root: &Path,
        question: &str,
    ) -> Result<AskAnswer, Box<dyn std::error::Error>> {
        let pages = self.load_pages(repo_root)?;
        Ok(answer_from_pages(question, &pages))
    }

    /// Builds a static MCP-style export payload.
    pub fn export(
        &self,
        repo_root: &Path,
        question: Option<&str>,
    ) -> Result<McpExport, Box<dyn std::error::Error>> {
        let contents = self.load_pages(repo_root)?;
        let structure = contents
            .iter()
            .map(|page| WikiPageSummary {
                id: page.id.clone(),
                path: page.path.clone(),
                title: page.title.clone(),
            })
            .collect();
        let answer = question.map(|question| answer_from_pages(question, &contents));
        Ok(McpExport {
            tools: vec![
                "read_wiki_structure".to_owned(),
                "read_wiki_contents".to_owned(),
                "ask_question".to_owned(),
            ],
            structure,
            contents,
            answer,
        })
    }
}

fn answer_from_pages(question: &str, pages: &[WikiPage]) -> AskAnswer {
    let terms = terms(question);
    let mut hits: Vec<AskHit> = pages
        .iter()
        .filter_map(|page| hit_for_page(page, &terms))
        .collect();
    hits.sort_by(|a, b| b.score.cmp(&a.score).then(a.path.cmp(&b.path)));
    hits.truncate(5);

    let answer = if hits.is_empty() {
        "No generated wiki page matched the question. Run `lithograph init` or ask about terms present in docs/lithograph.".to_owned()
    } else {
        format!(
            "Found {} generated wiki page(s) relevant to `{}`. Highest signal: {}",
            hits.len(),
            question,
            hits[0].excerpt
        )
    };

    AskAnswer {
        question: question.to_owned(),
        answer,
        hits,
    }
}

fn hit_for_page(page: &WikiPage, terms: &BTreeSet<String>) -> Option<AskHit> {
    let mut best_score = 0usize;
    let mut best_excerpt = String::new();
    for line in page.body.lines() {
        let normalized = line.to_lowercase();
        let score = terms
            .iter()
            .filter(|term| normalized.contains(term.as_str()))
            .count();
        if score > best_score {
            best_score = score;
            best_excerpt = line.trim().to_owned();
        }
    }
    if best_score == 0 {
        return None;
    }
    Some(AskHit {
        page_id: page.id.clone(),
        path: page.path.clone(),
        score: best_score,
        excerpt: best_excerpt,
    })
}

fn terms(question: &str) -> BTreeSet<String> {
    question
        .split(|character: char| !character.is_ascii_alphanumeric())
        .map(str::to_lowercase)
        .filter(|term| term.len() > 2)
        .collect()
}

fn first_heading(body: &str) -> Option<String> {
    body.lines()
        .find_map(|line| line.strip_prefix("# ").map(str::trim).map(str::to_owned))
}

/// Renders an answer in a compact table-like format.
pub fn render_ask_table(answer: &AskAnswer) -> String {
    let mut output = format!("{}\n", answer.answer);
    for hit in &answer.hits {
        output.push_str(&format!(
            "- {} (score {}): {}\n",
            hit.path, hit.score, hit.excerpt
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{WikiPage, answer_from_pages};

    #[test]
    fn answer_uses_lexical_hits_from_generated_pages() {
        let pages = vec![WikiPage {
            id: "page:overview".to_owned(),
            path: "docs/lithograph/overview.md".to_owned(),
            title: "Overview".to_owned(),
            body: "# Overview\n\nArchitecture uses modules and source evidence.".to_owned(),
        }];

        let answer = answer_from_pages("architecture modules", &pages);

        assert_eq!(answer.hits.len(), 1);
        assert_eq!(answer.hits[0].score, 2);
    }
}
