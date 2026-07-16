use super::*;

impl BuilderState {
    pub(super) fn process_dockerfile(
        &mut self,
        artifact: &Artifact,
        analysis: DockerfileAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        for stage in &analysis.stages {
            let target = self.image(&stage.image);
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::UsesImage,
                Confidence::High,
                vec![stage.evidence.clone()],
            );
        }
        for env in analysis.env.iter().chain(analysis.args.iter()) {
            let target = self.env_var(&env.key);
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::ReadsEnv,
                Confidence::High,
                vec![env.evidence.clone()],
            );
        }
        for command in &analysis.commands {
            let key = command.line.to_string();
            let mut provenance = command_provenance(artifact, &command.evidence);
            if provenance == CommandProvenance::Executable && command.kind == DockerCommandKind::Run
            {
                provenance = CommandProvenance::BuildAutomation;
            }
            let target = self.command_with_provenance(
                artifact,
                &key,
                &command.command,
                command.evidence.clone(),
                provenance,
            );
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::RunsCommand,
                Confidence::High,
                vec![command.evidence.clone()],
            );
        }
    }
    pub(super) fn process_markdown(
        &mut self,
        artifact: &Artifact,
        analysis: MarkdownAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        for heading in &analysis.headings {
            let key = heading
                .evidence
                .span
                .as_ref()
                .map(|span| span.start_line)
                .unwrap_or(0);
            let id = self.insert(GraphNode::Documentation(DocumentationNode {
                id: GraphNodeId::new(format!("doc:{}#{key}", artifact.path)),
                title: heading.text.clone(),
                evidence: heading.evidence.clone(),
            }));
            self.relate(
                artifact_node.clone(),
                id,
                RelationKind::Contains,
                Confidence::High,
                vec![heading.evidence.clone()],
            );
        }
        for link in analysis
            .links
            .iter()
            .filter(|link| matches!(link.kind, crate::analysis::LinkKind::Local))
        {
            let Some(target) = self.resolve_documentation_path(artifact, &link.target) else {
                continue;
            };
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::References,
                Confidence::High,
                vec![link.evidence.clone()],
            );
        }
        for path_ref in &analysis.source_paths {
            let Some(target) = self.resolve_documentation_path(artifact, &path_ref.path) else {
                continue;
            };
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::References,
                Confidence::High,
                vec![path_ref.evidence.clone()],
            );
        }
        for command in &analysis.commands {
            let key = command
                .evidence
                .span
                .as_ref()
                .map(|span| span.start_line)
                .unwrap_or(0);
            let target = self.command(
                artifact,
                &key.to_string(),
                &command.command,
                command.evidence.clone(),
            );
            self.relate(
                artifact_node.clone(),
                target,
                RelationKind::RunsCommand,
                Confidence::High,
                vec![command.evidence.clone()],
            );
        }
    }
    pub(super) fn process_structured(
        &mut self,
        artifact: &Artifact,
        analysis: StructuredAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        for reference in &analysis.references {
            match reference.kind {
                ConfigReferenceKind::Path | ConfigReferenceKind::Url => {
                    let (target, confidence) = self.reference_target(&reference.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::References,
                        confidence,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Port => {
                    let target = self.config(
                        artifact,
                        &reference.config_path,
                        ConfigNodeKind::Port,
                        &reference.value,
                        reference.evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::Contains,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Image => {
                    let target = self.image(&reference.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::UsesImage,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Service => {
                    let target = self.config(
                        artifact,
                        &reference.config_path,
                        ConfigNodeKind::Service,
                        &reference.value,
                        reference.evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::Contains,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::Command => {
                    let target = self.command(
                        artifact,
                        &reference.config_path,
                        &reference.value,
                        reference.evidence.clone(),
                    );
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
                ConfigReferenceKind::EnvironmentVariable => {
                    let target = self.env_var(&reference.value);
                    self.relate(
                        artifact_node.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::High,
                        vec![reference.evidence.clone()],
                    );
                }
            }
        }
    }
    pub(super) fn process_cargo(&mut self, profile: CargoProfile, artifact_node: &GraphNodeId) {
        let Some(package) = &profile.package else {
            return;
        };
        let Some(name) = &package.name else { return };
        let package_id = self.package(name, false);
        self.relate_with_provenance(
            artifact_node.clone(),
            package_id.clone(),
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![package.evidence.clone()],
            Some(format_provenance(
                "toml",
                RelationResolution::SyntaxOnly,
                Confidence::High,
            )),
        );
        for dependency in &profile.dependencies {
            let dependency_id = self.package(&dependency.name, true);
            self.relate_with_provenance(
                package_id.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence.clone()],
                Some(format_provenance(
                    "toml",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }
    /// Records this repo's own declared Python dependency names (LIT-44.1),
    /// normalized for comparison against a Python file's import module
    /// segment. Called from a pre-pass before any Python file is indexed, so
    /// order relative to `pyproject.toml`/`requirements.txt` in `artifacts`
    /// doesn't matter -- see `python_external_target`.
    /// Records the crate names Cargo.toml declares, so a Rust path rooted at
    /// one is known to leave this repository (LIT-66). Underscores and
    /// hyphens are interchangeable between a Cargo name and its `use` path
    /// (`tree-sitter` is used as `tree_sitter`), so both spellings are kept.
    pub(super) fn register_rust_manifest_packages(&mut self, output: &AnalyzerOutput) {
        if let AnalyzerOutput::Cargo(profile) = output {
            for dependency in &profile.dependencies {
                // A `path:` dependency is a crate in this very repository --
                // ripgrep declares `grep`, `globset`, and `ignore` that way.
                // Calling those external would be a lie, and would hide real
                // intra-workspace edges behind a dependency node.
                if dependency
                    .requirement
                    .as_deref()
                    .is_some_and(|requirement| requirement.starts_with("path:"))
                {
                    continue;
                }
                self.rust_manifest_packages.insert(dependency.name.clone());
                self.rust_manifest_packages
                    .insert(dependency.name.replace('-', "_"));
            }
        }
    }

    pub(super) fn register_python_manifest_packages(&mut self, output: &AnalyzerOutput) {
        match output {
            AnalyzerOutput::PyProject(profile) => {
                let Some(project) = &profile.project else {
                    return;
                };
                for dependency in &project.dependencies {
                    let name = python_dependency_name(&dependency.requirement);
                    self.python_manifest_packages
                        .insert(normalize_python_package_name(name));
                }
            }
            AnalyzerOutput::Requirements(profile) => {
                for requirement in &profile.requirements {
                    self.python_manifest_packages
                        .insert(normalize_python_package_name(&requirement.name));
                }
            }
            _ => {}
        }
    }
    pub(super) fn process_pyproject(
        &mut self,
        profile: PyProjectProfile,
        artifact_node: &GraphNodeId,
    ) {
        let Some(project) = &profile.project else {
            return;
        };
        let Some(name) = &project.name else { return };
        let package_id = self.package(name, false);
        self.relate_with_provenance(
            artifact_node.clone(),
            package_id.clone(),
            RelationKind::BelongsToPackage,
            Confidence::High,
            vec![project.evidence.clone()],
            Some(format_provenance(
                "toml",
                RelationResolution::SyntaxOnly,
                Confidence::High,
            )),
        );
        for dependency in &project.dependencies {
            let dependency_name = python_dependency_name(&dependency.requirement);
            let dependency_id = self.package(dependency_name, true);
            self.relate_with_provenance(
                package_id.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence.clone()],
                Some(format_provenance(
                    "toml",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }
    pub(super) fn process_requirements(
        &mut self,
        profile: RequirementsProfile,
        artifact_node: &GraphNodeId,
    ) {
        for requirement in &profile.requirements {
            let dependency_id = self.package(&requirement.name, true);
            self.relate_with_provenance(
                artifact_node.clone(),
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![requirement.evidence.clone()],
                Some(format_provenance(
                    "requirements-txt",
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }
    /// Resolves a [`PackageManifestAnalysis`] (LIT-22.2.4) into a local
    /// `Package` node (when the format declares one) and one external
    /// `Package` node per dependency, mirroring `process_cargo`/
    /// `process_pyproject`. Dependencies attach to the local package node
    /// when one exists (so `DependsOnPackage` reads package-to-package, like
    /// Cargo/pyproject), falling back to the artifact node otherwise (e.g.
    /// Gradle, which has no in-file local package name).
    pub(super) fn process_package_manifest(
        &mut self,
        format: PackageManifestFormat,
        analysis: PackageManifestAnalysis,
        artifact_node: &GraphNodeId,
    ) {
        let format_id = format.format_id();
        let local_package_id = analysis.local_package.map(|local| {
            let package_id = self.package(&local.name, false);
            self.relate_with_provenance(
                artifact_node.clone(),
                package_id.clone(),
                RelationKind::BelongsToPackage,
                Confidence::High,
                vec![local.evidence],
                Some(format_provenance(
                    format_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
            package_id
        });

        for dependency in analysis.dependencies {
            let dependency_id = self.package(&dependency.name, true);
            let source = local_package_id
                .clone()
                .unwrap_or_else(|| artifact_node.clone());
            self.relate_with_provenance(
                source,
                dependency_id,
                RelationKind::DependsOnPackage,
                Confidence::High,
                vec![dependency.evidence],
                Some(format_provenance(
                    format_id,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }
    /// Turns gRPC/protobuf `service.rpc` and GraphQL `Query`/`Mutation`
    /// field declarations (LIT-22.3.4 AC1/AC2) into first-class `Route`
    /// config nodes, the same node kind Python's route decorators produce
    /// (see `process_python_route_decorators`), so both surface uniformly
    /// in `KnowledgeIndex::architecture()`'s service links (AC3).
    pub(super) fn process_protocol_routes(
        &mut self,
        artifact: &Artifact,
        routes: &[ProtocolRoute],
        artifact_node: &GraphNodeId,
    ) {
        for (index, route) in routes.iter().enumerate() {
            let key = format!("route.{index}");
            let route_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Route,
                &route.name,
                route.evidence.clone(),
            );
            self.relate_with_provenance(
                artifact_node.clone(),
                route_id,
                RelationKind::Contains,
                Confidence::High,
                vec![route.evidence.clone()],
                Some(artifact_provenance(
                    artifact,
                    RelationResolution::SyntaxOnly,
                    Confidence::High,
                )),
            );
        }
    }
    pub(super) fn process_compose(
        &mut self,
        artifact: &Artifact,
        profile: ComposeProfile,
        artifact_node: &GraphNodeId,
    ) {
        for service in &profile.services {
            let key = format!("services.{}", service.name);
            let service_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Service,
                &service.name,
                service.evidence.clone(),
            );
            self.relate(
                artifact_node.clone(),
                service_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![service.evidence.clone()],
            );
            if let Some(image) = &service.image {
                let target = self.image(image);
                self.relate(
                    service_id.clone(),
                    target,
                    RelationKind::UsesImage,
                    Confidence::High,
                    vec![service.evidence.clone()],
                );
            }
            for env in &service.environment {
                let target = self.env_var(&env.key);
                self.relate(
                    service_id.clone(),
                    target,
                    RelationKind::ReadsEnv,
                    Confidence::High,
                    vec![env.evidence.clone()],
                );
            }
            for depends_on in &service.depends_on {
                let dependency_key = format!("services.{depends_on}");
                let target = if profile
                    .services
                    .iter()
                    .any(|other| &other.name == depends_on)
                {
                    GraphNodeId::new(format!("config:{}#{dependency_key}", artifact.path))
                } else {
                    self.unresolved(depends_on)
                };
                self.relate(
                    service_id.clone(),
                    target,
                    RelationKind::References,
                    Confidence::High,
                    vec![service.evidence.clone()],
                );
            }
        }
    }
    pub(super) fn process_actions(
        &mut self,
        artifact: &Artifact,
        profile: ActionsProfile,
        artifact_node: &GraphNodeId,
    ) {
        for job in &profile.jobs {
            let key = format!("jobs.{}", job.id);
            let job_id = self.config(
                artifact,
                &key,
                ConfigNodeKind::Job,
                &job.id,
                job.evidence.clone(),
            );
            self.relate(
                artifact_node.clone(),
                job_id.clone(),
                RelationKind::Contains,
                Confidence::High,
                vec![job.evidence.clone()],
            );
            for (index, step) in job.steps.iter().enumerate() {
                for env in &step.env {
                    let target = self.env_var(&env.key);
                    self.relate(
                        job_id.clone(),
                        target,
                        RelationKind::ReadsEnv,
                        Confidence::High,
                        vec![env.evidence.clone()],
                    );
                }
                if let Some(run) = &step.run {
                    let step_key = format!("{key}.steps[{index}]");
                    let target = self.command(artifact, &step_key, run, step.evidence.clone());
                    self.relate(
                        job_id.clone(),
                        target,
                        RelationKind::RunsCommand,
                        Confidence::High,
                        vec![step.evidence.clone()],
                    );
                }
                match &step.hint {
                    Some(ActionsStepHint::Build { image }) => {
                        let (target, confidence) = self.hint_image_target(image);
                        self.relate(
                            job_id.clone(),
                            target,
                            RelationKind::BuildsImage,
                            confidence,
                            vec![step.evidence.clone()],
                        );
                    }
                    Some(ActionsStepHint::Publish { image }) => {
                        let (target, confidence) = self.hint_image_target(image);
                        self.relate(
                            job_id.clone(),
                            target,
                            RelationKind::PublishesImage,
                            confidence,
                            vec![step.evidence.clone()],
                        );
                    }
                    None => {}
                }
            }
        }
    }
    fn hint_image_target(&mut self, image: &Option<String>) -> (GraphNodeId, Confidence) {
        match image {
            Some(image) => (self.image(image), Confidence::High),
            None => (self.unresolved("dynamic-image"), Confidence::Low),
        }
    }
}

fn python_dependency_name(requirement: &str) -> &str {
    let end = requirement
        .find(|character: char| {
            !(character.is_alphanumeric()
                || character == '-'
                || character == '_'
                || character == '.')
        })
        .unwrap_or(requirement.len());
    &requirement[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inventory::{RepositoryWalker, WalkOptions};

    #[test]
    fn documentation_noise_does_not_mint_unresolved_nodes() -> Result<(), Box<dyn std::error::Error>>
    {
        let temp = tempfile::TempDir::new()?;
        std::fs::create_dir_all(temp.path().join("docs"))?;
        std::fs::create_dir_all(temp.path().join(".github/workflows"))?;
        std::fs::write(temp.path().join("docs/guide.md"), "# Guide\n")?;
        // The default inventory intentionally excludes this hidden tree. A
        // physical file outside the artifact set is still not a graph target,
        // but documentation mentioning it must not become resolver failure.
        std::fs::write(temp.path().join(".github/workflows/ci.yml"), "name: CI\n")?;
        std::fs::write(
            temp.path().join("README.md"),
            "# Project\n\nSee [guide](docs/guide.md), [missing](docs/missing.md), `.github/workflows/ci.yml`, and https://example.test.\n\nUse `render_template('blog/create.html')`.\n```python\n@app.post(\"/add\")\n```\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let unresolved = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Unresolved(node) => Some(node.value.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();

        assert!(unresolved.is_empty(), "{unresolved:#?}");
        assert!(
            graph
                .nodes
                .iter()
                .all(|node| node.id().as_str() != "artifact:.github/workflows/ci.yml")
        );
        assert!(graph.relations.iter().any(|relation| {
            relation.source.as_str() == "artifact:README.md"
                && relation.target.as_str() == "artifact:docs/guide.md"
                && relation.kind == RelationKind::References
        }));
        Ok(())
    }

    #[test]
    fn builder_assigns_documentation_and_build_command_provenance()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("README.md"),
            "# Setup\n\n```sh\n$ . .venv/bin/activate\n```\n",
        )?;
        std::fs::create_dir(temp.path().join("docs"))?;
        std::fs::write(
            temp.path().join("docs/Makefile"),
            "html:\n\tsphinx-build -M html . _build\n",
        )?;
        std::fs::write(temp.path().join("Makefile"), "test:\n\tcargo test\n")?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);
        let commands = graph.nodes.iter().filter_map(|node| match node {
            GraphNode::Command(command) => Some(command),
            _ => None,
        });
        let provenances = commands
            .map(|command| (command.text.as_str(), command.provenance))
            .collect::<BTreeMap<_, _>>();

        assert_eq!(
            provenances.get(". .venv/bin/activate"),
            Some(&CommandProvenance::DocumentationExample)
        );
        assert!(
            provenances
                .get("sphinx-build -M html . _build")
                .is_none_or(|provenance| *provenance == CommandProvenance::DocumentationExample)
        );
        assert_eq!(
            provenances.get("cargo test"),
            Some(&CommandProvenance::BuildAutomation)
        );
        Ok(())
    }

    #[test]
    fn graph_links_dockerfile_and_compose_to_image_nodes() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = fixture_root();
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        let graph = GraphBuilder.build(&root, &artifacts);

        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::UsesImage
                    && relation.source.as_str() == "artifact:Dockerfile")
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::UsesImage
                    && relation.target.as_str() == "image:node:24-alpine")
        );
        assert!(
            graph
                .nodes
                .iter()
                .any(|node| node.id().as_str() == "package:fixture-worker")
        );
        assert!(
            graph
                .relations
                .iter()
                .any(|relation| relation.kind == RelationKind::DependsOnPackage
                    && relation.target.as_str() == "package:anyhow")
        );

        Ok(())
    }

    /// LIT-22.2.4 AC1/AC2/AC4: an isolated repo (not the shared polyglot
    /// fixture, to avoid golden-snapshot churn across the rest of the test
    /// suite) exercising every wired package manifest format end to end --
    /// local vs. external `Package` nodes and `DependsOnPackage` edges.
    #[test]
    fn package_manifests_produce_local_and_external_package_nodes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        let root = temp.path();
        std::fs::write(
            root.join("package.json"),
            r#"{"name": "acme-web", "version": "1.0.0", "dependencies": {"react": "^18.0.0"}}"#,
        )?;
        std::fs::write(
            root.join("go.mod"),
            "module github.com/acme/svc\n\nrequire github.com/gin-gonic/gin v1.9.1\n",
        )?;
        std::fs::write(
            root.join("composer.json"),
            r#"{"name": "acme/php-app", "require": {"guzzlehttp/guzzle": "^7.0"}}"#,
        )?;
        std::fs::write(
            root.join("pom.xml"),
            "<project><groupId>com.acme</groupId><artifactId>svc</artifactId><version>1.0</version>\
             <dependencies><dependency><groupId>org.apache.commons</groupId>\
             <artifactId>commons-lang3</artifactId><version>3.14.0</version></dependency>\
             </dependencies></project>",
        )?;
        std::fs::write(
            root.join("build.gradle"),
            "dependencies {\n    implementation(\"com.squareup.okhttp3:okhttp:4.12.0\")\n}\n",
        )?;
        std::fs::create_dir_all(root.join("dotnet"))?;
        std::fs::write(
            root.join("dotnet/App.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup>
                <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
            </ItemGroup></Project>"#,
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(root)?;
        let graph = GraphBuilder.build(root, &artifacts);

        let expectations = [
            ("acme-web", false, "react", true),
            (
                "github.com/acme/svc",
                false,
                "github.com/gin-gonic/gin",
                true,
            ),
            ("acme/php-app", false, "guzzlehttp/guzzle", true),
            (
                "com.acme:svc",
                false,
                "org.apache.commons:commons-lang3",
                true,
            ),
            ("com.squareup.okhttp3:okhttp", true, "", false),
            ("App", false, "Newtonsoft.Json", true),
        ];
        for (local_name, local_is_external, dependency_name, has_dependency) in expectations {
            let local = graph
                .nodes
                .iter()
                .find_map(|node| match node {
                    GraphNode::Package(package) if package.name == local_name => Some(package),
                    _ => None,
                })
                .ok_or_else(|| std::io::Error::other(format!("missing package {local_name}")))?;
            assert_eq!(
                local.is_external, local_is_external,
                "{local_name} is_external mismatch"
            );

            if !has_dependency {
                continue;
            }
            let dependency = graph
                .nodes
                .iter()
                .find_map(|node| match node {
                    GraphNode::Package(package) if package.name == dependency_name => Some(package),
                    _ => None,
                })
                .ok_or_else(|| {
                    std::io::Error::other(format!("missing dependency {dependency_name}"))
                })?;
            assert!(dependency.is_external, "{dependency_name} must be external");
            assert!(
                graph.relations.iter().any(|relation| {
                    relation.kind == RelationKind::DependsOnPackage
                        && relation.target == dependency.id
                        && relation
                            .provenance
                            .as_ref()
                            .is_some_and(|p| p.resolution == RelationResolution::SyntaxOnly)
                }),
                "missing DependsOnPackage relation to {dependency_name}"
            );
        }

        Ok(())
    }

    /// LIT-22.3.4 AC2: `.proto` and `.graphql` schema facts produce `Route`
    /// config nodes.
    #[test]
    fn proto_and_graphql_schemas_produce_route_config_nodes()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("api.proto"),
            "service Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply) {}\n}\n",
        )?;
        std::fs::write(
            temp.path().join("schema.graphql"),
            "type Query {\n  user(id: ID!): User\n}\n",
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        let route_names: Vec<&str> = graph
            .nodes
            .iter()
            .filter_map(|node| match node {
                GraphNode::Config(config)
                    if config.kind == crate::graph::model::ConfigNodeKind::Route =>
                {
                    Some(config.name.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(route_names.contains(&"Greeter.SayHello"));
        assert!(route_names.contains(&"Query.user"));

        Ok(())
    }

    /// LIT-23.3: `package-lock.json`'s internal dependency-tree fields
    /// (`resolved` URLs, `bin` entries, integrity hashes) must not produce
    /// spurious reference/config relations the way hand-written JSON
    /// config would. Confirmed live: a single real-world lockfile produced
    /// 504 spurious relations before this fix.
    #[test]
    fn package_lock_json_produces_no_spurious_reference_relations()
    -> Result<(), Box<dyn std::error::Error>> {
        let temp = tempfile::TempDir::new()?;
        std::fs::write(
            temp.path().join("package-lock.json"),
            r#"{
  "name": "app",
  "lockfileVersion": 3,
  "packages": {
    "": { "dependencies": { "esbuild": "^0.21.0" } },
    "node_modules/esbuild": {
      "version": "0.21.5",
      "resolved": "https://registry.npmjs.org/esbuild/-/esbuild-0.21.5.tgz",
      "integrity": "sha512-abc123==",
      "bin": { "esbuild": "bin/esbuild" },
      "engines": { "node": ">=12" }
    }
  }
}
"#,
        )?;

        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(temp.path())?;
        let graph = GraphBuilder.build(temp.path(), &artifacts);

        assert!(
            graph.nodes.iter().any(
                |node| matches!(node, GraphNode::Artifact(artifact) if artifact.path == "package-lock.json")
            ),
            "a bare Artifact node must still exist for the lockfile"
        );
        assert!(
            !graph
                .relations
                .iter()
                .any(|relation| relation.source.as_str() == "artifact:package-lock.json"),
            "a lockfile must produce no relations at all from its content"
        );

        Ok(())
    }
}
