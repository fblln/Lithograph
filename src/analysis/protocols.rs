//! Cross-service protocol schema analyzers (LIT-22.3.4): gRPC/protobuf
//! `service`/`rpc` declarations and GraphQL `Query`/`Mutation` schema
//! fields. Both formats are simple, brace-delimited declaration lists, so
//! this parses them directly line-by-line rather than pulling in a full
//! grammar -- there is no other syntax in either format this needs to
//! understand.

use crate::domain::{Artifact, ArtifactId, EvidenceRef, SourceSpan};
use serde::{Deserialize, Serialize};

/// Which protocol schema format was parsed. `Copy` so it can be used as an
/// [`AnalyzerKind`](crate::analysis::AnalyzerKind) cache-key discriminant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProtocolFormat {
    /// `.proto` (protobuf/gRPC).
    Proto,
    /// `.graphql`/`.gql` schema.
    GraphQl,
}

impl ProtocolFormat {
    /// Looks up the variant matching a classifier/registry format id (see
    /// `inventory::classify`/`inventory::language`).
    pub fn from_format_id(id: &str) -> Option<Self> {
        Some(match id {
            "protobuf" => Self::Proto,
            "graphql" => Self::GraphQl,
            _ => return None,
        })
    }

    /// Runs this format's analyzer against `text`.
    pub fn analyze(self, artifact: &Artifact, text: &str) -> Vec<ProtocolRoute> {
        match self {
            Self::Proto => ProtoAnalyzer.analyze(artifact, text),
            Self::GraphQl => GraphQlAnalyzer.analyze(artifact, text),
        }
    }
}

/// One RPC or schema field declaration extracted from a protocol schema
/// file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolRoute {
    /// `"service.rpc"` (protobuf) or `"Query.field"`/`"Mutation.field"`
    /// (GraphQL).
    pub name: String,
    /// Evidence for the declaration line.
    pub evidence: EvidenceRef,
}

fn line_evidence(artifact: &Artifact, line_number: u32) -> EvidenceRef {
    let base = EvidenceRef::file(ArtifactId::from_path(&artifact.path), artifact.path.clone());
    match SourceSpan::new(line_number, line_number) {
        Ok(span) => base.with_span(span),
        Err(_) => base,
    }
}

/// Parser-backed analyzer for `.proto` (protobuf/gRPC) schema files.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProtoAnalyzer;

impl ProtoAnalyzer {
    /// Extracts one [`ProtocolRoute`] per `rpc` declaration inside each
    /// `service { ... }` block.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> Vec<ProtocolRoute> {
        let mut routes = Vec::new();
        let mut current_service: Option<String> = None;
        let mut depth_at_service_start = 0i32;
        let mut depth = 0i32;

        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.split("//").next().unwrap_or("").trim();
            let line_number = index as u32 + 1;

            if let Some(rest) = line.strip_prefix("service ") {
                let name = rest.split(['{', ' ']).next().unwrap_or("").trim();
                if !name.is_empty() {
                    current_service = Some(name.to_owned());
                    depth_at_service_start = depth;
                }
            } else if let Some(service) = &current_service
                && let Some(rest) = line.strip_prefix("rpc ")
            {
                let name = rest.split(['(', ' ']).next().unwrap_or("").trim();
                if !name.is_empty() {
                    routes.push(ProtocolRoute {
                        name: format!("{service}.{name}"),
                        evidence: line_evidence(artifact, line_number),
                    });
                }
            }

            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if current_service.is_some() && depth <= depth_at_service_start {
                current_service = None;
            }
        }

        routes
    }
}

/// Parser-backed analyzer for GraphQL schema files: extracts each field
/// declared directly inside a top-level `type Query { ... }` or
/// `type Mutation { ... }` block.
#[derive(Debug, Clone, Copy, Default)]
pub struct GraphQlAnalyzer;

impl GraphQlAnalyzer {
    /// Extracts one [`ProtocolRoute`] per `Query`/`Mutation` field.
    pub fn analyze(&self, artifact: &Artifact, text: &str) -> Vec<ProtocolRoute> {
        let mut routes = Vec::new();
        let mut current_root: Option<String> = None;
        let mut depth_at_root_start = 0i32;
        let mut depth = 0i32;

        for (index, raw_line) in text.lines().enumerate() {
            let line = raw_line.split('#').next().unwrap_or("").trim();
            let line_number = index as u32 + 1;

            if let Some(rest) = line
                .strip_prefix("type Query")
                .or_else(|| line.strip_prefix("type Mutation"))
            {
                let root = if line.starts_with("type Query") {
                    "Query"
                } else {
                    "Mutation"
                };
                let _ = rest;
                current_root = Some(root.to_owned());
                depth_at_root_start = depth;
            } else if let Some(root) = &current_root
                && depth == depth_at_root_start + 1
                && let Some(name) = graphql_field_name(line)
            {
                routes.push(ProtocolRoute {
                    name: format!("{root}.{name}"),
                    evidence: line_evidence(artifact, line_number),
                });
            }

            depth += line.matches('{').count() as i32 - line.matches('}').count() as i32;
            if current_root.is_some() && depth <= depth_at_root_start {
                current_root = None;
            }
        }

        routes
    }
}

/// Extracts a GraphQL field name from one schema body line
/// (`user(id: ID!): User` or `users: [User!]!` -> `"user"`/`"users"`).
/// Returns `None` for blank lines or lines that aren't a field declaration
/// (e.g. a stray `{`).
fn graphql_field_name(line: &str) -> Option<&str> {
    let name = line.split(['(', ':']).next()?.trim();
    (!name.is_empty() && name != "{" && name != "}").then_some(name)
}

#[cfg(test)]
mod tests {
    use super::{GraphQlAnalyzer, ProtoAnalyzer};
    use crate::domain::{
        Artifact, ArtifactCategory, ContentHash, RepoPath, SupportTier, TextStatus,
    };

    fn artifact(path: &str) -> Result<Artifact, Box<dyn std::error::Error>> {
        Ok(Artifact::new(
            RepoPath::new(path)?,
            ArtifactCategory::SourceCode,
            SupportTier::StructuredFormat,
            ContentHash::new("aaaaaaaa")?,
            10,
        )
        .with_text_status(TextStatus::Text, Some(1)))
    }

    #[test]
    fn proto_extracts_rpcs_scoped_to_their_service() -> Result<(), Box<dyn std::error::Error>> {
        let text = "syntax = \"proto3\";\n\nservice Greeter {\n  rpc SayHello (HelloRequest) returns (HelloReply) {}\n  rpc SayGoodbye (ByeRequest) returns (ByeReply) {}\n}\n\nmessage HelloRequest { string name = 1; }\n\nservice Other {\n  rpc Ping (PingRequest) returns (PingReply) {}\n}\n";
        let routes = ProtoAnalyzer.analyze(&artifact("api.proto")?, text);

        let names: Vec<&str> = routes.iter().map(|route| route.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Greeter.SayHello", "Greeter.SayGoodbye", "Other.Ping"]
        );

        Ok(())
    }

    #[test]
    fn proto_with_no_service_produces_no_routes() -> Result<(), Box<dyn std::error::Error>> {
        let routes = ProtoAnalyzer.analyze(
            &artifact("messages.proto")?,
            "message Foo { string bar = 1; }\n",
        );
        assert!(routes.is_empty());

        Ok(())
    }

    #[test]
    fn graphql_extracts_query_and_mutation_fields_only() -> Result<(), Box<dyn std::error::Error>> {
        let text = "type Query {\n  user(id: ID!): User\n  users: [User!]!\n}\n\ntype Mutation {\n  createUser(input: CreateUserInput!): User\n}\n\ntype User {\n  id: ID!\n  name: String\n}\n";
        let routes = GraphQlAnalyzer.analyze(&artifact("schema.graphql")?, text);

        let names: Vec<&str> = routes.iter().map(|route| route.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["Query.user", "Query.users", "Mutation.createUser"]
        );

        Ok(())
    }
}
