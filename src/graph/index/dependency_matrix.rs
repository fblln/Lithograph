//! Deterministic module-level dependency matrix (Design Structure Matrix).

use super::KnowledgeIndex;
use crate::graph::{GraphNode, GraphNodeId, RelationKind};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// Deterministic module-level dependency matrix for CLI inspection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DependencyMatrix {
    /// Module ids in SCC-condensation topological order.
    pub modules: Vec<GraphNodeId>,
    /// Aggregated directed relation counts, indexed by `modules`.
    pub cells: Vec<Vec<usize>>,
    /// Strongly-connected module groups with at least two members.
    pub cycles: Vec<Vec<GraphNodeId>>,
}

impl<'a> KnowledgeIndex<'a> {
    /// Computes a deterministic Design Structure Matrix over real Module nodes.
    pub(crate) fn dependency_matrix(&self) -> DependencyMatrix {
        let modules: BTreeSet<GraphNodeId> = self
            .graph
            .nodes
            .iter()
            .filter(|node| matches!(node, GraphNode::Module(_)))
            .map(|node| node.id().clone())
            .collect();
        let mut owner = modules
            .iter()
            .map(|id| (id.clone(), id.clone()))
            .collect::<BTreeMap<_, _>>();
        for _ in 0..self.graph.nodes.len().max(1) {
            let mut changed = false;
            for relation in &self.graph.relations {
                match relation.kind {
                    RelationKind::BelongsToModule => {
                        if modules.contains(&relation.target)
                            && owner.get(&relation.source) != Some(&relation.target)
                        {
                            owner.insert(relation.source.clone(), relation.target.clone());
                            changed = true;
                        }
                    }
                    RelationKind::Contains => {
                        if let Some(parent) = owner.get(&relation.source).cloned()
                            && owner.get(&relation.target) != Some(&parent)
                        {
                            owner.insert(relation.target.clone(), parent);
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
            if !changed {
                break;
            }
        }
        let dependency_kinds = [
            RelationKind::Calls,
            RelationKind::Imports,
            RelationKind::TypeRefs,
            RelationKind::Implements,
            RelationKind::Inherits,
        ];
        let mut edges = BTreeMap::<GraphNodeId, BTreeSet<GraphNodeId>>::new();
        let mut counts = BTreeMap::<(GraphNodeId, GraphNodeId), usize>::new();
        for relation in &self.graph.relations {
            if !dependency_kinds.contains(&relation.kind) {
                continue;
            }
            let (Some(source), Some(target)) =
                (owner.get(&relation.source), owner.get(&relation.target))
            else {
                continue;
            };
            if source == target {
                continue;
            }
            edges
                .entry(source.clone())
                .or_default()
                .insert(target.clone());
            *counts.entry((source.clone(), target.clone())).or_default() += 1;
        }
        for module in &modules {
            edges.entry(module.clone()).or_default();
        }
        let sccs = tarjan_scc(&modules, &edges);
        let cycles: Vec<_> = sccs
            .iter()
            .filter(|group| group.len() > 1)
            .cloned()
            .collect();
        let ordered = condensation_order(&sccs, &edges);
        let mut cells = vec![vec![0; ordered.len()]; ordered.len()];
        let positions: BTreeMap<_, _> = ordered
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index))
            .collect();
        for ((source, target), count) in counts {
            cells[positions[&source]][positions[&target]] += count;
        }
        DependencyMatrix {
            modules: ordered,
            cells,
            cycles,
        }
    }
}

fn tarjan_scc(
    nodes: &BTreeSet<GraphNodeId>,
    edges: &BTreeMap<GraphNodeId, BTreeSet<GraphNodeId>>,
) -> Vec<Vec<GraphNodeId>> {
    struct State {
        next_index: usize,
        indexes: BTreeMap<GraphNodeId, usize>,
        lowlinks: BTreeMap<GraphNodeId, usize>,
        stack: Vec<GraphNodeId>,
        on_stack: BTreeSet<GraphNodeId>,
        groups: Vec<Vec<GraphNodeId>>,
    }
    fn visit(
        node: GraphNodeId,
        edges: &BTreeMap<GraphNodeId, BTreeSet<GraphNodeId>>,
        state: &mut State,
    ) {
        let index = state.next_index;
        state.next_index += 1;
        state.indexes.insert(node.clone(), index);
        state.lowlinks.insert(node.clone(), index);
        state.stack.push(node.clone());
        state.on_stack.insert(node.clone());
        for next in edges.get(&node).into_iter().flatten() {
            if !state.indexes.contains_key(next) {
                visit(next.clone(), edges, state);
                let low = state.lowlinks[&node].min(state.lowlinks[next]);
                state.lowlinks.insert(node.clone(), low);
            } else if state.on_stack.contains(next) {
                let low = state.lowlinks[&node].min(state.indexes[next]);
                state.lowlinks.insert(node.clone(), low);
            }
        }
        if state.lowlinks[&node] == state.indexes[&node] {
            let mut group = Vec::new();
            while let Some(member) = state.stack.pop() {
                state.on_stack.remove(&member);
                group.push(member.clone());
                if member == node {
                    break;
                }
            }
            group.sort();
            state.groups.push(group);
        }
    }
    let mut state = State {
        next_index: 0,
        indexes: BTreeMap::new(),
        lowlinks: BTreeMap::new(),
        stack: Vec::new(),
        on_stack: BTreeSet::new(),
        groups: Vec::new(),
    };
    for node in nodes {
        if !state.indexes.contains_key(node) {
            visit(node.clone(), edges, &mut state);
        }
    }
    state.groups.sort();
    state.groups
}

fn condensation_order(
    sccs: &[Vec<GraphNodeId>],
    edges: &BTreeMap<GraphNodeId, BTreeSet<GraphNodeId>>,
) -> Vec<GraphNodeId> {
    let mut component = BTreeMap::new();
    for (index, group) in sccs.iter().enumerate() {
        for node in group {
            component.insert(node.clone(), index);
        }
    }
    let mut outgoing = vec![BTreeSet::new(); sccs.len()];
    let mut indegree = vec![0usize; sccs.len()];
    for (source, targets) in edges {
        for target in targets {
            let (a, b) = (component[source], component[target]);
            if a != b && outgoing[a].insert(b) {
                indegree[b] += 1;
            }
        }
    }
    let mut ready = BTreeSet::<(GraphNodeId, usize)>::new();
    for (index, group) in sccs.iter().enumerate() {
        if indegree[index] == 0 {
            ready.insert((group[0].clone(), index));
        }
    }
    let mut ordered = Vec::new();
    while let Some((_, index)) = ready.pop_first() {
        ordered.extend(sccs[index].iter().cloned());
        for next in &outgoing[index] {
            indegree[*next] -= 1;
            if indegree[*next] == 0 {
                ready.insert((sccs[*next][0].clone(), *next));
            }
        }
    }
    ordered
}
