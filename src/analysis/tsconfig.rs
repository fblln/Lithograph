//! LIT-45.2: `tsconfig.json` parsing for module path aliases.
//!
//! Monorepo TypeScript imports a first-party module by an alias declared in
//! `compilerOptions.paths` (`@nestjs/common`, `~/lib/x`) rather than by a
//! relative path. Without the config those specifiers name nothing on disk, so
//! every import through one is lost.
//!
//! These files are JSONC, not JSON: the TypeScript compiler accepts comments
//! and trailing commas, and real configs use both -- the pinned NestJS corpus
//! has a trailing comma in `compilerOptions.paths` that `serde_json` rejects
//! outright. So the text is normalized to JSON before parsing.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The parts of a `tsconfig.json` that affect module resolution.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct TsConfigProfile {
    /// `extends` specifier, when the config inherits from another.
    pub extends: Option<String>,
    /// `compilerOptions.baseUrl`, relative to this config's directory.
    pub base_url: Option<String>,
    /// `compilerOptions.paths`: alias pattern to replacement patterns, each
    /// relative to `base_url` (or the config directory when unset).
    pub paths: BTreeMap<String, Vec<String>>,
}

/// Parses one `tsconfig.json`'s resolution-relevant fields.
///
/// Returns `None` only when the text is not recoverable as JSON at all. A
/// config without `paths` parses to an empty profile rather than an error:
/// most configs have none, and that is not a failure.
pub(crate) fn parse_tsconfig(text: &str) -> Option<TsConfigProfile> {
    #[derive(Deserialize)]
    struct Raw {
        extends: Option<String>,
        #[serde(rename = "compilerOptions")]
        compiler_options: Option<RawOptions>,
    }
    #[derive(Deserialize)]
    struct RawOptions {
        #[serde(rename = "baseUrl")]
        base_url: Option<String>,
        paths: Option<BTreeMap<String, Vec<String>>>,
    }

    let raw: Raw = serde_json::from_str(&strip_jsonc(text)).ok()?;
    let options = raw.compiler_options;
    Some(TsConfigProfile {
        extends: raw.extends,
        base_url: options.as_ref().and_then(|value| value.base_url.clone()),
        paths: options.and_then(|value| value.paths).unwrap_or_default(),
    })
}

/// Rewrites JSONC as JSON: strips `//` and `/* */` comments and trailing
/// commas.
///
/// String-aware, because a `//` inside a path value (`"http://x"`) or a `*/`
/// inside a glob is data, not a comment. Escapes are tracked for the same
/// reason: a quote inside a string must not end it.
///
/// Comments are replaced by nothing and commas by a space, so byte offsets do
/// not need to survive -- only the parse does.
fn strip_jsonc(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(character) = chars.next() {
        if in_string {
            out.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => {
                in_string = true;
                out.push(character);
            }
            '/' if chars.peek() == Some(&'/') => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for next in chars.by_ref() {
                    if previous == '*' && next == '/' {
                        break;
                    }
                    previous = next;
                }
            }
            _ => out.push(character),
        }
    }

    strip_trailing_commas(&out)
}

/// Removes a comma that is followed only by whitespace and a closing bracket.
fn strip_trailing_commas(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_string = false;
    let mut escaped = false;
    let characters: Vec<char> = text.chars().collect();

    for (index, character) in characters.iter().enumerate() {
        if in_string {
            out.push(*character);
            if escaped {
                escaped = false;
            } else if *character == '\\' {
                escaped = true;
            } else if *character == '"' {
                in_string = false;
            }
            continue;
        }
        if *character == '"' {
            in_string = true;
            out.push(*character);
            continue;
        }
        if *character == ',' {
            let next = characters[index + 1..]
                .iter()
                .find(|candidate| !candidate.is_whitespace());
            if matches!(next, Some('}') | Some(']')) {
                // Drop the comma; the whitespace after it is preserved by the
                // loop, so the document keeps its shape.
                continue;
            }
        }
        out.push(*character);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::{parse_tsconfig, strip_jsonc};

    /// AC2, using the exact shape of the pinned NestJS corpus config: comments
    /// and a trailing comma in `paths`. `serde_json` rejects both, so without
    /// stripping there is no alias map at all.
    #[test]
    fn jsonc_comments_and_trailing_commas_parse() -> Result<(), Box<dyn std::error::Error>> {
        let profile = parse_tsconfig(
            r#"{
  // Line comment.
  /* Block
     comment. */
  "extends": "./base.json",
  "compilerOptions": {
    "baseUrl": ".",
    "paths": {
      "@nestjs/common": ["./packages/common"],
      "@nestjs/common/*": ["./packages/common/*"],
    },
  },
}"#,
        )
        .ok_or("expected a profile")?;

        assert_eq!(profile.extends.as_deref(), Some("./base.json"));
        assert_eq!(profile.base_url.as_deref(), Some("."));
        assert_eq!(
            profile.paths.get("@nestjs/common/*").map(Vec::as_slice),
            Some(["./packages/common/*".to_owned()].as_slice()),
        );

        Ok(())
    }

    /// A `//` inside a string is data. Treating it as a comment would silently
    /// truncate the value and, for a path, produce a wrong alias target.
    #[test]
    fn comment_markers_inside_strings_are_left_alone() {
        assert_eq!(
            strip_jsonc(r#"{"a": "http://x", "b": "/* not a comment */"}"#),
            r#"{"a": "http://x", "b": "/* not a comment */"}"#,
        );
        // An escaped quote must not end the string early.
        assert_eq!(
            strip_jsonc(r#"{"a": "say \"hi\" // here"}"#),
            r#"{"a": "say \"hi\" // here"}"#,
        );
    }

    /// A comma inside a string, and a legitimate separating comma, must both
    /// survive; only a comma before a closing bracket is trailing.
    #[test]
    fn only_genuinely_trailing_commas_are_removed() -> Result<(), Box<dyn std::error::Error>> {
        let profile = parse_tsconfig(r#"{"compilerOptions": {"paths": {"a": ["x", "y,z"],}}}"#)
            .ok_or("expected a profile")?;

        assert_eq!(
            profile.paths.get("a").map(Vec::as_slice),
            Some(["x".to_owned(), "y,z".to_owned()].as_slice()),
        );

        Ok(())
    }

    /// A config with no paths is normal, not an error.
    #[test]
    fn a_config_without_paths_is_an_empty_profile() -> Result<(), Box<dyn std::error::Error>> {
        let profile =
            parse_tsconfig(r#"{"compilerOptions": {"target": "ES2021"}}"#).ok_or("expected")?;

        assert!(profile.paths.is_empty());
        assert_eq!(profile.base_url, None);

        Ok(())
    }

    #[test]
    fn unparseable_text_yields_no_profile() {
        assert_eq!(parse_tsconfig("not json at all {{{"), None);
    }
}
