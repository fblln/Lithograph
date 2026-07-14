use super::clones::CLONE_ALGORITHM_VERSION;
use super::*;

/// BLAKE3 hash of the exact body lines `start_line..=end_line` (one-based,
/// inclusive) used as a candidate's content identity for the clone snapshot
/// cache (LIT-35.3). Independent of tokenization so a tokenizer change is
/// versioned separately in the set identity.
pub(super) fn body_content_hash(text: &str, start_line: u32, end_line: u32) -> String {
    let start = start_line.saturating_sub(1) as usize;
    let end = end_line as usize;
    let body: Vec<&str> = text
        .lines()
        .enumerate()
        .filter(|(index, _)| *index >= start && *index < end)
        .map(|(_, line)| line)
        .collect();
    blake3::hash(body.join("\n").as_bytes())
        .to_hex()
        .to_string()
}

/// Canonical identity of a whole near-clone candidate set (LIT-35.3 AC1/AC3).
/// Sorts the per-candidate descriptors so enumeration order cannot change the
/// key, and prefixes the algorithm version, tokenizer version, and every
/// threshold so a change to any of them invalidates persisted snapshots.
/// `parts` is sorted in place.
pub(super) fn clone_set_identity(
    parts: &mut [String],
    min_body_lines: u32,
    similar_threshold: f64,
    trace_threshold: f64,
    high_confidence_threshold: f64,
) -> String {
    parts.sort_unstable();
    let mut hasher = blake3::Hasher::new();
    // Tokenizer semantics are baked into the graph pipeline version; bump that
    // (or CLONE_ALGORITHM_VERSION) when `word_tokens` changes.
    hasher.update(
        format!(
            "clone-v{CLONE_ALGORITHM_VERSION}|pipeline-v{GRAPH_BUILD_PIPELINE_VERSION}|min{min_body_lines}|sim{}|trace{}|high{}\n",
            (similar_threshold * 1_000_000.0).round() as u64,
            (trace_threshold * 1_000_000.0).round() as u64,
            (high_confidence_threshold * 1_000_000.0).round() as u64,
        )
        .as_bytes(),
    );
    for part in parts.iter() {
        hasher.update(part.as_bytes());
        hasher.update(b"\n");
    }
    hasher.finalize().to_hex().to_string()
}

/// Lowercase word-shaped tokens (letters/digits/underscore runs longer
/// than one character) from `text`'s `start_line..=end_line` (one-based,
/// inclusive) -- deterministic lexical content for near-clone comparison
/// (LIT-22.3.6 AC2). Single-character tokens (`x`, `_`) are dropped:
/// they're common enough to dominate the Jaccard score without indicating
/// real similarity.
pub(super) fn word_tokens(text: &str, start_line: u32, end_line: u32) -> BTreeSet<String> {
    let start = start_line.saturating_sub(1) as usize;
    let end = end_line as usize;
    text.lines()
        .enumerate()
        .filter(|(index, _)| *index >= start && *index < end)
        .flat_map(|(_, line)| line.split(|ch: char| !ch.is_alphanumeric() && ch != '_'))
        .filter(|token| token.len() > 1)
        .map(str::to_lowercase)
        .collect()
}

/// `|intersection| / |union|`, `0.0` when both sets are empty. `a` and `b`
/// must be sorted and unique; the intersection is a linear two-pointer scan
/// (LIT-35.1 AC1), giving the same score as the previous set-based version.
pub(super) fn jaccard_similarity(a: &[u32], b: &[u32]) -> f64 {
    let (mut i, mut j, mut intersection) = (0, 0, 0usize);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                intersection += 1;
                i += 1;
                j += 1;
            }
        }
    }
    let union = a.len() + b.len() - intersection;
    if union == 0 {
        return 0.0;
    }
    intersection as f64 / union as f64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-35.1 AC1: the two-pointer Jaccard over sorted interned IDs matches
    /// the set definition |A∩B| / |A∪B| across empty, disjoint, identical, and
    /// partial-overlap inputs.
    #[test]
    fn two_pointer_jaccard_matches_set_definition() {
        assert_eq!(jaccard_similarity(&[], &[]), 0.0);
        assert_eq!(jaccard_similarity(&[1, 2, 3], &[]), 0.0);
        assert_eq!(jaccard_similarity(&[1, 2, 3], &[4, 5, 6]), 0.0);
        assert_eq!(jaccard_similarity(&[1, 2, 3], &[1, 2, 3]), 1.0);
        // {1,2,3} ∩ {2,3,4} = {2,3} (2), ∪ = {1,2,3,4} (4) -> 0.5
        assert_eq!(jaccard_similarity(&[1, 2, 3], &[2, 3, 4]), 0.5);
        // Asymmetry in lengths: {1,2} ∩ {1,2,3,4} = 2, ∪ = 4 -> 0.5
        assert_eq!(jaccard_similarity(&[1, 2], &[1, 2, 3, 4]), 0.5);
    }

    /// LIT-22.3.6 AC2/AC4: two near-identical functions (same shape,
    /// trivially renamed) produce a `SimilarTo` relation via deterministic
    /// lexical similarity; a clearly different function does not pair
    /// with either.
    #[test]
    fn near_identical_functions_produce_a_similar_to_relation()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("clones.py"),
            "\
def calculate_total(items):
    total = 0
    for item in items:
        total += item.price
    return total


def calculate_total_v2(items):
    total = 0
    for item in items:
        total += item.price * 2
    return total


def render_report(data):
    output = []
    for section in data.sections:
        output.append(section.title)
    return \"\\n\".join(output)
",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let symbol_id = |name: &str| -> Option<&crate::graph::model::GraphNodeId> {
            graph.nodes.iter().find_map(|node| match node {
                GraphNode::Symbol(symbol) if symbol.qualified_name.ends_with(name) => {
                    Some(&symbol.id)
                }
                _ => None,
            })
        };
        let total_id = symbol_id("calculate_total").ok_or("missing calculate_total symbol")?;
        let total_v2_id =
            symbol_id("calculate_total_v2").ok_or("missing calculate_total_v2 symbol")?;
        let report_id = symbol_id("render_report").ok_or("missing render_report symbol")?;

        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::SimilarTo
                    && ((&relation.source == total_id && &relation.target == total_v2_id)
                        || (&relation.source == total_v2_id && &relation.target == total_id)))
        );
        assert!(!graph.relations.iter().any(|relation| {
            relation.kind == RelationKind::SimilarTo
                && (&relation.source == report_id || &relation.target == report_id)
        }));

        Ok(())
    }

    /// LIT-22.3.6 AC2/AC4: near-clone scoring is a pure deterministic
    /// function of source text -- building the same repository twice
    /// yields byte-identical `SimilarTo` relations, never live/varying
    /// embedding-based scores.
    #[test]
    fn near_clone_detection_is_deterministic_across_repeated_builds()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("clones.py"),
            "def calculate_total(items):\n    total = 0\n    for item in items:\n        total += item.price\n    return total\n\n\ndef calculate_total_v2(items):\n    total = 0\n    for item in items:\n        total += item.price * 2\n    return total\n",
        )?;
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;

        let first = GraphBuilder.build(temp.path(), &artifacts);
        let second = GraphBuilder.build(temp.path(), &artifacts);

        let similar_relations = |graph: &crate::graph::model::Graph| {
            graph
                .relations
                .iter()
                .filter(|relation| relation.kind == RelationKind::SimilarTo)
                .cloned()
                .collect::<Vec<_>>()
        };
        assert_eq!(similar_relations(&first), similar_relations(&second));
        assert!(!similar_relations(&first).is_empty());

        Ok(())
    }
}
