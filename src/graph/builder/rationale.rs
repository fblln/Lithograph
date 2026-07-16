//! Promotes author-recorded intent to graph nodes (LIT-46).
//!
//! Comments are parsed for every tree-sitter language already; until now only
//! full-text search consumed them, so the one thing a parser cannot recover --
//! *why* -- was thrown away at graph construction.

use super::evidence::syntax_fact_evidence;
use super::*;
use crate::analysis::{TreeSitterComment, classify_rationale};
use crate::domain::{ArtifactCategory, SourceSpan};
use crate::graph::model::RationaleNode;

/// One symbol's span, used to decide which symbol a comment sits inside.
pub(super) struct SymbolSpan {
    /// The symbol's node id.
    pub id: GraphNodeId,
    /// Lines the symbol covers, inclusive.
    pub span: SourceSpan,
}

impl BuilderState {
    /// Records every rationale-bearing comment in `comments` as a node
    /// explaining the symbol it sits inside, or the artifact when it sits
    /// outside every symbol.
    ///
    /// `symbol_spans` may be empty: a file-level note is still worth keeping,
    /// and attaching it to the artifact is honest about what it explains.
    pub(super) fn process_rationale(
        &mut self,
        artifact: &Artifact,
        artifact_node: &GraphNodeId,
        comments: &[TreeSitterComment],
        symbol_spans: &[SymbolSpan],
    ) {
        // Generated files carry their generator's notes, not this
        // repository's. Attributing a template's `TODO` to these authors
        // would be inventing intent nobody here expressed.
        if artifact.category == ArtifactCategory::GeneratedSource {
            return;
        }

        for comment in comments {
            let Some(rationale) = classify_rationale(&comment.text) else {
                continue;
            };
            let evidence = syntax_fact_evidence(artifact, comment.span.clone());
            let id = GraphNodeId::new(format!(
                "rationale:{}#L{}",
                artifact.path, comment.span.start_line
            ));
            let node = self.insert(GraphNode::Rationale(RationaleNode {
                id,
                kind: rationale.kind,
                text: rationale.text,
                evidence: evidence.clone(),
            }));
            // The file literally contains the comment, exactly as it
            // contains its symbols. Without this the note is unreachable from
            // its artifact, and documentation planning -- which walks outward
            // from artifacts -- would never see it.
            self.relate(
                artifact_node.clone(),
                node.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![evidence.clone()],
            );
            let target = innermost_symbol(symbol_spans, comment.span.start_line)
                .unwrap_or_else(|| artifact_node.clone());
            self.relate_with_provenance(
                node,
                target,
                RelationKind::RationaleFor,
                Confidence::High,
                vec![evidence],
                Some(format_provenance(
                    "rationale",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }
}

/// The smallest symbol whose span contains `line`.
///
/// Smallest wins because symbols nest: a note inside a method sits inside
/// that method's class too, and the method is what it explains. Confidence is
/// `High` regardless -- the comment's position is a fact, not an inference.
fn innermost_symbol(symbol_spans: &[SymbolSpan], line: u32) -> Option<GraphNodeId> {
    symbol_spans
        .iter()
        .filter(|symbol| symbol.span.start_line <= line && line <= symbol.span.end_line)
        .min_by_key(|symbol| symbol.span.end_line - symbol.span.start_line)
        .map(|symbol| symbol.id.clone())
}

#[cfg(test)]
mod tests {
    use super::{SymbolSpan, innermost_symbol};
    use crate::domain::SourceSpan;
    use crate::graph::GraphNodeId;

    fn symbol(id: &str, start: u32, end: u32) -> SymbolSpan {
        SymbolSpan {
            id: GraphNodeId::new(id),
            span: SourceSpan::new(start, end).unwrap_or_else(|_| unreachable!()),
        }
    }

    #[test]
    fn picks_the_innermost_containing_symbol() {
        let spans = vec![symbol("class", 10, 40), symbol("method", 20, 30)];

        assert_eq!(
            innermost_symbol(&spans, 25),
            Some(GraphNodeId::new("method")),
            "a note inside a method explains the method, not its class",
        );
        assert_eq!(
            innermost_symbol(&spans, 15),
            Some(GraphNodeId::new("class")),
            "a note between methods explains the class",
        );
        assert_eq!(
            innermost_symbol(&spans, 5),
            None,
            "a note outside every symbol belongs to the artifact",
        );
    }
}

#[cfg(test)]
mod builder_tests {
    use crate::graph::{GraphBuilder, GraphNode, RelationKind};
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-46: notes attach to the symbol they sit inside, carry their span,
    /// and a generated file contributes none.
    #[test]
    fn rationale_attaches_to_the_enclosing_symbol_and_skips_generated_files()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("app.py"),
            concat!(
                "# NOTE: loaded before config.\n",
                "import os\n\n\n",
                "class Cache:\n",
                "    # WHY: global is fine; contention is bounded.\n",
                "    def get(self, key):\n",
                "        # TODO(alex): add an LRU bound\n",
                "        return os.environ.get(key)\n",
            ),
        )?;
        std::fs::write(
            temp.path().join("client.py"),
            "# Code generated by thing-gen. DO NOT EDIT.\n# TODO: the generator's note, not ours\n\n\ndef call():\n    return 1\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let rationale: Vec<_> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Rationale(node) => Some(node),
                _ => None,
            })
            .collect();
        assert_eq!(
            rationale.len(),
            3,
            "expected exactly the three authored notes, got {:?}",
            rationale.iter().map(|node| &node.text).collect::<Vec<_>>()
        );
        assert!(
            rationale
                .iter()
                .all(|node| node.evidence.path.as_str() == "app.py"
                    && node.evidence.span.is_some()),
            "the generated file must contribute nothing, and every note needs a span",
        );

        // Returns the id the named note explains, or a message naming what
        // was missing, so a failure reads as a fact rather than a panic.
        let explains = |text: &str| -> String {
            let Some(node) = rationale.iter().find(|node| node.text.contains(text)) else {
                return format!("(no note matching {text})");
            };
            graph
                .relations
                .iter()
                .find(|relation| {
                    relation.kind == RelationKind::RationaleFor && relation.source == node.id
                })
                .map(|relation| relation.target.as_str().to_owned())
                .unwrap_or_else(|| format!("(note {text} explains nothing)"))
        };

        assert_eq!(explains("loaded before config"), "artifact:app.py");
        assert_eq!(explains("global is fine"), "symbol:app.py#app::Cache");
        assert_eq!(
            explains("add an LRU bound"),
            "symbol:app.py#app::Cache::get",
            "a note inside a method explains the method, not its class",
        );

        // Builder output must satisfy the validator that gates `init`.
        let invalid: Vec<_> = crate::graph::GraphValidator
            .validate(&graph, &artifacts)
            .into_iter()
            .filter(|issue| issue.kind == crate::graph::GraphIssueKind::InvalidRelationTarget)
            .collect();
        assert!(invalid.is_empty(), "invalid targets: {invalid:?}");

        Ok(())
    }
}
