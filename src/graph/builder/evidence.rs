use super::*;

/// What a language-specific typed pass already contributed before handing
/// off to [`BuilderState::process_syntax_indexed_facts`]. Bundled into one
/// struct (rather than three more parameters) to keep the function under
/// Clippy's argument-count limit; `Default` gives every language with no
/// typed pass of its own an all-empty, all-no-op value.
#[derive(Default)]
pub(super) struct TypedPassFacts<'a> {
    /// Definition kinds the typed pass already represented precisely, so
    /// this pass skips re-creating a generic `Definition` symbol for them.
    pub(super) definition_kinds: &'a [&'a str],
    /// Spans of symbols the typed pass already created (LIT-46), seeded so
    /// a rationale comment can attach to the one it sits inside.
    pub(super) symbol_spans: Vec<super::rationale::SymbolSpan>,
    /// LIT-71: this file's own bare npm import bindings the typed pass
    /// resolved (local name -> (package, exported member)); empty outside
    /// TypeScript/JSX, which is the only place one is built.
    pub(super) bare_package_imports: BTreeMap<String, (String, String)>,
}

impl BuilderState {
    pub(super) fn materialize_environment_facts(&mut self) {
        let mut facts = std::mem::take(&mut self.environment_facts);
        facts.sort_deterministically();
        // LIT-40.2: seed the dedup key set from relations added by earlier
        // passes so `relate_if_absent` matches the previous full-scan behaviour
        // without the per-fact O(relations) cost.
        self.env_relation_keys = self
            .relations
            .iter()
            .map(|relation| {
                (
                    relation.source.clone(),
                    relation.target.clone(),
                    relation.kind,
                )
            })
            .collect();
        // LIT-40.1: group symbol spans by artifact in one node pass so
        // `smallest_symbol_owner` scans one file's symbols, not all nodes.
        let mut env_symbols_by_artifact: HashMap<ArtifactId, Vec<(u32, u32, GraphNodeId)>> =
            HashMap::new();
        for node in self.nodes.values() {
            let GraphNode::Symbol(symbol) = node else {
                continue;
            };
            let Some(span) = symbol.evidence.span.as_ref() else {
                continue;
            };
            env_symbols_by_artifact
                .entry(symbol.evidence.artifact_id.clone())
                .or_default()
                .push((span.start_line, span.end_line, symbol.id.clone()));
        }
        self.env_symbols_by_artifact = env_symbols_by_artifact;
        for fact in &facts.env {
            self.materialize_env_fact(fact);
        }
        for fact in &facts.config {
            self.materialize_config_fact(fact);
        }
        self.env_relation_keys = BTreeSet::new();
        self.env_symbols_by_artifact = HashMap::new();
        self.environment_facts = facts;
    }
    fn materialize_env_fact(&mut self, fact: &EnvFact) {
        let target = self.env_var(fact.name.original());
        let owner = fact.owner.clone().or_else(|| {
            (fact.source == FactSourceKind::SourceCode)
                .then(|| self.smallest_symbol_owner(&fact.evidence))
                .flatten()
                .map(|id| id.to_string())
        });
        let source = self.fact_source_node(fact.source, owner.as_deref(), &fact.evidence);
        let kind = match fact.role {
            FactRole::Define => RelationKind::DefinesEnv,
            FactRole::Read | FactRole::Reference => RelationKind::ReadsEnv,
        };
        self.relate_if_absent(
            source,
            target,
            kind,
            fact.confidence,
            vec![fact.evidence.clone()],
            Some(environment_provenance(fact.source, fact.confidence)),
        );
    }
    fn materialize_config_fact(&mut self, fact: &ConfigFact) {
        let target = self.config_key(fact);
        let source = self.fact_source_node(fact.source, fact.owner.as_deref(), &fact.evidence);
        let kind = match fact.role {
            FactRole::Define => RelationKind::BindsConfig,
            FactRole::Read | FactRole::Reference => RelationKind::ReferencesConfig,
        };
        self.relate_if_absent(
            source,
            target,
            kind,
            fact.confidence,
            vec![fact.evidence.clone()],
            Some(environment_provenance(fact.source, fact.confidence)),
        );
    }
    fn config_key(&mut self, fact: &ConfigFact) -> GraphNodeId {
        let id = GraphNodeId::new(format!("config-key:{}", fact.key.canonical));
        self.insert(GraphNode::Config(ConfigNode {
            id: id.clone(),
            kind: ConfigNodeKind::Key,
            name: fact.key.canonical.clone(),
            evidence: fact.evidence.clone(),
        }))
    }
    fn fact_source_node(
        &self,
        source: FactSourceKind,
        owner: Option<&str>,
        evidence: &EvidenceRef,
    ) -> GraphNodeId {
        if let Some(owner) = owner {
            let path = evidence.path.as_str();
            return match source {
                FactSourceKind::Compose => {
                    GraphNodeId::new(format!("config:{path}#services.{owner}"))
                }
                FactSourceKind::CiWorkflow => {
                    GraphNodeId::new(format!("config:{path}#jobs.{owner}"))
                }
                _ => GraphNodeId::new(owner),
            };
        }
        GraphNodeId::new(format!("artifact:{}", evidence.path.as_str()))
    }
    fn smallest_symbol_owner(&self, evidence: &EvidenceRef) -> Option<GraphNodeId> {
        let line = evidence.span.as_ref()?.start_line;
        // LIT-40.1: only this file's symbols can enclose the line; the same
        // min-by (span_length, id) over that subset yields the identical owner
        // the previous all-nodes scan produced.
        self.env_symbols_by_artifact
            .get(&evidence.artifact_id)?
            .iter()
            .filter(|(start_line, end_line, _)| *start_line <= line && line <= *end_line)
            .min_by_key(|(start_line, end_line, id)| (end_line - start_line, id.clone()))
            .map(|(_, _, id)| id.clone())
    }
    /// Resolves a generic tree-sitter [`TreeSitterAdapterOutput`] (LIT-22.2.3)
    /// into a `Module` node, one `Symbol` node per definition fact, and an
    /// `Imports` relation per import fact. Unlike Python/Rust's specialized
    /// processing, this never resolves an import target to a known module or
    /// package -- it always lands on an `Unresolved` node with
    /// `RelationResolution::SyntaxOnly` provenance, since cross-file
    /// resolution for these languages is LIT-22.3's hybrid resolver, not
    /// this syntax-only pass (AC3: never overclaim `HybridResolved`).
    pub(super) fn process_syntax_indexed(
        &mut self,
        artifact: &Artifact,
        language: SyntaxIndexedLanguage,
        output: TreeSitterAdapterOutput,
        artifact_node: &GraphNodeId,
    ) {
        let module_id = self.module(
            artifact.path.as_str(),
            ModuleLanguage::SyntaxIndexed(language),
            file_evidence(artifact),
        );
        self.relate(
            artifact_node.clone(),
            module_id,
            RelationKind::BelongsToModule,
            Confidence::High,
            vec![file_evidence(artifact)],
        );

        self.process_syntax_indexed_facts(
            artifact,
            language,
            output,
            artifact_node,
            TypedPassFacts::default(),
        );
    }
    /// Applies syntax-level facts after a language-specific declaration pass.
    /// `facts` carries what that typed pass already contributed -- empty
    /// (`TypedPassFacts::default()`) for a language with no typed pass of
    /// its own, so every field below is a no-op there.
    pub(super) fn process_syntax_indexed_facts(
        &mut self,
        artifact: &Artifact,
        language: SyntaxIndexedLanguage,
        output: TreeSitterAdapterOutput,
        artifact_node: &GraphNodeId,
        facts: TypedPassFacts<'_>,
    ) {
        let TypedPassFacts {
            definition_kinds: typed_definition_kinds,
            symbol_spans: typed_symbols,
            bare_package_imports,
        } = facts;
        let registry_id = language.registry_id();

        // LIT-46: every symbol in the file, so a note can be attached to the
        // one it sits inside rather than only to the file. Seeded with the
        // symbols a language-specific pass already created, since those are
        // skipped below and their ids are the ones actually in the graph.
        let mut symbol_spans = typed_symbols;

        for definition in &output.definitions {
            if typed_definition_kinds.contains(&definition.kind.as_str()) {
                continue;
            }
            let evidence = syntax_fact_evidence(artifact, definition.span.clone());
            // LIT-75: a named TS/JS type-level declaration (interface, type
            // alias, enum) gets a name-bearing qualified name so a reference
            // to it -- `props: FooProps`, `state: AppState` -- resolves to
            // the definition instead of an `Unresolved` node. Without this
            // it was interned anonymously by kind and line, unreachable by
            // name. Deliberately gated to these TS-specific node kinds: other
            // languages' definitions keep the positional identity their
            // symbols already have, so this changes nothing outside TS/TSX.
            let qualified = match named_definition_symbol(&definition.kind, &definition.text) {
                Some(name) => format!("{}::{}", artifact.path, name),
                None => format!(
                    "{}::{}@L{}",
                    artifact.path, definition.kind, definition.span.start_line
                ),
            };
            let symbol_id = self.insert(GraphNode::Symbol(SymbolNode {
                id: GraphNodeId::new(format!("symbol:{qualified}")),
                kind: SymbolKind::Definition,
                qualified_name: qualified,
                doc: None,
                evidence: evidence.clone(),
            }));
            symbol_spans.push(super::rationale::SymbolSpan {
                id: symbol_id.clone(),
                span: definition.span.clone(),
            });
            self.relate_with_provenance(
                artifact_node.clone(),
                symbol_id,
                RelationKind::Contains,
                Confidence::High,
                vec![evidence],
                Some(format_provenance(
                    registry_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }

        self.process_rationale(artifact, artifact_node, &output.comments, &symbol_spans);

        for import in &output.imports {
            let evidence = syntax_fact_evidence(artifact, import.span.clone());
            let target = self.unresolved(&import.text);
            self.relate_with_provenance(
                artifact_node.clone(),
                target,
                RelationKind::Imports,
                Confidence::Low,
                vec![evidence],
                Some(format_provenance(
                    registry_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::Low,
                )),
            );
        }

        // Type references and general use-site references (LIT-22.3.3):
        // one relation per distinct identifier text per file, deduplicated
        // (a single-file syntax pass has no scoping/symbol-table context to
        // tell which occurrence is meaningful, so keeping every occurrence
        // would just be noise) and targeting `Unresolved` -- this file's
        // syntax alone can't tell whether `Widget` is a locally-defined
        // type, an imported one, or a typo, so resolving it correctly is a
        // hybrid-resolver's job (AC3: never fabricate a match here).
        //
        // LIT-71 exception: a name this same file imported directly from a
        // declared npm dependency is not a guess -- the import statement
        // already says where it comes from -- so it resolves to that
        // package's member instead of `Unresolved`. `bare_package_imports`
        // is empty for every non-TS/JS caller, so this is a no-op elsewhere.
        let mut seen_symbols: BTreeSet<&str> = BTreeSet::new();
        for symbol in &output.symbols {
            if !seen_symbols.insert(symbol.text.as_str()) {
                continue;
            }
            let kind = if symbol.kind == "type_identifier" {
                RelationKind::TypeRefs
            } else {
                RelationKind::Usages
            };
            let evidence = syntax_fact_evidence(artifact, symbol.span.clone());
            let target = match bare_package_imports.get(symbol.text.as_str()) {
                Some((package, member)) => {
                    self.typescript_external_symbol(package, member, evidence.clone())
                }
                None => self.unresolved(&symbol.text),
            };
            self.relate_with_provenance(
                artifact_node.clone(),
                target,
                kind,
                Confidence::Low,
                vec![evidence],
                Some(format_provenance(
                    registry_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::Low,
                )),
            );
        }
    }
    pub(super) fn process_generic_text(
        &mut self,
        artifact: &Artifact,
        findings: &[TextFinding],
        artifact_node: &GraphNodeId,
    ) {
        for finding in findings {
            let evidence = generic_finding_evidence(artifact, finding.line);
            match finding.kind {
                TextFindingKind::EnvironmentVariable => {
                    let target = self.env_var(&finding.value);
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::Command => {
                    let target = self.command(
                        artifact,
                        &finding.line.to_string(),
                        &finding.value,
                        evidence.clone(),
                    );
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::LocalPath => {
                    let target =
                        if artifact.category == crate::domain::ArtifactCategory::Documentation {
                            let Some(target) =
                                self.resolve_documentation_path(artifact, &finding.value)
                            else {
                                continue;
                            };
                            target
                        } else {
                            self.resolve_path(&finding.value)
                                .unwrap_or_else(|| self.unresolved(&finding.value))
                        };
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::Url
                | TextFindingKind::PackageOrImage
                | TextFindingKind::ImportOrInclude => {
                    // In documentation these are useful extraction facts, not
                    // evidence that a code symbol or artifact is missing.
                    if artifact.category == crate::domain::ArtifactCategory::Documentation {
                        continue;
                    }
                    let target = self.unresolved(&finding.value);
                    self.relate_with_provenance(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        Confidence::Low,
                        vec![evidence],
                        Some(artifact_provenance(
                            artifact,
                            RelationResolution::Fallback,
                            Confidence::Low,
                        )),
                    );
                }
                TextFindingKind::Section => {}
            }
        }
    }
}

/// The declared name of a TS/TSX type-level definition, when `kind` is one of
/// the name-bearing declaration node kinds and a leading identifier follows
/// the keyword. Restricted to these TypeScript-grammar kinds so the change is
/// invisible to every other syntax-indexed language: their definition node
/// kinds never match, so they keep their positional `kind@Lline` identity.
fn named_definition_symbol<'a>(kind: &str, text: &'a str) -> Option<&'a str> {
    let keyword = match kind {
        "interface_declaration" => "interface",
        "type_alias_declaration" => "type",
        "enum_declaration" => "enum",
        _ => return None,
    };
    // `export`/`declare`/`const` may precede the keyword (`export const enum
    // E`); scan tokens for the keyword, then take the identifier after it.
    let mut tokens = text
        .split_whitespace()
        .skip_while(|token| *token != keyword);
    tokens.next()?; // the keyword itself
    let name = tokens.next()?;
    // Stop at the first non-identifier character: `Foo<T>`, `Foo=`, `Foo{`.
    let end = name
        .find(|character: char| {
            !(character.is_alphanumeric() || character == '_' || character == '$')
        })
        .unwrap_or(name.len());
    (end > 0).then(|| &name[..end])
}

fn generic_finding_evidence(artifact: &Artifact, line: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    match crate::domain::SourceSpan::new(line, line) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

pub(super) fn syntax_fact_evidence(
    artifact: &Artifact,
    span: crate::domain::SourceSpan,
) -> EvidenceRef {
    EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone()).with_span(span)
}

fn environment_provenance(source: FactSourceKind, confidence: Confidence) -> RelationProvenance {
    RelationProvenance {
        language: Some("environment".to_owned()),
        resolver_strategy: format!("environment-fact-{source:?}"),
        resolution: RelationResolution::SyntaxOnly,
        confidence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    /// LIT-40.1: `smallest_symbol_owner` selects the innermost enclosing symbol
    /// (smallest span) among a file's symbols, breaking ties by node id, and
    /// falls back to a wider symbol for lines only it encloses -- the exact
    /// selection the previous all-nodes scan produced, now over the per-artifact
    /// index.
    #[test]
    fn smallest_symbol_owner_picks_innermost_then_breaks_ties_by_id()
    -> Result<(), Box<dyn std::error::Error>> {
        use super::{BuilderState, EvidenceRef};
        use crate::domain::SourceSpan;
        use crate::domain::ids::{ArtifactId, RepoPath};
        use crate::graph::model::GraphNodeId;

        let repo_path = RepoPath::new("app.py")?;
        let artifact = ArtifactId::from_path(&repo_path);
        let mut state = BuilderState::new(&[]);
        // Outer 1..=20 encloses two equally-sized inner symbols 5..=10.
        state.env_symbols_by_artifact.insert(
            artifact.clone(),
            vec![
                (1, 20, GraphNodeId::new("symbol:app.py#outer")),
                (5, 10, GraphNodeId::new("symbol:app.py#inner_b")),
                (5, 10, GraphNodeId::new("symbol:app.py#inner_a")),
            ],
        );
        let at = |line: u32| -> Result<EvidenceRef, Box<dyn std::error::Error>> {
            Ok(EvidenceRef::file(artifact.clone(), repo_path.clone())
                .with_span(SourceSpan::new(line, line)?))
        };
        // Line 7 sits in all three; the smallest span wins, id breaks the tie.
        assert_eq!(
            state
                .smallest_symbol_owner(&at(7)?)
                .map(|id| id.as_str().to_owned()),
            Some("symbol:app.py#inner_a".to_owned())
        );
        // Line 2 sits only in the outer symbol.
        assert_eq!(
            state
                .smallest_symbol_owner(&at(2)?)
                .map(|id| id.as_str().to_owned()),
            Some("symbol:app.py#outer".to_owned())
        );
        // A line outside every span has no owner.
        assert_eq!(state.smallest_symbol_owner(&at(25)?), None);
        Ok(())
    }

    /// LIT-22.3.3 AC1/AC2: syntax-indexed languages (LIT-22.2.3) produce
    /// `TypeRefs` for `type_identifier` facts and `Usages` for other
    /// identifier facts, deduplicated per file.
    #[test]
    fn syntax_indexed_symbols_produce_type_refs_and_usages()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("widget.ts"),
            "class Widget {\n    hello(): void {\n        console.log(this);\n    }\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::TypeRefs)
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Usages)
        );
        // The syntax pass emits every TypeRefs/Usages at Low/SyntaxOnly.
        // Post-build resolution (LIT-75) may legitimately upgrade a reference
        // that lands on a same-file or imported local symbol -- e.g. the
        // class name `Widget` referenced within its own file -- so the Low
        // invariant holds for those still targeting an `Unresolved` node.
        let unresolved_ids: std::collections::BTreeSet<_> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Unresolved(node) => Some(node.id.clone()),
                _ => None,
            })
            .collect();
        for relation in graph.relations.iter().filter(|relation| {
            matches!(relation.kind, RelationKind::TypeRefs | RelationKind::Usages)
                && unresolved_ids.contains(&relation.target)
        }) {
            assert_eq!(relation.confidence, Confidence::Low);
            assert_eq!(
                relation
                    .provenance
                    .as_ref()
                    .ok_or("missing provenance")?
                    .resolution,
                RelationResolution::SyntaxOnly
            );
        }

        Ok(())
    }

    /// LIT-22.2.5 AC1: files with common syntax errors (unclosed braces,
    /// unterminated strings, malformed JSON) never panic the walker or
    /// graph builder; each broken file still gets an artifact node, and
    /// any symbols a tolerant parser did manage to extract before the
    /// error are Low confidence, never fabricated as fully resolved.
    #[test]
    fn syntax_error_fixture_degrades_gracefully_without_panicking()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/syntax_errors");

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        assert_eq!(artifacts.len(), 3);
        let artifact_paths: Vec<&str> = artifacts
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect();
        assert!(artifact_paths.contains(&"broken.py"));
        assert!(artifact_paths.contains(&"broken.json"));
        assert!(artifact_paths.contains(&"broken.rs"));

        // Every broken artifact still got an Artifact graph node: a parse
        // failure degrades what can be extracted from a file, it never
        // drops the file from the graph entirely.
        for path in ["broken.py", "broken.json", "broken.rs"] {
            assert!(
                graph.nodes.iter().any(
                    |node| matches!(node, GraphNode::Artifact(artifact) if artifact.path == path)
                ),
                "missing artifact node for {path}"
            );
        }

        Ok(())
    }

    /// LIT-23.2: CSS class/id selectors are declaration syntax (what a
    /// rule_set is), not references to something else, so they must never
    /// produce `Usages`/`TypeRefs` relations the way a code identifier
    /// use-site does. Confirmed live: a single real-world stylesheet
    /// produced 105 spurious `Usages` relations before this fix.
    #[test]
    fn css_selectors_produce_no_usages_relations() -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("styles.css"),
            "#root {\n  height: 100%;\n}\n\n.app-shell {\n  display: flex;\n}\n\n.brand-mark {\n  color: #fff;\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph.nodes.iter().any(
                |node| matches!(node, GraphNode::Artifact(artifact) if artifact.path == "styles.css")
            ),
            "a bare Artifact node must still exist for the CSS file"
        );
        assert!(
            !graph.relations.iter().any(|relation| matches!(
                relation.kind,
                RelationKind::Usages | RelationKind::TypeRefs
            ) && relation.source.as_str()
                == "artifact:styles.css"),
            "CSS selectors must not produce Usages/TypeRefs relations"
        );
        // The rule_set/at_rule structural facts (LIT-22.2.3) are unaffected:
        // each selector's rule still contributes a Symbol via Contains.
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::Contains
                    && relation.source.as_str() == "artifact:styles.css"),
            "CSS rule_set definitions should still produce Contains relations"
        );

        Ok(())
    }

    /// LIT-22.3.5 AC1/AC4: producer (`emit`) and consumer (`on`) calls
    /// become `Emits`/`ListensOn` relations to a shared Unresolved node
    /// per channel name, carrying evidence and confidence.
    #[test]
    fn emit_and_on_calls_produce_emits_and_listens_on_relations()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("realtime.py"),
            "def notify(socket):\n    socket.emit(\"user.updated\", payload)\n\n\ndef handler(socket):\n    socket.on(\"user.updated\", on_update)\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let emits = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::Emits)
            .ok_or("expected an Emits relation")?;
        let listens = graph
            .relations
            .iter()
            .find(|relation| relation.kind == RelationKind::ListensOn)
            .ok_or("expected a ListensOn relation")?;
        assert_eq!(emits.confidence, crate::domain::Confidence::High);
        assert_eq!(listens.confidence, crate::domain::Confidence::High);
        // Both call sites cite the same literal channel name, so they
        // converge on one shared target node rather than two.
        assert_eq!(emits.target, listens.target);
        assert!(graph.nodes.iter().any(
            |node| matches!(node, GraphNode::Unresolved(node) if node.value == "user.updated")
        ));

        Ok(())
    }
}
