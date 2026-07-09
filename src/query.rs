//! A narrow Cypher-like `MATCH`/`WHERE`/`RETURN` query subset for common
//! graph exploration, without exposing arbitrary execution (LIT-22.4.5):
//! the parser only ever accepts one fixed shape --
//! `MATCH (a:Label)[-[:KIND]->(b:Label)] [WHERE a.prop OP "value"] RETURN a[, b]`
//! -- never a general expression language.

use crate::graph::index::{node_file_path, node_label, node_name};
use crate::graph::{Graph, GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::fmt::{Display, Formatter};

/// A validation or parse failure, always with an actionable message (AC2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryError {
    /// Human-readable, actionable description of what was wrong.
    pub message: String,
}

impl Display for QueryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for QueryError {}

fn error(message: impl Into<String>) -> QueryError {
    QueryError {
        message: message.into(),
    }
}

/// One node pattern: `(alias:Label)`, `Label` optional.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NodePattern {
    alias: String,
    label: Option<String>,
}

/// One relation hop: `-[:KIND]->` or `-->` (kind omitted matches any).
#[derive(Debug, Clone, PartialEq, Eq)]
struct RelationPattern {
    kind: Option<String>,
}

/// `WHERE alias.property OP "value"`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WhereClause {
    alias: String,
    property: String,
    operator: WhereOperator,
    value: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WhereOperator {
    Equals,
    Contains,
}

/// One parsed, validated query (AC1): a fixed node, or node-edge-node,
/// pattern with an optional filter and a required projection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatchQuery {
    source: NodePattern,
    hop: Option<(RelationPattern, NodePattern)>,
    filter: Option<WhereClause>,
    return_aliases: Vec<String>,
}

/// One returned row: the matched node plus structured graph refs and
/// evidence (AC3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryRow {
    /// Which `RETURN` alias this row's node came from.
    pub alias: String,
    /// Matched graph node id.
    pub id: GraphNodeId,
    /// Node label (`Symbol`, `Artifact`, ...).
    pub label: String,
    /// Display name.
    pub name: String,
    /// Repository-relative file path, when the node has one.
    pub file_path: Option<String>,
}

/// Parses `text` into a [`MatchQuery`] (AC1/AC2).
pub fn parse(text: &str) -> Result<MatchQuery, QueryError> {
    let tokens = tokenize(text)?;
    let mut parser = Parser {
        tokens,
        position: 0,
    };
    parser.parse_query()
}

/// Runs `query` against `graph`, returning one [`QueryRow`] per matched
/// pattern occurrence, per `RETURN` alias, in a stable order.
pub fn evaluate(query: &MatchQuery, graph: &Graph) -> Vec<QueryRow> {
    let candidates: Vec<&GraphNode> = graph
        .nodes
        .iter()
        .filter(|node| matches_label(node, query.source.label.as_deref()))
        .collect();

    let mut bindings: Vec<Vec<(&str, &GraphNode)>> = Vec::new();
    if let Some((relation, target_pattern)) = &query.hop {
        for source_node in &candidates {
            for target_node in graph
                .nodes
                .iter()
                .filter(|node| matches_label(node, target_pattern.label.as_deref()))
            {
                let connected = graph.relations.iter().any(|edge| {
                    edge.source == *source_node.id()
                        && edge.target == *target_node.id()
                        && relation
                            .kind
                            .as_deref()
                            .is_none_or(|kind| relation_kind_matches(edge.kind, kind))
                });
                if connected {
                    bindings.push(vec![
                        (query.source.alias.as_str(), *source_node),
                        (target_pattern.alias.as_str(), target_node),
                    ]);
                }
            }
        }
    } else {
        for source_node in &candidates {
            bindings.push(vec![(query.source.alias.as_str(), *source_node)]);
        }
    }

    if let Some(filter) = &query.filter {
        bindings.retain(|binding| {
            binding
                .iter()
                .find(|(alias, _)| *alias == filter.alias)
                .is_some_and(|(_, node)| where_matches(node, filter))
        });
    }

    let mut rows = Vec::new();
    for binding in &bindings {
        for return_alias in &query.return_aliases {
            if let Some((_, node)) = binding.iter().find(|(alias, _)| alias == return_alias) {
                rows.push(QueryRow {
                    alias: return_alias.clone(),
                    id: node.id().clone(),
                    label: node_label(node).to_owned(),
                    name: node_name(node),
                    file_path: node_file_path(node),
                });
            }
        }
    }
    rows.sort_by(|a, b| a.alias.cmp(&b.alias).then_with(|| a.id.cmp(&b.id)));
    rows.dedup();
    rows
}

fn matches_label(node: &GraphNode, label: Option<&str>) -> bool {
    label.is_none_or(|wanted| node_label(node).eq_ignore_ascii_case(wanted))
}

fn relation_kind_matches(kind: RelationKind, wanted: &str) -> bool {
    format!("{kind:?}").eq_ignore_ascii_case(wanted)
}

fn where_matches(node: &GraphNode, filter: &WhereClause) -> bool {
    let Some(actual) = node_property(node, &filter.property) else {
        return false;
    };
    match filter.operator {
        WhereOperator::Equals => actual == filter.value,
        WhereOperator::Contains => actual.to_lowercase().contains(&filter.value.to_lowercase()),
    }
}

/// The only two properties this narrow subset exposes (AC1): `name` (the
/// node's display name) and `path` (its file path, when it has one).
fn node_property(node: &GraphNode, property: &str) -> Option<String> {
    match property {
        "name" => Some(node_name(node)),
        "path" => node_file_path(node),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Match,
    Where,
    Return,
    Contains,
    LParen,
    RParen,
    LBracket,
    RBracket,
    Dash,
    GreaterThan,
    Colon,
    Dot,
    Comma,
    Equals,
    Identifier(String),
    StringLiteral(String),
}

fn tokenize(text: &str) -> Result<Vec<Token>, QueryError> {
    let mut tokens = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        let ch = chars[index];
        match ch {
            ch if ch.is_whitespace() => index += 1,
            '(' => {
                tokens.push(Token::LParen);
                index += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                index += 1;
            }
            '[' => {
                tokens.push(Token::LBracket);
                index += 1;
            }
            ']' => {
                tokens.push(Token::RBracket);
                index += 1;
            }
            '-' => {
                tokens.push(Token::Dash);
                index += 1;
            }
            '>' => {
                tokens.push(Token::GreaterThan);
                index += 1;
            }
            ':' => {
                tokens.push(Token::Colon);
                index += 1;
            }
            '.' => {
                tokens.push(Token::Dot);
                index += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                index += 1;
            }
            '=' => {
                tokens.push(Token::Equals);
                index += 1;
            }
            '"' | '\'' => {
                let quote = ch;
                let start = index + 1;
                let mut end = start;
                while end < chars.len() && chars[end] != quote {
                    end += 1;
                }
                if end >= chars.len() {
                    return Err(error(format!(
                        "unterminated string literal starting at character {index}"
                    )));
                }
                tokens.push(Token::StringLiteral(chars[start..end].iter().collect()));
                index = end + 1;
            }
            ch if ch.is_alphanumeric() || ch == '_' => {
                let start = index;
                while index < chars.len() && (chars[index].is_alphanumeric() || chars[index] == '_')
                {
                    index += 1;
                }
                let word: String = chars[start..index].iter().collect();
                tokens.push(match word.to_uppercase().as_str() {
                    "MATCH" => Token::Match,
                    "WHERE" => Token::Where,
                    "RETURN" => Token::Return,
                    "CONTAINS" => Token::Contains,
                    _ => Token::Identifier(word),
                });
            }
            other => {
                return Err(error(format!(
                    "unexpected character `{other}` at position {index}; \
                     only MATCH (a:Label)[-[:KIND]->(b:Label)] [WHERE a.prop OP \"value\"] RETURN a[, b] is supported"
                )));
            }
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    position: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }

    fn advance(&mut self) -> Option<Token> {
        let token = self.tokens.get(self.position).cloned();
        self.position += 1;
        token
    }

    fn expect(&mut self, expected: &Token, what: &str) -> Result<(), QueryError> {
        match self.advance() {
            Some(token) if &token == expected => Ok(()),
            Some(other) => Err(error(format!("expected {what}, found {other:?}"))),
            None => Err(error(format!("expected {what}, but the query ended"))),
        }
    }

    /// Reads one identifier. Reserved words (`MATCH`, `WHERE`, `RETURN`,
    /// `CONTAINS`) are usable as identifiers here too -- `RelationKind`'s
    /// own `Contains` variant would otherwise be unspellable as a
    /// relation-kind identifier, since the tokenizer has no way to know
    /// ahead of time which grammar position a word will land in.
    fn expect_identifier(&mut self, what: &str) -> Result<String, QueryError> {
        match self.advance() {
            Some(Token::Identifier(name)) => Ok(name),
            Some(Token::Match) => Ok("MATCH".to_owned()),
            Some(Token::Where) => Ok("WHERE".to_owned()),
            Some(Token::Return) => Ok("RETURN".to_owned()),
            Some(Token::Contains) => Ok("Contains".to_owned()),
            Some(other) => Err(error(format!("expected {what}, found {other:?}"))),
            None => Err(error(format!("expected {what}, but the query ended"))),
        }
    }

    fn parse_node_pattern(&mut self) -> Result<NodePattern, QueryError> {
        self.expect(&Token::LParen, "`(` starting a node pattern")?;
        let alias = self.expect_identifier("a node alias, e.g. `a`")?;
        let label = if matches!(self.peek(), Some(Token::Colon)) {
            self.advance();
            Some(self.expect_identifier("a node label, e.g. `Symbol`")?)
        } else {
            None
        };
        self.expect(&Token::RParen, "`)` closing the node pattern")?;
        Ok(NodePattern { alias, label })
    }

    fn parse_query(&mut self) -> Result<MatchQuery, QueryError> {
        self.expect(&Token::Match, "`MATCH` at the start of the query")?;
        let source = self.parse_node_pattern()?;

        let hop = if matches!(self.peek(), Some(Token::Dash)) {
            self.advance();
            let kind = if matches!(self.peek(), Some(Token::LBracket)) {
                self.advance();
                let kind = if matches!(self.peek(), Some(Token::Colon)) {
                    self.advance();
                    Some(self.expect_identifier("a relation kind, e.g. `Calls`")?)
                } else {
                    None
                };
                self.expect(&Token::RBracket, "`]` closing the relation pattern")?;
                kind
            } else {
                None
            };
            self.expect(&Token::Dash, "`-` in `-[:KIND]->`")?;
            self.expect(&Token::GreaterThan, "`>` in `-[:KIND]->`")?;
            let target = self.parse_node_pattern()?;
            Some((RelationPattern { kind }, target))
        } else {
            None
        };

        let filter = if matches!(self.peek(), Some(Token::Where)) {
            self.advance();
            let alias = self.expect_identifier("a bound alias in the WHERE clause")?;
            self.expect(&Token::Dot, "`.` between the alias and its property")?;
            let property = self.expect_identifier("a property name, e.g. `name` or `path`")?;
            let operator = match self.advance() {
                Some(Token::Equals) => WhereOperator::Equals,
                Some(Token::Contains) => WhereOperator::Contains,
                Some(other) => {
                    return Err(error(format!(
                        "expected `=` or `CONTAINS` in the WHERE clause, found {other:?}"
                    )));
                }
                None => return Err(error("expected `=` or `CONTAINS`, but the query ended")),
            };
            let value = match self.advance() {
                Some(Token::StringLiteral(value)) => value,
                Some(other) => {
                    return Err(error(format!(
                        "expected a quoted string value in the WHERE clause, found {other:?}"
                    )));
                }
                None => return Err(error("expected a quoted string value, but the query ended")),
            };
            Some(WhereClause {
                alias,
                property,
                operator,
                value,
            })
        } else {
            None
        };

        self.expect(&Token::Return, "`RETURN` before the projection list")?;
        let mut return_aliases = vec![self.expect_identifier("an alias to return")?];
        while matches!(self.peek(), Some(Token::Comma)) {
            self.advance();
            return_aliases.push(self.expect_identifier("an alias to return")?);
        }

        if let Some(extra) = self.advance() {
            return Err(error(format!(
                "unexpected trailing token {extra:?} after the RETURN clause"
            )));
        }

        let bound_aliases: Vec<&str> = std::iter::once(source.alias.as_str())
            .chain(hop.iter().map(|(_, target)| target.alias.as_str()))
            .collect();
        for returned in &return_aliases {
            if !bound_aliases.contains(&returned.as_str()) {
                return Err(error(format!(
                    "RETURN references unbound alias `{returned}`; bound aliases are: {}",
                    bound_aliases.join(", ")
                )));
            }
        }
        if let Some(filter) = &filter
            && !bound_aliases.contains(&filter.alias.as_str())
        {
            return Err(error(format!(
                "WHERE references unbound alias `{}`; bound aliases are: {}",
                filter.alias,
                bound_aliases.join(", ")
            )));
        }

        Ok(MatchQuery {
            source,
            hop,
            filter,
            return_aliases,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{evaluate, parse};
    use crate::graph::GraphBuilder;
    use crate::inventory::{RepositoryWalker, WalkOptions};
    use std::path::Path;

    fn fixture_graph() -> Result<crate::graph::Graph, Box<dyn std::error::Error>> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/polyglot");
        let artifacts = RepositoryWalker::new(WalkOptions::default()).walk(&root)?;
        Ok(GraphBuilder.build(&root, &artifacts))
    }

    /// LIT-22.4.5 AC1/AC4: a single-node MATCH with a WHERE filter parses
    /// and evaluates to matching rows carrying structured graph refs.
    #[test]
    fn single_node_match_with_where_returns_matching_rows() -> Result<(), Box<dyn std::error::Error>>
    {
        let graph = fixture_graph()?;
        let query = parse(r#"MATCH (a:Symbol) WHERE a.name CONTAINS "RouteService" RETURN a"#)?;

        let rows = evaluate(&query, &graph);

        assert!(!rows.is_empty());
        assert!(rows.iter().all(|row| row.label == "Symbol"));
        assert!(
            rows.iter()
                .all(|row| row.name.to_lowercase().contains("routeservice"))
        );

        Ok(())
    }

    /// LIT-22.4.5 AC1/AC3/AC4: a node-edge-node MATCH with a relation kind
    /// returns rows for both bound aliases, each with structured graph refs.
    #[test]
    fn node_edge_node_match_returns_both_aliases() -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let query = parse("MATCH (a:Artifact)-[:Contains]->(b:Symbol) RETURN a, b")?;

        let rows = evaluate(&query, &graph);

        assert!(!rows.is_empty());
        assert!(
            rows.iter()
                .any(|row| row.alias == "a" && row.label == "Artifact")
        );
        assert!(
            rows.iter()
                .any(|row| row.alias == "b" && row.label == "Symbol")
        );

        Ok(())
    }

    /// LIT-22.4.5 AC1: a relation pattern with no kind (`-->`) matches any
    /// relation kind between the two node patterns.
    #[test]
    fn relation_with_no_kind_matches_any_relation() -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let query = parse("MATCH (a:Artifact)-->(b:Symbol) RETURN b")?;

        let rows = evaluate(&query, &graph);

        assert!(!rows.is_empty());

        Ok(())
    }

    /// LIT-22.4.5 AC2/AC4: syntactically invalid queries are rejected with
    /// an actionable message, not a panic or a silent empty result.
    #[test]
    fn invalid_syntax_returns_actionable_error() {
        assert!(parse("SELECT * FROM nodes").is_err());
        assert!(parse("MATCH (a:Symbol RETURN a").is_err());
        assert!(parse("MATCH (a:Symbol) WHERE a.name = notquoted RETURN a").is_err());
    }

    /// LIT-22.4.5 AC2/AC4: a RETURN referencing an alias that was never
    /// bound by MATCH is rejected before evaluation, not silently empty.
    #[test]
    fn unbound_return_alias_returns_actionable_error() -> Result<(), Box<dyn std::error::Error>> {
        match parse("MATCH (a:Symbol) RETURN b") {
            Ok(_) => Err("expected an unbound-alias error".into()),
            Err(error) => {
                assert!(error.message.contains("unbound alias"));
                Ok(())
            }
        }
    }

    /// LIT-22.4.5 AC4: an empty match (no node satisfies WHERE) evaluates
    /// to no rows rather than erroring.
    #[test]
    fn no_matching_nodes_evaluates_to_empty_rows() -> Result<(), Box<dyn std::error::Error>> {
        let graph = fixture_graph()?;
        let query =
            parse(r#"MATCH (a:Symbol) WHERE a.name = "definitely-not-a-real-symbol" RETURN a"#)?;

        let rows = evaluate(&query, &graph);

        assert!(rows.is_empty());

        Ok(())
    }
}
