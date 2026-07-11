//! Deterministic project-wide indexes for symbol and import resolution.

use crate::domain::Confidence;
use crate::graph::{Graph, GraphNode, GraphNodeId, ModuleLanguage, RelationKind, SymbolKind};
use std::collections::{BTreeMap, BTreeSet};

/// One definition addressable by the resolver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectSymbol {
    /// Stable graph node id.
    pub id: GraphNodeId,
    /// Unqualified declaration name.
    pub simple_name: String,
    /// Language-qualified declaration name.
    pub qualified_name: String,
    /// Defining repository path.
    pub file: String,
    /// Defining module path when known.
    pub module: Option<String>,
    /// Source language label.
    pub language: String,
    /// Symbol category.
    pub kind: SymbolKind,
}

/// Project-wide, deterministic indexes over every definition in a graph.
#[derive(Debug, Clone, Default)]
pub struct ProjectSymbolRegistry {
    /// Definitions by stable graph id.
    pub by_id: BTreeMap<GraphNodeId, ProjectSymbol>,
    /// Candidate ids by simple name.
    pub by_simple_name: BTreeMap<String, BTreeSet<GraphNodeId>>,
    /// Candidate ids by qualified name.
    pub by_qualified_name: BTreeMap<String, BTreeSet<GraphNodeId>>,
    /// Candidate ids by defining file.
    pub by_file: BTreeMap<String, BTreeSet<GraphNodeId>>,
    /// Candidate ids by defining module or package path.
    pub by_module: BTreeMap<String, BTreeSet<GraphNodeId>>,
    /// Candidate ids by language.
    pub by_language: BTreeMap<String, BTreeSet<GraphNodeId>>,
}

impl ProjectSymbolRegistry {
    /// Builds indexes once from a stable graph snapshot.
    pub fn build(graph: &Graph) -> Self {
        let module_language: BTreeMap<_, _> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Module(module) => {
                    Some((module.path.as_str(), language(module.language)))
                }
                _ => None,
            })
            .collect();
        let file_module: BTreeMap<_, _> = graph
            .relations
            .iter()
            .filter(|relation| relation.kind == RelationKind::BelongsToModule)
            .filter_map(|relation| {
                let file = relation.source.as_str().strip_prefix("artifact:")?;
                let module = graph.nodes.iter().find_map(|node| match node {
                    GraphNode::Module(module) if node.id() == &relation.target => {
                        Some(module.path.as_str())
                    }
                    _ => None,
                })?;
                Some((file, module))
            })
            .collect();
        let mut registry = Self::default();
        for node in &graph.nodes {
            let GraphNode::Symbol(symbol) = node else {
                continue;
            };
            let file = symbol.evidence.path.as_str().to_owned();
            let module = file_module
                .get(file.as_str())
                .map(|value| (*value).to_owned());
            let language = module
                .as_deref()
                .and_then(|value| module_language.get(value))
                .cloned()
                .unwrap_or_else(|| "unknown".to_owned());
            let simple_name = symbol
                .qualified_name
                .rsplit("::")
                .next()
                .unwrap_or(&symbol.qualified_name)
                .to_owned();
            let record = ProjectSymbol {
                id: symbol.id.clone(),
                simple_name: simple_name.clone(),
                qualified_name: symbol.qualified_name.clone(),
                file: file.clone(),
                module: module.clone(),
                language: language.clone(),
                kind: symbol.kind,
            };
            registry.by_id.insert(record.id.clone(), record.clone());
            registry
                .by_simple_name
                .entry(simple_name)
                .or_default()
                .insert(record.id.clone());
            registry
                .by_qualified_name
                .entry(record.qualified_name.clone())
                .or_default()
                .insert(record.id.clone());
            registry
                .by_file
                .entry(file)
                .or_default()
                .insert(record.id.clone());
            if let Some(module) = module {
                registry
                    .by_module
                    .entry(module)
                    .or_default()
                    .insert(record.id.clone());
            }
            registry
                .by_language
                .entry(language)
                .or_default()
                .insert(record.id.clone());
        }
        registry
    }
}

/// Explicit result of an import-map lookup, including unresolved ambiguity.
#[allow(missing_docs)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportLookup {
    SameModule {
        target: GraphNodeId,
        confidence: Confidence,
    },
    ExplicitImport {
        target: GraphNodeId,
        confidence: Confidence,
    },
    Suffix {
        target: GraphNodeId,
        confidence: Confidence,
    },
    UniqueName {
        target: GraphNodeId,
        confidence: Confidence,
    },
    Ambiguous {
        candidates: BTreeSet<GraphNodeId>,
    },
    Unresolved,
}

/// Resolves names over a [`ProjectSymbolRegistry`] without guessing.
pub struct ImportMap<'a> {
    registry: &'a ProjectSymbolRegistry,
}
impl<'a> ImportMap<'a> {
    /// Creates an import map over one registry snapshot.
    pub fn new(registry: &'a ProjectSymbolRegistry) -> Self {
        Self { registry }
    }
    /// Looks up a reference with deterministic strategy precedence.
    pub fn lookup(
        &self,
        source_module: Option<&str>,
        explicit: Option<&str>,
        name: &str,
    ) -> ImportLookup {
        if let Some(module) = source_module {
            let ids: BTreeSet<_> = self
                .registry
                .by_module
                .get(module)
                .into_iter()
                .flatten()
                .filter(|id| {
                    self.registry
                        .by_id
                        .get(*id)
                        .is_some_and(|symbol| symbol.simple_name == name)
                })
                .cloned()
                .collect();
            if let Some(target) = (ids.len() == 1).then(|| ids.into_iter().next()).flatten() {
                return ImportLookup::SameModule {
                    target,
                    confidence: Confidence::High,
                };
            }
        }
        if let Some(target) = explicit
            .and_then(|key| self.registry.by_qualified_name.get(key))
            .filter(|ids| ids.len() == 1)
            .and_then(|ids| ids.iter().next().cloned())
        {
            return ImportLookup::ExplicitImport {
                target,
                confidence: Confidence::High,
            };
        }
        let normalized = name.replace('.', "::");
        let suffix: BTreeSet<_> = self
            .registry
            .by_qualified_name
            .iter()
            .filter(|(key, _)| key.ends_with(&normalized))
            .flat_map(|(_, ids)| ids.iter().cloned())
            .collect();
        if let Some(target) = (suffix.len() == 1)
            .then(|| suffix.into_iter().next())
            .flatten()
        {
            return ImportLookup::Suffix {
                target,
                confidence: Confidence::Low,
            };
        }
        let simple_name = name.rsplit('.').next().unwrap_or(name);
        let names = self
            .registry
            .by_simple_name
            .get(simple_name)
            .cloned()
            .unwrap_or_default();
        match names.len() {
            0 => ImportLookup::Unresolved,
            1 => names
                .into_iter()
                .next()
                .map_or(ImportLookup::Unresolved, |target| {
                    ImportLookup::UniqueName {
                        target,
                        confidence: Confidence::Low,
                    }
                }),
            _ => ImportLookup::Ambiguous { candidates: names },
        }
    }
}

fn language(value: ModuleLanguage) -> String {
    match value {
        ModuleLanguage::Python => "python".to_owned(),
        ModuleLanguage::Rust => "rust".to_owned(),
        ModuleLanguage::TypeScript(value) => value.registry_id().to_owned(),
        ModuleLanguage::SyntaxIndexed(value) => value.registry_id().to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::{ImportLookup, ImportMap, ProjectSymbol, ProjectSymbolRegistry};
    use crate::graph::{GraphNodeId, SymbolKind};

    fn registry(records: &[(&str, &str, &str)]) -> ProjectSymbolRegistry {
        let mut result = ProjectSymbolRegistry::default();
        for (id, qualified, module) in records {
            let symbol = ProjectSymbol {
                id: GraphNodeId::new(*id),
                simple_name: qualified
                    .rsplit("::")
                    .next()
                    .unwrap_or(qualified)
                    .to_owned(),
                qualified_name: (*qualified).to_owned(),
                file: format!("{module}.rs"),
                module: Some((*module).to_owned()),
                language: "rust".to_owned(),
                kind: SymbolKind::Function,
            };
            result
                .by_simple_name
                .entry(symbol.simple_name.clone())
                .or_default()
                .insert(symbol.id.clone());
            result
                .by_qualified_name
                .entry(symbol.qualified_name.clone())
                .or_default()
                .insert(symbol.id.clone());
            result
                .by_file
                .entry(symbol.file.clone())
                .or_default()
                .insert(symbol.id.clone());
            result
                .by_module
                .entry((*module).to_owned())
                .or_default()
                .insert(symbol.id.clone());
            result
                .by_language
                .entry("rust".to_owned())
                .or_default()
                .insert(symbol.id.clone());
            result.by_id.insert(symbol.id.clone(), symbol);
        }
        result
    }

    #[test]
    fn import_map_reports_same_module_explicit_suffix_unique_and_unresolved() {
        let registry = registry(&[
            ("symbol:a", "crate::a::run", "crate::a"),
            ("symbol:b", "crate::b::start", "crate::b"),
        ]);
        let map = ImportMap::new(&registry);
        assert!(matches!(
            map.lookup(Some("crate::a"), None, "run"),
            ImportLookup::SameModule { .. }
        ));
        assert!(matches!(
            map.lookup(None, Some("crate::b::start"), "start"),
            ImportLookup::ExplicitImport { .. }
        ));
        assert!(matches!(
            map.lookup(None, None, "start"),
            ImportLookup::Suffix { .. }
        ));
        assert_eq!(map.lookup(None, None, "missing"), ImportLookup::Unresolved);
    }

    #[test]
    fn import_map_never_guesses_an_ambiguous_name() {
        let registry = registry(&[
            ("symbol:a", "crate::a::run", "crate::a"),
            ("symbol:b", "crate::b::run", "crate::b"),
        ]);
        assert!(
            matches!(ImportMap::new(&registry).lookup(None, None, "run"), ImportLookup::Ambiguous { candidates } if candidates.len() == 2)
        );
    }
}
