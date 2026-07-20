//! Classification of rationale-bearing comments (LIT-46).
//!
//! A parser recovers *what* the code does; only the author can say *why*. That
//! why is usually already written down, as a `# WHY:` or `// HACK:` comment
//! sitting next to the code it explains, and it is the one input the
//! documentation generator cannot derive from an AST.
//!
//! Only prefixed comments qualify. Ordinary commentary is narration of the
//! adjacent line and restates what the code already says; promoting all of it
//! would bury the deliberate notes in noise. The prefix is the author marking
//! a comment as worth keeping.

use serde::{Deserialize, Serialize};

/// Why an author flagged a comment as worth keeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum RationaleKind {
    /// `NOTE:` -- context a reader needs.
    Note,
    /// `WHY:` / `RATIONALE:` -- the reason behind a decision.
    Why,
    /// `IMPORTANT:` -- a constraint that must not be broken.
    Important,
    /// `SAFETY:` -- why an unsafe or delicate operation is sound.
    Safety,
    /// `HACK:` / `XXX:` -- a known compromise.
    Hack,
    /// `TODO:` -- intended future work.
    Todo,
    /// `FIXME:` / `BUG:` -- a known defect.
    Fixme,
}

impl RationaleKind {
    /// Stable lowercase id used in node ids and output.
    pub fn id(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Why => "why",
            Self::Important => "important",
            Self::Safety => "safety",
            Self::Hack => "hack",
            Self::Todo => "todo",
            Self::Fixme => "fixme",
        }
    }
}

/// Recognized prefixes, longest-first within each kind so `RATIONALE:` is not
/// shadowed by a shorter alternative.
const PREFIXES: &[(&str, RationaleKind)] = &[
    ("RATIONALE", RationaleKind::Why),
    ("IMPORTANT", RationaleKind::Important),
    ("SAFETY", RationaleKind::Safety),
    ("FIXME", RationaleKind::Fixme),
    ("NOTE", RationaleKind::Note),
    ("HACK", RationaleKind::Hack),
    ("TODO", RationaleKind::Todo),
    ("WHY", RationaleKind::Why),
    ("BUG", RationaleKind::Fixme),
    ("XXX", RationaleKind::Hack),
];

/// One classified comment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Rationale {
    /// Why the author flagged it.
    pub kind: RationaleKind,
    /// The note itself, with comment syntax and prefix removed.
    pub text: String,
}

/// Classifies one comment's raw source text, or `None` when it carries no
/// rationale prefix.
///
/// Accepts the prefix only at the start of the comment body, so a passing
/// mention ("see the TODO in parse.rs") is not mistaken for a marker.
pub(crate) fn classify(comment: &str) -> Option<Rationale> {
    let body = strip_comment_syntax(comment);
    let trimmed = body.trim_start();
    let (kind, rest) = PREFIXES
        .iter()
        .find_map(|(prefix, kind)| Some((kind, marker_body(trimmed, prefix)?)))?;
    let text = rest.trim_start_matches([':', '-', ' ', '\t']).trim();
    // A bare `// TODO` with nothing after it records no intent worth a node.
    if text.is_empty() {
        return None;
    }
    Some(Rationale {
        kind: *kind,
        text: normalize_whitespace(text),
    })
}

/// The text following `prefix` when it opens `text` as a marker, else `None`.
///
/// The colon is required, because the colon is what separates a marker from
/// an ordinary word. Without it, prose that merely begins with "Why" or
/// "Note" reads as rationale -- this module's own header ("why is usually
/// already written down...") classified as a WHY note until the colon became
/// mandatory. `TODO(owner):` still marks: naming an owner does not stop it
/// being one.
fn marker_body<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    let rest = text.get(..prefix.len())?;
    if !rest.eq_ignore_ascii_case(prefix) {
        return None;
    }
    let rest = &text[prefix.len()..];
    let rest = match rest.strip_prefix('(') {
        Some(after_open) => after_open.split_once(')')?.1,
        None => rest,
    };
    rest.starts_with(':').then_some(rest)
}

/// Strips the comment delimiters every supported language uses, leaving the
/// body. Handles line comments (`//`, `#`, `--`), block comments (`/* */`,
/// `<!-- -->`), doc variants (`///`, `//!`, `/**`), and the leading `*` of
/// continued block-comment lines.
fn strip_comment_syntax(comment: &str) -> String {
    comment
        .lines()
        .map(strip_line_syntax)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Strips one line's comment delimiters. Applied per line because a
/// consecutive run of line comments arrives as one multi-line comment, and
/// every line of it carries its own marker.
fn strip_line_syntax(line: &str) -> &str {
    let mut text = line.trim();
    for open in [
        "<!--", "/**", "/*!", "/*", "///", "//!", "//", "#!", "#", "--",
    ] {
        if let Some(rest) = text.strip_prefix(open) {
            text = rest;
            break;
        }
    }
    for close in ["-->", "*/"] {
        if let Some(rest) = text.strip_suffix(close) {
            text = rest;
            break;
        }
    }
    // The leading `*` of a continued block-comment line is decoration.
    text.trim().trim_start_matches('*').trim()
}

/// Collapses runs of whitespace so a note's text is stable regardless of how
/// the comment was wrapped in source.
fn normalize_whitespace(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Markers that identify machine-written source.
///
/// A generator's banner and its `TODO`s belong to the generator's template,
/// not to this repository's authors, so promoting them would attribute
/// intent no one here expressed. The canonical `DO NOT EDIT` line is a
/// widely honored convention (Go, protobuf, gRPC, OpenAPI, Prisma).
const GENERATED_MARKERS: &[&str] = &[
    "DO NOT EDIT",
    "DO NOT MODIFY",
    "@generated",
    "Code generated by",
    "Generated by the protocol buffer compiler",
    "autogenerated",
    "auto-generated",
    "This file was automatically generated",
];

/// How much of a file's head is scanned for a generated marker. Generators
/// put their banner first; scanning the whole file would let an incidental
/// "do not edit" deep in a handwritten file silence its rationale.
const GENERATED_SCAN_BYTES: usize = 2_048;

/// Whether `text` looks machine-written and should contribute no rationale.
pub(crate) fn is_generated_source(text: &str) -> bool {
    let head = head_bytes(text);
    let head_lower = head.to_lowercase();
    GENERATED_MARKERS
        .iter()
        .any(|marker| head_lower.contains(&marker.to_lowercase()))
        || is_framework_migration(head)
}

/// The first [`GENERATED_SCAN_BYTES`] of `text`, truncated at a character
/// boundary.
///
/// Slicing to a fixed byte offset panics when that offset lands inside a
/// multi-byte character, which is not hypothetical: a Markdown report whose
/// box-drawing characters straddled byte 2048 crashed `init` outright. The
/// cut is approximate by design -- it only bounds how far a banner search
/// reads -- so moving it to the nearest boundary below costs nothing.
fn head_bytes(text: &str) -> &str {
    if text.len() <= GENERATED_SCAN_BYTES {
        return text;
    }
    let mut end = GENERATED_SCAN_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Framework-generated migration files, which carry no banner but whose
/// docstrings are revision annotations rather than authored intent.
///
/// Matched by their required structure rather than by path, since these live
/// wherever a project keeps migrations.
fn is_framework_migration(head: &str) -> bool {
    // Alembic / Flask-Migrate: a revision id plus the upgrade entry point.
    let alembic = head.contains("down_revision")
        && head.contains("def upgrade(")
        && head
            .lines()
            .any(|line| line.trim_start().starts_with("revision"));
    // Django.
    let django = head.contains("class Migration(migrations.Migration)");
    alembic || django
}

#[cfg(test)]
mod tests {
    use super::{RationaleKind, classify, is_generated_source};

    #[test]
    fn classifies_prefixed_comments_across_comment_syntaxes()
    -> Result<(), Box<dyn std::error::Error>> {
        for (comment, kind, text) in [
            (
                "# WHY: the cache is global",
                RationaleKind::Why,
                "the cache is global",
            ),
            (
                "// NOTE: callers must lock",
                RationaleKind::Note,
                "callers must lock",
            ),
            (
                "/// SAFETY: pointer is non-null",
                RationaleKind::Safety,
                "pointer is non-null",
            ),
            (
                "//! IMPORTANT: order matters",
                RationaleKind::Important,
                "order matters",
            ),
            (
                "/* HACK: works around #42 */",
                RationaleKind::Hack,
                "works around #42",
            ),
            ("<!-- TODO: rewrite -->", RationaleKind::Todo, "rewrite"),
            ("-- FIXME: slow query", RationaleKind::Fixme, "slow query"),
            (
                "# RATIONALE: keeps ids stable",
                RationaleKind::Why,
                "keeps ids stable",
            ),
            ("// BUG: off by one", RationaleKind::Fixme, "off by one"),
            ("// XXX: revisit", RationaleKind::Hack, "revisit"),
        ] {
            let rationale = classify(comment).ok_or(format!("{comment} must classify"))?;
            assert_eq!(rationale.kind, kind, "{comment}");
            assert_eq!(rationale.text, text, "{comment}");
        }

        Ok(())
    }

    #[test]
    fn accepts_owner_annotated_markers_and_normalizes_wrapping()
    -> Result<(), Box<dyn std::error::Error>> {
        let rationale = classify("// TODO(alex): split\n// this up").ok_or("must classify")?;
        assert_eq!(rationale.kind, RationaleKind::Todo);
        assert_eq!(rationale.text, "split this up");

        Ok(())
    }

    /// The prefix marks intent only when the author used it as a marker.
    #[test]
    fn ignores_unprefixed_commentary_and_passing_mentions() {
        for comment in [
            "// increment the counter",
            "# see the TODO in parse.rs",
            "// TODOS remain",
            "// NOTES from the meeting",
            "# TODO",
            "// TODO fix this without a colon",
            "//",
        ] {
            assert!(classify(comment).is_none(), "{comment} must not classify");
        }
    }

    #[test]
    fn detects_generated_sources_by_banner_and_migration_shape() {
        for text in [
            "// Code generated by protoc-gen-go. DO NOT EDIT.\n",
            "# @generated by fixture-generator\n",
            "/* This file was automatically generated */\n",
            "\"\"\"empty message\"\"\"\nrevision = \"abc123\"\ndown_revision = None\n\n\ndef upgrade():\n    pass\n",
            "class Migration(migrations.Migration):\n    operations = []\n",
        ] {
            assert!(is_generated_source(text), "must detect: {text}");
        }
    }

    #[test]
    fn treats_handwritten_sources_as_authored() {
        for text in [
            "# WHY: this module is hand written\n",
            "def upgrade():\n    pass\n",
            "// a note about editing files\n",
        ] {
            assert!(!is_generated_source(text), "must not detect: {text}");
        }
    }

    /// A marker buried far below the head belongs to prose, not to a
    /// generator's banner.
    #[test]
    fn only_scans_the_head_for_generated_markers() {
        let text = format!("{}\n// DO NOT EDIT\n", "x".repeat(4_000));
        assert!(!is_generated_source(&text));
    }

    /// The head cut is a byte offset, and slicing to one that lands inside a
    /// multi-byte character panics. A Markdown report whose box-drawing
    /// characters straddled byte 2048 crashed `init` outright.
    #[test]
    fn head_cut_never_splits_a_multibyte_character() {
        // Pad so the 2048-byte boundary falls inside a 3-byte character.
        for padding in 2_040..2_050 {
            let text = format!("{}{}", "x".repeat(padding), "│".repeat(20));
            assert!(!is_generated_source(&text), "padding {padding}");
        }
        // The same text is still searched for a banner within the head.
        let text = format!("// DO NOT EDIT\n{}{}", "x".repeat(2_040), "│".repeat(20));
        assert!(is_generated_source(&text));
    }
}
