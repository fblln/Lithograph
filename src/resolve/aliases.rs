//! LIT-45.2: resolves `compilerOptions.paths` aliases to repository paths.
//!
//! A monorepo import names a first-party module by alias (`@nestjs/common`)
//! rather than by relative path, so without the config the specifier matches
//! nothing on disk. This module turns an alias into the paths it could name;
//! the caller still has to find a real file among them, so an alias whose
//! target does not exist resolves to nothing rather than to a guess.

use crate::analysis::TsConfigProfile;
use std::collections::BTreeMap;
use std::path::Path;

/// One `tsconfig.json`'s alias rules, with its patterns already anchored to
/// repository-relative paths.
#[derive(Debug, Clone, PartialEq, Eq)]
struct AliasRules {
    /// Directory containing the config, repository-relative (`""` at root).
    /// Decides which files these rules apply to.
    directory: String,
    /// Alias pattern to repository-relative replacement patterns.
    paths: BTreeMap<String, Vec<String>>,
}

/// Every tsconfig alias rule in the repository, queried by importing file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TsAliasMap {
    rules: Vec<AliasRules>,
}

impl TsAliasMap {
    /// Builds the map from `(config path, profile)` pairs.
    ///
    /// `extends` is resolved here rather than at parse time because it names
    /// another config by path, which only the whole set can answer (AC3). A
    /// child's own `paths` win over an inherited one, matching TypeScript,
    /// where `compilerOptions` are merged key by key.
    pub(crate) fn build(configs: &BTreeMap<String, TsConfigProfile>) -> Self {
        let mut rules = Vec::new();
        for (path, profile) in configs {
            // Only `tsconfig.json` governs a directory. A repository has
            // several other configs (`base.json`, `tsconfig.build.json`,
            // `tsconfig.spec.json`) that exist to be extended or invoked
            // explicitly; treating those as applicable would let a base
            // config's aliases win over the real one in the same directory.
            if file_name(path) != "tsconfig.json" {
                continue;
            }
            let directory = parent_directory(path);
            let mut paths = BTreeMap::new();
            // Walk the extends chain from the far end, so nearer configs
            // overwrite what they inherit.
            for (config_path, inherited) in extends_chain(path, profile, configs).iter().rev() {
                let base = resolve_directory(
                    &parent_directory(config_path),
                    inherited.base_url.as_deref().unwrap_or("."),
                );
                for (alias, targets) in &inherited.paths {
                    paths.insert(
                        alias.clone(),
                        targets
                            .iter()
                            .map(|target| resolve_directory(&base, target))
                            .collect(),
                    );
                }
            }
            if !paths.is_empty() {
                rules.push(AliasRules { directory, paths });
            }
        }
        // Deepest directory first, so `nearest` is a find rather than a scan.
        rules.sort_by(|left, right| {
            right
                .directory
                .len()
                .cmp(&left.directory.len())
                .then_with(|| left.directory.cmp(&right.directory))
        });
        Self { rules }
    }

    /// True when no config in the repository declares any alias.
    pub(crate) fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// Repository-relative paths `specifier`, imported from `source_path`,
    /// could name -- empty when no alias applies.
    ///
    /// The results are base paths without extensions; the caller runs them
    /// through the normal candidate order, which is what keeps a miss a miss
    /// (AC4): an alias pointing at a file that does not exist yields no
    /// artifact and the specifier falls through to existing resolution.
    pub(crate) fn resolve(&self, source_path: &str, specifier: &str) -> Vec<String> {
        let Some(rules) = self.nearest(source_path) else {
            return Vec::new();
        };

        // TypeScript takes the longest matching pattern, so an exact alias
        // beats a wildcard and `@app/deep/*` beats `@app/*`.
        let mut best: Option<(usize, Vec<String>)> = None;
        for (alias, targets) in &rules.paths {
            let Some(expanded) = expand(alias, targets, specifier) else {
                continue;
            };
            let specificity = alias.len();
            if best.as_ref().is_none_or(|(best, _)| specificity > *best) {
                best = Some((specificity, expanded));
            }
        }
        best.map(|(_, targets)| targets).unwrap_or_default()
    }

    /// The rules of the closest config at or above `source_path`, which is the
    /// config TypeScript itself would apply to that file.
    fn nearest(&self, source_path: &str) -> Option<&AliasRules> {
        let directory = parent_directory(source_path);
        self.rules
            .iter()
            .find(|rules| is_within(&directory, &rules.directory))
    }
}

/// Applies one alias pattern to a specifier.
///
/// A pattern holds at most one `*`, which captures the rest of the specifier
/// and is substituted into each target. A pattern without `*` must match the
/// specifier exactly.
fn expand(alias: &str, targets: &[String], specifier: &str) -> Option<Vec<String>> {
    match alias.split_once('*') {
        None => (alias == specifier).then(|| targets.to_vec()),
        Some((prefix, suffix)) => {
            let rest = specifier
                .strip_prefix(prefix)?
                .strip_suffix(suffix)
                .filter(|rest| !rest.is_empty() || suffix.is_empty())?;
            Some(
                targets
                    .iter()
                    .map(|target| match target.split_once('*') {
                        Some((target_prefix, target_suffix)) => {
                            format!("{target_prefix}{rest}{target_suffix}")
                        }
                        // A wildcard alias may map to a fixed file.
                        None => target.clone(),
                    })
                    .collect(),
            )
        }
    }
}

/// The `extends` chain starting at `path`, nearest first.
///
/// Bounded by a visited set: a config that extends itself, directly or through
/// a cycle, would otherwise never terminate.
fn extends_chain<'a>(
    path: &'a str,
    profile: &'a TsConfigProfile,
    configs: &'a BTreeMap<String, TsConfigProfile>,
) -> Vec<(String, &'a TsConfigProfile)> {
    let mut chain = vec![(path.to_owned(), profile)];
    let mut seen = std::collections::BTreeSet::from([path.to_owned()]);
    let mut current = (path.to_owned(), profile);

    while let Some(extends) = current.1.extends.as_deref() {
        // Only relative `extends` names a file in this repository; a bare
        // specifier (`@tsconfig/node20/tsconfig.json`) lives in node_modules,
        // which is not scanned.
        if !(extends.starts_with("./") || extends.starts_with("../")) {
            break;
        }
        let mut resolved = resolve_directory(&parent_directory(&current.0), extends);
        if !resolved.ends_with(".json") {
            resolved.push_str(".json");
        }
        let Some(next) = configs.get(&resolved) else {
            break;
        };
        if !seen.insert(resolved.clone()) {
            break;
        }
        chain.push((resolved.clone(), next));
        current = (resolved, next);
    }

    chain
}

/// Normalizes `base` joined with `relative`, resolving `.` and `..`.
fn resolve_directory(base: &str, relative: &str) -> String {
    use std::path::{Component, PathBuf};

    let mut components: Vec<Component<'_>> = Path::new(base).components().collect();
    for component in Path::new(relative).components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                components.pop();
            }
            other => components.push(other),
        }
    }
    components
        .into_iter()
        .collect::<PathBuf>()
        .to_string_lossy()
        .replace('\\', "/")
}

fn file_name(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, name)| name)
}

fn parent_directory(path: &str) -> String {
    path.rsplit_once('/')
        .map(|(directory, _)| directory.to_owned())
        .unwrap_or_default()
}

/// True when `directory` is `root` or sits inside it. `root` is `""` for a
/// config at the repository root, which contains everything.
fn is_within(directory: &str, root: &str) -> bool {
    root.is_empty() || directory == root || directory.starts_with(&format!("{root}/"))
}

#[cfg(test)]
mod tests {
    use super::TsAliasMap;
    use crate::analysis::TsConfigProfile;
    use std::collections::BTreeMap;

    fn profile(base_url: &str, paths: &[(&str, &[&str])]) -> TsConfigProfile {
        TsConfigProfile {
            extends: None,
            base_url: Some(base_url.to_owned()),
            paths: paths
                .iter()
                .map(|(alias, targets)| {
                    (
                        (*alias).to_owned(),
                        targets.iter().map(|target| (*target).to_owned()).collect(),
                    )
                })
                .collect(),
        }
    }

    /// AC1: `@app/*` -> `src/*`.
    #[test]
    fn wildcard_aliases_expand_to_repository_paths() {
        let configs = BTreeMap::from([(
            "tsconfig.json".to_owned(),
            profile(".", &[("@app/*", &["src/*"])]),
        )]);

        let map = TsAliasMap::build(&configs);

        assert_eq!(map.resolve("src/main.ts", "@app/util"), vec!["src/util"]);
        // No alias matches: not this module's problem to guess.
        assert_eq!(map.resolve("src/main.ts", "react"), Vec::<String>::new());
    }

    /// The NestJS corpus shape: an exact alias and a wildcard for the same
    /// prefix. The exact one is longer, so it must win for the bare specifier.
    #[test]
    fn the_longest_matching_pattern_wins() {
        let configs = BTreeMap::from([(
            "tsconfig.json".to_owned(),
            profile(
                ".",
                &[
                    ("@nestjs/common", &["./packages/common"]),
                    ("@nestjs/common/*", &["./packages/common/*"]),
                ],
            ),
        )]);

        let map = TsAliasMap::build(&configs);

        assert_eq!(
            map.resolve("packages/core/app.ts", "@nestjs/common"),
            vec!["packages/common"],
        );
        assert_eq!(
            map.resolve("packages/core/app.ts", "@nestjs/common/interfaces"),
            vec!["packages/common/interfaces"],
        );
    }

    /// AC3: paths declared by an extended config apply to the child, and the
    /// child's own mapping for the same alias overrides it.
    #[test]
    fn extends_chains_inherit_and_override_paths() {
        let mut child = profile(".", &[("@app/*", &["child/*"])]);
        child.extends = Some("./base.json".to_owned());
        let configs = BTreeMap::from([
            (
                "base.json".to_owned(),
                profile(".", &[("@app/*", &["base/*"]), ("@lib/*", &["lib/*"])]),
            ),
            ("tsconfig.json".to_owned(), child),
        ]);

        let map = TsAliasMap::build(&configs);

        assert_eq!(
            map.resolve("src/main.ts", "@app/x"),
            vec!["child/x"],
            "the child's own mapping must win over the inherited one",
        );
        assert_eq!(
            map.resolve("src/main.ts", "@lib/x"),
            vec!["lib/x"],
            "an alias only the base declares is still inherited",
        );
    }

    /// A config that extends itself must not hang the build.
    #[test]
    fn a_self_extending_config_terminates() {
        let mut looping = profile(".", &[("@app/*", &["src/*"])]);
        looping.extends = Some("./tsconfig.json".to_owned());
        let configs = BTreeMap::from([("tsconfig.json".to_owned(), looping)]);

        let map = TsAliasMap::build(&configs);

        assert_eq!(map.resolve("src/main.ts", "@app/util"), vec!["src/util"]);
    }

    /// TypeScript applies the nearest config to a file. A package's own
    /// aliases must not leak into a sibling package.
    #[test]
    fn the_nearest_config_applies_to_a_file() {
        // Each config's `paths` are relative to its own `baseUrl`, which is
        // itself relative to that config's directory -- so the web package
        // writes `./src/*`, not the repository-relative path.
        let configs = BTreeMap::from([
            (
                "tsconfig.json".to_owned(),
                profile(".", &[("~/*", &["./root/*"])]),
            ),
            (
                "packages/web/tsconfig.json".to_owned(),
                profile(".", &[("~/*", &["./src/*"])]),
            ),
        ]);

        let map = TsAliasMap::build(&configs);

        assert_eq!(
            map.resolve("packages/web/app.ts", "~/util"),
            vec!["packages/web/src/util"],
        );
        assert_eq!(
            map.resolve("packages/api/app.ts", "~/util"),
            vec!["root/util"],
            "a package without its own config falls back to the root config",
        );
    }

    /// `baseUrl` anchors the replacements, and both it and the config's own
    /// location are relative to the repository root.
    #[test]
    fn base_url_anchors_replacements_relative_to_the_config() {
        let configs = BTreeMap::from([(
            "frontend/tsconfig.json".to_owned(),
            profile("./src", &[("@/*", &["./*"])]),
        )]);

        let map = TsAliasMap::build(&configs);

        assert_eq!(
            map.resolve("frontend/src/app.ts", "@/components/Button"),
            vec!["frontend/src/components/Button"],
        );
    }

    #[test]
    fn a_repository_without_alias_configs_is_empty() {
        assert!(TsAliasMap::build(&BTreeMap::new()).is_empty());
    }
}
