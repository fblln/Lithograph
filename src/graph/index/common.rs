//! Shared node-inspection helpers used across the knowledge-index submodules.

use super::search::SearchResult;
use crate::graph::{GraphNode, GraphNodeId};
use std::collections::BTreeMap;

pub(crate) fn search_result(
    node: &GraphNode,
    degree: &BTreeMap<&GraphNodeId, (usize, usize)>,
) -> SearchResult {
    let (in_degree, out_degree) = degree.get(node.id()).copied().unwrap_or((0, 0));
    SearchResult {
        id: node.id().clone(),
        label: node_label(node).to_owned(),
        name: node_name(node),
        file_path: node_file_path(node),
        in_degree,
        out_degree,
    }
}

pub(crate) fn node_label(node: &GraphNode) -> &'static str {
    match node {
        GraphNode::Artifact(_) => "Artifact",
        GraphNode::Symbol(_) => "Symbol",
        GraphNode::Config(_) => "Config",
        GraphNode::Documentation(_) => "Documentation",
        GraphNode::Container(_) => "Container",
        GraphNode::Command(_) => "Command",
        GraphNode::EnvVar(_) => "EnvVar",
        GraphNode::Module(_) => "Module",
        GraphNode::Package(_) => "Package",
        GraphNode::Unresolved(_) => "Unresolved",
    }
}

pub(crate) fn node_name(node: &GraphNode) -> String {
    match node {
        GraphNode::Artifact(node) => node.path.clone(),
        GraphNode::Symbol(node) => node.qualified_name.clone(),
        GraphNode::Config(node) => node.name.clone(),
        GraphNode::Documentation(node) => node.title.clone(),
        GraphNode::Container(node) => node.reference.clone(),
        GraphNode::Command(node) => node.text.clone(),
        GraphNode::EnvVar(node) => node.name.clone(),
        GraphNode::Module(node) => node.path.clone(),
        GraphNode::Package(node) => node.name.clone(),
        GraphNode::Unresolved(node) => node.value.clone(),
    }
}

pub(crate) fn node_file_path(node: &GraphNode) -> Option<String> {
    match node {
        GraphNode::Artifact(node) => Some(node.path.clone()),
        GraphNode::Symbol(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Config(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Documentation(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Command(node) => Some(node.evidence.path.as_str().to_owned()),
        GraphNode::Module(node) => Some(node.evidence.path.as_str().to_owned()),
        _ => None,
    }
}

pub(crate) fn node_search_text(node: &GraphNode) -> String {
    let mut text = format!("{} {} {}", node.id(), node_label(node), node_name(node));
    if let Some(path) = node_file_path(node) {
        text.push(' ');
        text.push_str(&path);
    }
    text.to_lowercase()
}
