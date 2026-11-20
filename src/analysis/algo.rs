//! Graph questions on petgraph.
//!
//! `A -> B` means A depends on B. Only active edges are walked. The caller
//! supplies the snapshot; nothing here reads the clock or the network.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};

use serde::Serialize;

use petgraph::Direction;
use petgraph::graph::{Graph, NodeIndex};
use petgraph::visit::EdgeRef;

use crate::error::Error;
use crate::models::{Criticality, Edge, EdgeStatus, EdgeType, Env, GraphSnapshot, Timestamp};

/// One step away from the start node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Visit {
    /// Node id.
    pub id: String,
    /// Distance from the start. The start itself is not included.
    pub depth: usize,
    /// How many other nodes this one is linked to by dependency edges, in
    /// either direction. It follows the `--env` filter, so a node can be a hub
    /// in all environments and not in one.
    pub links: usize,
}

/// One step of an impact path: `dependent` depends on `dependency`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hop {
    /// The side that is affected by a change on the other side.
    pub dependent: String,
    /// The side the change starts from.
    pub dependency: String,
    /// What kind of tie this is.
    pub edge_type: EdgeType,
    /// Whether the tie was declared, observed, both, or only inferred.
    pub status: EdgeStatus,
    /// Environment of the tie, when it has one.
    pub env: Option<Env>,
}

/// Why a node is affected: the chain of ties from the changed node to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ImpactPath {
    /// Hops in order, starting at the changed node.
    pub hops: Vec<Hop>,
    /// True when every route to this node uses at least one `candidate` tie.
    /// It is then potentially affected, not confirmed.
    pub inferred: bool,
}

/// For each node reached by [`impact`], the best chain of ties that reaches it.
///
/// "Best" is fewest `candidate` ties first, then fewest hops, so a node that
/// has a confirmed route is never explained by an inferred one. Ties are
/// compared as ids, so the result is the same on every run.
///
/// # Errors
///
/// Returns an error when `component` is not in the snapshot.
pub fn impact_paths(
    graph: &GraphSnapshot,
    component: &str,
    env: Option<Env>,
) -> Result<BTreeMap<String, ImpactPath>, Error> {
    use std::cmp::Reverse;
    use std::collections::BinaryHeap;
    let start = resolve_id(graph, component)
        .ok_or_else(|| Error::Validation(format!("unknown node `{component}`")))?;
    // dependency id -> ties to its dependents.
    let mut out: BTreeMap<&str, Vec<(&Edge, &str)>> = BTreeMap::new();
    for edge in &graph.edges {
        if !edge.active || !env_matches(edge, env) || !dependency_edge(edge) {
            continue;
        }
        let (dependent, dependency) = if edge.edge_type.points_to_dependent() {
            (edge.to_id.as_str(), edge.from_id.as_str())
        } else {
            (edge.from_id.as_str(), edge.to_id.as_str())
        };
        out.entry(dependency).or_default().push((edge, dependent));
    }
    let mut best: BTreeMap<String, (usize, usize, Vec<Hop>)> = BTreeMap::new();
    best.insert(start.clone(), (0, 0, Vec::new()));
    let mut heap = BinaryHeap::from([Reverse((0_usize, 0_usize, start.clone()))]);
    while let Some(Reverse((candidates, hops, node))) = heap.pop() {
        let Some((c, h, path)) = best.get(&node).cloned() else {
            continue;
        };
        if (c, h) != (candidates, hops) {
            continue;
        }
        for (edge, dependent) in out.get(node.as_str()).into_iter().flatten() {
            let cost = (c + usize::from(edge.status == EdgeStatus::Candidate), h + 1);
            let better = best
                .get(*dependent)
                .is_none_or(|(bc, bh, _)| cost < (*bc, *bh));
            if !better {
                continue;
            }
            let mut next = path.clone();
            next.push(Hop {
                dependent: (*dependent).to_owned(),
                dependency: node.clone(),
                edge_type: edge.edge_type,
                status: edge.status,
                env: edge.env,
            });
            best.insert((*dependent).to_owned(), (cost.0, cost.1, next));
            heap.push(Reverse((cost.0, cost.1, (*dependent).to_owned())));
        }
    }
    best.remove(&start);
    Ok(best
        .into_iter()
        .map(|(id, (candidates, _, hops))| {
            (
                id,
                ImpactPath {
                    inferred: candidates > 0,
                    hops,
                },
            )
        })
        .collect())
}

/// From this many links a node is a hub: a shared database, a platform
/// repository. Impact through a hub is real but says little about one change.
pub const HUB_LINKS: usize = 20;

impl Visit {
    /// Whether the node is connected to so much that impact through it is broad.
    #[must_use]
    pub fn is_hub(&self) -> bool {
        self.links >= HUB_LINKS
    }
}

/// A deploy order, or the cycles that blocked one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeployOrder {
    /// Dependencies first. Empty when `cycles` is not.
    pub order: Vec<String>,
    /// Strongly connected components that blocked the order.
    pub cycles: Vec<Vec<String>>,
}

/// One node in the critical ranking.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CriticalNode {
    /// Node id.
    pub id: String,
    /// Incoming active edges in the directed graph.
    pub in_degree: usize,
    /// Betweenness on the undirected projection.
    pub betweenness: f64,
    /// Whether the node is an articulation point.
    pub articulation: bool,
}

/// One edge in a diff, readable without the snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EdgeView {
    /// Source node id.
    pub from: String,
    /// Target node id.
    pub to: String,
    /// Edge type, as written in the snapshot.
    #[serde(rename = "type")]
    pub edge_type: String,
    /// Canonical environment, when the edge has one.
    pub env: Option<String>,
    /// Evidence status.
    pub status: String,
}

/// An edge present on both sides whose evidence changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EdgeChange {
    /// The edge, as it is on the right side.
    #[serde(flatten)]
    pub edge: EdgeView,
    /// What changed, such as `status candidate -> both`.
    pub changes: Vec<String>,
}

/// Nodes and edges that differ between two snapshots.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SnapshotDiff {
    /// Node ids only in `right`.
    pub nodes_added: Vec<String>,
    /// Node ids only in `left`.
    pub nodes_removed: Vec<String>,
    /// Edges only in `right`.
    pub edges_added: Vec<EdgeView>,
    /// Edges only in `left`.
    pub edges_removed: Vec<EdgeView>,
    /// Edges on both sides whose status, criticality or active flag changed.
    pub edges_changed: Vec<EdgeChange>,
}

/// What `c` depends on, walking outgoing edges up to `depth`.
///
/// `depth` `None` walks until the graph ends. Depth `0` returns nothing.
///
/// # Errors
///
/// Returns an error when `component` is not in the snapshot.
pub fn dependencies(
    graph: &GraphSnapshot,
    component: &str,
    env: Option<Env>,
    depth: Option<usize>,
) -> Result<Vec<Visit>, Error> {
    walk(graph, component, env, depth, true)
}

/// Who depends on `component`: the ancestors, walking incoming edges.
///
/// # Errors
///
/// Returns an error when `component` is not in the snapshot.
pub fn consumers(
    graph: &GraphSnapshot,
    component: &str,
    env: Option<Env>,
    depth: Option<usize>,
) -> Result<Vec<Visit>, Error> {
    walk(graph, component, env, depth, false)
}

/// Components transitively affected if `component` fails.
///
/// That is the ancestor set: every component that depends on it.
///
/// # Errors
///
/// Returns an error when `component` is not in the snapshot.
pub fn impact(
    graph: &GraphSnapshot,
    component: &str,
    env: Option<Env>,
) -> Result<Vec<Visit>, Error> {
    consumers(graph, component, env, None)
}

/// Order `set` so a dependency is deployed before its dependent.
///
/// Edges are active `declared_depends_on` and `observed_calls` whose
/// criticality is not `soft`. A cycle is returned in [`DeployOrder::cycles`]
/// and `order` stays empty.
pub fn deploy_order(graph: &GraphSnapshot, set: &[String], env: Option<Env>) -> DeployOrder {
    let wanted: BTreeSet<String> = set
        .iter()
        .map(|name| resolve_id(graph, name).unwrap_or_else(|| name.clone()))
        .collect();
    let indexed = index(graph, env, |edge| {
        deploy_edge(edge) && wanted.contains(&edge.from_id) && wanted.contains(&edge.to_id)
    });
    let mut keep = Vec::new();
    for id in &wanted {
        if let Some(index) = indexed.ids.get(id) {
            keep.push(*index);
        }
    }
    let cycles = cycles_from(&indexed, &keep);
    if !cycles.is_empty() {
        return DeployOrder {
            order: Vec::new(),
            cycles,
        };
    }
    let subgraph = induced(&indexed, &keep);
    let sorted = petgraph::algo::toposort(&subgraph, None).unwrap_or_default();
    // toposort emits a node before its successors. A depends on B, so B comes first.
    let mut order: Vec<String> = sorted
        .into_iter()
        .rev()
        .filter_map(|index| subgraph.node_weight(index).cloned())
        .collect();
    let mut rest: Vec<String> = wanted
        .into_iter()
        .filter(|id| !order.contains(id))
        .collect();
    rest.sort();
    order.extend(rest);
    DeployOrder {
        order,
        cycles: Vec::new(),
    }
}

/// Strongly connected components with more than one node, plus self-loops.
pub fn cycles(graph: &GraphSnapshot, env: Option<Env>) -> Vec<Vec<String>> {
    let indexed = index(graph, env, |_| true);
    let nodes: Vec<NodeIndex> = indexed.graph.node_indices().collect();
    cycles_from(&indexed, &nodes)
}

/// Library impact is disabled. There are no internal libraries.
#[must_use]
pub fn library_impact_disabled() -> &'static str {
    "libraries disabled"
}

/// Highest in-degree, betweenness, and articulation points. At most 20.
pub fn critical_nodes(graph: &GraphSnapshot, env: Option<Env>) -> Vec<CriticalNode> {
    let indexed = index(graph, env, |_| true);
    let between = betweenness(&indexed.undirected);
    let cuts = petgraph::algo::articulation_points::articulation_points(&indexed.undirected);
    let mut ranked = Vec::new();
    for index in indexed.graph.node_indices() {
        let id = indexed.graph[index].clone();
        let in_degree = indexed
            .graph
            .edges_directed(index, Direction::Incoming)
            .count();
        let betweenness = between.get(&index).copied().unwrap_or(0.0);
        let articulation = cuts.contains(&index);
        if in_degree == 0 && betweenness == 0.0 && !articulation {
            continue;
        }
        ranked.push(CriticalNode {
            id,
            in_degree,
            betweenness,
            articulation,
        });
    }
    ranked.sort_by(|left, right| {
        right
            .betweenness
            .partial_cmp(&left.betweenness)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| right.in_degree.cmp(&left.in_degree))
            .then_with(|| right.articulation.cmp(&left.articulation))
            .then_with(|| left.id.cmp(&right.id))
    });
    ranked.truncate(20);
    ranked
}

/// Nodes with no active edge.
pub fn orphans(graph: &GraphSnapshot) -> Vec<String> {
    let indexed = index(graph, None, |_| true);
    let mut connected = BTreeSet::new();
    for edge in indexed.graph.edge_indices() {
        if let Some((from, to)) = indexed.graph.edge_endpoints(edge) {
            connected.insert(indexed.graph[from].clone());
            connected.insert(indexed.graph[to].clone());
        }
    }
    graph
        .nodes
        .iter()
        .map(|node| node.id.clone())
        .filter(|id| !connected.contains(id))
        .collect()
}

/// Compare two snapshots by node id and edge id.
#[must_use]
pub fn diff(left: &GraphSnapshot, right: &GraphSnapshot) -> SnapshotDiff {
    let left_nodes: BTreeSet<_> = left.nodes.iter().map(|node| node.id.as_str()).collect();
    let right_nodes: BTreeSet<_> = right.nodes.iter().map(|node| node.id.as_str()).collect();
    let left_edges: BTreeMap<&str, &Edge> = left.edges.iter().map(|e| (e.id.as_str(), e)).collect();
    let right_edges: BTreeMap<&str, &Edge> =
        right.edges.iter().map(|e| (e.id.as_str(), e)).collect();
    let mut edges_added: Vec<EdgeView> = right_edges
        .iter()
        .filter(|(id, _)| !left_edges.contains_key(*id))
        .map(|(_, e)| edge_view(e))
        .collect();
    let mut edges_removed: Vec<EdgeView> = left_edges
        .iter()
        .filter(|(id, _)| !right_edges.contains_key(*id))
        .map(|(_, e)| edge_view(e))
        .collect();
    let mut edges_changed = Vec::new();
    for (id, new) in &right_edges {
        let Some(old) = left_edges.get(id) else {
            continue;
        };
        let mut changes = Vec::new();
        if old.status != new.status {
            changes.push(format!(
                "status {} -> {}",
                old.status.as_str(),
                new.status.as_str()
            ));
        }
        if old.criticality != new.criticality {
            changes.push(format!(
                "criticality {} -> {}",
                format!("{:?}", old.criticality).to_lowercase(),
                format!("{:?}", new.criticality).to_lowercase()
            ));
        }
        if old.active != new.active {
            changes.push(format!("active {} -> {}", old.active, new.active));
        }
        if !changes.is_empty() {
            edges_changed.push(EdgeChange {
                edge: edge_view(new),
                changes,
            });
        }
    }
    let key = |e: &EdgeView| {
        (
            e.from.clone(),
            e.to.clone(),
            e.edge_type.clone(),
            e.env.clone(),
        )
    };
    edges_added.sort_by_key(key);
    edges_removed.sort_by_key(key);
    edges_changed.sort_by_key(|c| key(&c.edge));
    SnapshotDiff {
        nodes_added: difference(&right_nodes, &left_nodes),
        nodes_removed: difference(&left_nodes, &right_nodes),
        edges_added,
        edges_removed,
        edges_changed,
    }
}

fn edge_view(edge: &Edge) -> EdgeView {
    EdgeView {
        from: edge.from_id.clone(),
        to: edge.to_id.clone(),
        edge_type: edge.edge_type.as_str().to_owned(),
        env: edge.env.map(|env| env.as_str().to_owned()),
        status: edge.status.as_str().to_owned(),
    }
}

/// Confidence in `0.0..=1.0` from status and whether calls were observed.
///
/// Inactive edges score `0`. A declared-and-observed edge with calls scores `1`.
#[must_use]
pub fn confidence(edge: &crate::models::Edge) -> f64 {
    if !edge.active {
        return 0.0;
    }
    let base = match edge.status {
        crate::models::EdgeStatus::Both => 0.9,
        crate::models::EdgeStatus::ObservedOnly => 0.6,
        crate::models::EdgeStatus::DeclaredOnly => 0.4,
        crate::models::EdgeStatus::Candidate => 0.2,
        crate::models::EdgeStatus::NotApplicable => 0.5,
    };
    if edge.attrs.calls_7d.unwrap_or(0) > 0 || edge.attrs.calls_total.unwrap_or(0) > 0 {
        f64::min(base + 0.1, 1.0)
    } else {
        base
    }
}

/// Edges whose evidence covers `at`.
#[must_use]
pub fn edges_at(graph: &GraphSnapshot, at: &Timestamp) -> Vec<Edge> {
    graph
        .edges
        .iter()
        .filter(|edge| edge_covers(edge, at))
        .cloned()
        .collect()
}

fn edge_covers(edge: &Edge, at: &Timestamp) -> bool {
    edge.sources.iter().any(|source| {
        source.first_seen.as_str() <= at.as_str() && at.as_str() <= source.last_seen.as_str()
    })
}

fn walk(
    graph: &GraphSnapshot,
    component: &str,
    env: Option<Env>,
    depth: Option<usize>,
    outgoing: bool,
) -> Result<Vec<Visit>, Error> {
    let start_id = resolve_id(graph, component)
        .ok_or_else(|| Error::Validation(format!("unknown node `{component}`")))?;
    let indexed = index(graph, env, dependency_edge);
    let start = *indexed
        .ids
        .get(&start_id)
        .ok_or_else(|| Error::Validation(format!("unknown node `{component}`")))?;
    if depth == Some(0) {
        return Ok(Vec::new());
    }
    let direction = if outgoing {
        Direction::Outgoing
    } else {
        Direction::Incoming
    };
    let mut seen = BTreeSet::from([start]);
    let mut queue = VecDeque::from([(start, 0_usize)]);
    let mut visits = Vec::new();
    while let Some((node, distance)) = queue.pop_front() {
        let next = distance + 1;
        if depth.is_some_and(|limit| next > limit) {
            continue;
        }
        for neighbor in indexed.graph.neighbors_directed(node, direction) {
            if !seen.insert(neighbor) {
                continue;
            }
            visits.push(Visit {
                id: indexed.graph[neighbor].clone(),
                depth: next,
                links: indexed.links(neighbor),
            });
            queue.push_back((neighbor, next));
        }
    }
    visits.sort_by(|left, right| left.depth.cmp(&right.depth).then(left.id.cmp(&right.id)));
    Ok(visits)
}

impl Indexed {
    /// Distinct other nodes this node is linked to, in either direction.
    /// Parallel edges and self-loops count once and not at all.
    fn links(&self, node: NodeIndex) -> usize {
        self.undirected
            .neighbors(node)
            .filter(|other| *other != node)
            .collect::<BTreeSet<_>>()
            .len()
    }
}

struct Indexed {
    graph: Graph<String, (), petgraph::Directed>,
    undirected: Graph<String, (), petgraph::Undirected>,
    ids: BTreeMap<String, NodeIndex>,
}

fn index(graph: &GraphSnapshot, env: Option<Env>, keep: impl Fn(&Edge) -> bool) -> Indexed {
    let mut directed = Graph::new();
    let mut undirected = Graph::new_undirected();
    let mut ids = BTreeMap::new();
    for node in &graph.nodes {
        let index = directed.add_node(node.id.clone());
        undirected.add_node(node.id.clone());
        ids.insert(node.id.clone(), index);
    }
    for edge in &graph.edges {
        if !edge.active || !env_matches(edge, env) || !keep(edge) {
            continue;
        }
        let Some(&from) = ids.get(&edge.from_id) else {
            continue;
        };
        let Some(&to) = ids.get(&edge.to_id) else {
            continue;
        };
        // A publisher is not affected by the topic it writes to; the topic and
        // its consumers are affected by the publisher. Point the edge the way a
        // dependency points: from the one that depends to the one depended on.
        if edge.edge_type.points_to_dependent() {
            directed.add_edge(to, from, ());
        } else {
            directed.add_edge(from, to, ());
        }
        if from != to {
            undirected.add_edge(from, to, ());
        }
    }
    Indexed {
        graph: directed,
        undirected,
        ids,
    }
}

fn env_matches(edge: &Edge, env: Option<Env>) -> bool {
    match env {
        Some(env) => edge.env == Some(env) || edge.env.is_none(),
        None => true,
    }
}

/// Network and placement evidence is not a service dependency.
pub(crate) fn dependency_edge(edge: &Edge) -> bool {
    edge.edge_type.affects_impact()
}

fn deploy_edge(edge: &Edge) -> bool {
    if edge.criticality == Criticality::Soft {
        return false;
    }
    matches!(
        edge.edge_type,
        EdgeType::DeclaredDependsOn | EdgeType::ObservedCalls
    )
}

fn resolve_id(graph: &GraphSnapshot, name: &str) -> Option<String> {
    if graph.nodes.iter().any(|node| node.id == name) {
        return Some(name.to_owned());
    }
    let component = format!("component:{name}");
    graph
        .nodes
        .iter()
        .any(|node| node.id == component)
        .then_some(component)
}

fn induced(indexed: &Indexed, nodes: &[NodeIndex]) -> Graph<String, (), petgraph::Directed> {
    let mut subgraph = Graph::new();
    let mut map = HashMap::new();
    for index in nodes {
        let added = subgraph.add_node(indexed.graph[*index].clone());
        map.insert(*index, added);
    }
    for edge in indexed.graph.edge_references() {
        if let (Some(&from), Some(&to)) = (map.get(&edge.source()), map.get(&edge.target())) {
            subgraph.add_edge(from, to, ());
        }
    }
    subgraph
}

fn cycles_from(indexed: &Indexed, nodes: &[NodeIndex]) -> Vec<Vec<String>> {
    let subgraph = induced(indexed, nodes);
    let mut cycles = Vec::new();
    for component in petgraph::algo::tarjan_scc(&subgraph) {
        if component.len() > 1 {
            let mut names: Vec<String> = component
                .into_iter()
                .filter_map(|index| subgraph.node_weight(index).cloned())
                .collect();
            names.sort();
            cycles.push(names);
        }
    }
    for index in subgraph.node_indices() {
        if subgraph.contains_edge(index, index)
            && let Some(name) = subgraph.node_weight(index)
        {
            cycles.push(vec![name.clone()]);
        }
    }
    cycles.sort();
    cycles
}

fn betweenness(graph: &Graph<String, (), petgraph::Undirected>) -> HashMap<NodeIndex, f64> {
    let nodes: Vec<NodeIndex> = graph.node_indices().collect();
    let mut score: HashMap<NodeIndex, f64> = HashMap::new();
    for start in &nodes {
        let mut stack = Vec::new();
        let mut predecessors: HashMap<NodeIndex, Vec<NodeIndex>> = HashMap::new();
        let mut sigma: HashMap<NodeIndex, f64> = HashMap::new();
        let mut distance: HashMap<NodeIndex, i32> = HashMap::new();
        for node in &nodes {
            sigma.insert(*node, 0.0);
            distance.insert(*node, -1);
        }
        sigma.insert(*start, 1.0);
        distance.insert(*start, 0);
        let mut queue = VecDeque::from([*start]);
        while let Some(node) = queue.pop_front() {
            stack.push(node);
            let next = distance[&node] + 1;
            for neighbor in graph.neighbors(node) {
                if distance[&neighbor] < 0 {
                    distance.insert(neighbor, next);
                    queue.push_back(neighbor);
                }
                if distance[&neighbor] == next {
                    *sigma.entry(neighbor).or_insert(0.0) += sigma[&node];
                    predecessors.entry(neighbor).or_default().push(node);
                }
            }
        }
        let mut delta: HashMap<NodeIndex, f64> = HashMap::new();
        while let Some(node) = stack.pop() {
            let node_delta = delta.get(&node).copied().unwrap_or(0.0);
            if let Some(preds) = predecessors.get(&node) {
                for predecessor in preds {
                    let share = (sigma[predecessor] / sigma[&node]) * (1.0 + node_delta);
                    *delta.entry(*predecessor).or_insert(0.0) += share;
                }
            }
            if node != *start {
                *score.entry(node).or_insert(0.0) += node_delta;
            }
        }
    }
    for value in score.values_mut() {
        *value /= 2.0;
    }
    score
}

fn difference(left: &BTreeSet<&str>, right: &BTreeSet<&str>) -> Vec<String> {
    left.difference(right)
        .map(|item| (*item).to_owned())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids;
    use crate::models::{EdgeAttrs, EdgeSource, EdgeStatus, Node, NodeType, SourceKind};

    fn node(id: &str) -> Node {
        Node {
            id: id.to_owned(),
            node_type: NodeType::Component,
            attrs: BTreeMap::new(),
        }
    }

    fn edge(
        from: &str,
        to: &str,
        kind: EdgeType,
        env: Option<Env>,
        soft: bool,
        active: bool,
    ) -> Edge {
        Edge {
            id: ids::edge_id(kind, from, to, env),
            edge_type: kind,
            from_id: from.to_owned(),
            to_id: to.to_owned(),
            env,
            sources: vec![EdgeSource {
                kind: SourceKind::Static,
                first_seen: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_seen: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
                evidence_ref: None,
            }],
            status: EdgeStatus::DeclaredOnly,
            criticality: if soft {
                Criticality::Soft
            } else {
                Criticality::Hard
            },
            criticality_source: Default::default(),
            active,
            attrs: EdgeAttrs::default(),
        }
    }

    fn fixture() -> GraphSnapshot {
        GraphSnapshot {
            schema_version: "0.1.0".to_owned(),
            run_id: "test".to_owned(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: vec![
                node("component:a"),
                node("component:b"),
                node("component:c"),
                node("component:d"),
                node("component:e"),
                node("component:f"),
            ],
            edges: vec![
                edge(
                    "component:a",
                    "component:b",
                    EdgeType::DeclaredDependsOn,
                    Some(Env::Prod),
                    false,
                    true,
                ),
                edge(
                    "component:b",
                    "component:c",
                    EdgeType::DeclaredDependsOn,
                    Some(Env::Prod),
                    false,
                    true,
                ),
                edge(
                    "component:c",
                    "component:a",
                    EdgeType::DeclaredDependsOn,
                    Some(Env::Prod),
                    false,
                    true,
                ),
                edge(
                    "component:e",
                    "component:f",
                    EdgeType::ObservedCalls,
                    Some(Env::Prod),
                    true,
                    true,
                ),
                edge(
                    "component:a",
                    "component:d",
                    EdgeType::DeclaredDependsOn,
                    Some(Env::Prod),
                    false,
                    false,
                ),
            ],
        }
    }

    #[test]
    fn a_topic_carries_impact_from_publisher_to_consumer_and_not_back() {
        let mut graph = fixture();
        graph.nodes.push(node("component:pub"));
        graph.nodes.push(node("component:sub"));
        graph.nodes.push(node("resource:topic:events"));
        graph.edges.push(edge(
            "component:pub",
            "resource:topic:events",
            EdgeType::PublishesTo,
            None,
            false,
            true,
        ));
        graph.edges.push(edge(
            "component:sub",
            "resource:topic:events",
            EdgeType::ConsumesFrom,
            None,
            false,
            true,
        ));
        let from_publisher: Vec<String> = impact(&graph, "pub", None)
            .unwrap()
            .into_iter()
            .map(|visit| visit.id)
            .collect();
        assert_eq!(from_publisher, ["resource:topic:events", "component:sub"]);
        assert!(impact(&graph, "sub", None).unwrap().is_empty());
        let needs: Vec<String> = dependencies(&graph, "sub", None, None)
            .unwrap()
            .into_iter()
            .map(|visit| visit.id)
            .collect();
        assert_eq!(needs, ["resource:topic:events", "component:pub"]);
    }

    #[test]
    fn dependencies_and_consumers_respect_depth_and_active_edges() {
        let graph = fixture();
        let direct = dependencies(&graph, "a", Some(Env::Prod), Some(1)).unwrap();
        assert_eq!(
            direct,
            vec![Visit {
                id: "component:b".into(),
                depth: 1,
                links: 2
            }]
        );
        let deep = dependencies(&graph, "a", Some(Env::Prod), None).unwrap();
        assert!(
            deep.iter()
                .any(|visit| visit.id == "component:c" && visit.depth == 2)
        );
        let consumers = consumers(&graph, "b", Some(Env::Prod), Some(1)).unwrap();
        assert_eq!(consumers[0].id, "component:a");
        assert!(dependencies(&graph, "missing", None, None).is_err());
        assert!(
            dependencies(&graph, "a", Some(Env::Prod), Some(0))
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn impact_is_the_ancestor_set() {
        let graph = fixture();
        let affected = impact(&graph, "c", Some(Env::Prod)).unwrap();
        let ids: Vec<_> = affected.into_iter().map(|visit| visit.id).collect();
        assert!(ids.contains(&"component:a".to_owned()));
        assert!(ids.contains(&"component:b".to_owned()));
    }

    #[test]
    fn impact_on_a_cycle_ends_and_lists_each_node_once_without_the_start() {
        let graph = fixture();
        assert!(!cycles(&graph, Some(Env::Prod)).is_empty());
        let ids: Vec<_> = impact(&graph, "c", Some(Env::Prod))
            .unwrap()
            .into_iter()
            .map(|visit| visit.id)
            .collect();
        let unique: std::collections::BTreeSet<_> = ids.iter().collect();
        assert_eq!(ids.len(), unique.len(), "{ids:?}");
        assert!(!ids.contains(&"component:c".to_owned()), "{ids:?}");
        let paths = impact_paths(&graph, "c", Some(Env::Prod)).unwrap();
        assert!(!paths.contains_key("component:c"));
        assert!(paths.contains_key("component:a") && paths.contains_key("component:b"));
    }

    #[test]
    fn deploy_order_reports_the_cycle_and_ignores_soft_edges() {
        let graph = fixture();
        let blocked = deploy_order(
            &graph,
            &["a".into(), "b".into(), "c".into()],
            Some(Env::Prod),
        );
        assert!(blocked.order.is_empty());
        assert!(blocked.cycles.iter().any(|cycle| cycle.len() == 3));
        let soft = deploy_order(&graph, &["e".into(), "f".into()], Some(Env::Prod));
        assert!(soft.cycles.is_empty());
        assert_eq!(soft.order.len(), 2);
    }

    #[test]
    fn deploy_order_puts_the_dependency_first_when_there_is_no_cycle() {
        let mut graph = fixture();
        graph.edges.retain(|edge| edge.from_id != "component:c");
        let order = deploy_order(&graph, &["a".into(), "b".into()], Some(Env::Prod));
        assert!(order.cycles.is_empty());
        assert_eq!(
            order.order,
            vec!["component:b".to_owned(), "component:a".to_owned()]
        );
    }

    #[test]
    fn cycles_empty_graph_and_orphans() {
        let empty = GraphSnapshot {
            schema_version: "0.1.0".to_owned(),
            run_id: "test".to_owned(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        assert!(cycles(&empty, None).is_empty());
        assert!(orphans(&empty).is_empty());
        assert!(critical_nodes(&empty, None).is_empty());
        let graph = fixture();
        assert!(
            cycles(&graph, Some(Env::Prod))
                .iter()
                .any(|cycle| cycle.len() == 3)
        );
        assert_eq!(orphans(&graph), vec!["component:d".to_owned()]);
        let mut lonely = graph.clone();
        lonely.edges.clear();
        assert!(orphans(&lonely).contains(&"component:d".to_owned()));
    }

    #[test]
    fn critical_nodes_rank_the_middle_of_a_line() {
        let graph = GraphSnapshot {
            schema_version: "0.1.0".to_owned(),
            run_id: "test".to_owned(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: vec![
                node("component:a"),
                node("component:b"),
                node("component:c"),
            ],
            edges: vec![
                edge(
                    "component:a",
                    "component:b",
                    EdgeType::DeclaredDependsOn,
                    None,
                    false,
                    true,
                ),
                edge(
                    "component:b",
                    "component:c",
                    EdgeType::DeclaredDependsOn,
                    None,
                    false,
                    true,
                ),
            ],
        };
        let ranked = critical_nodes(&graph, None);
        assert_eq!(ranked[0].id, "component:b");
        assert!(ranked[0].articulation);
        assert!(ranked[0].betweenness > 0.0);
        assert!(ranked.len() <= 20);
    }

    #[test]
    fn diff_and_at_filter_by_id_and_time() {
        let left = fixture();
        let mut right = fixture();
        right.nodes.push(node("component:z"));
        let diff = diff(&left, &right);
        assert_eq!(diff.nodes_added, vec!["component:z".to_owned()]);
        assert!(diff.nodes_removed.is_empty());
        let at = Timestamp::parse("2026-01-10T00:00:00Z").unwrap();
        assert!(!edges_at(&left, &at).is_empty());
        let before = Timestamp::parse("2025-01-01T00:00:00Z").unwrap();
        assert!(edges_at(&left, &before).is_empty());
        assert_eq!(library_impact_disabled(), "libraries disabled");
    }

    #[test]
    fn diff_names_added_removed_and_changed_edges() {
        let left = fixture();
        let mut right = fixture();
        let removed = right.edges.remove(0);
        right.edges.push(edge(
            "component:a",
            "component:f",
            EdgeType::DeclaredDependsOn,
            Some(Env::Prod),
            false,
            true,
        ));
        right.edges[0].status = EdgeStatus::Both;
        let changed = right.edges[0].clone();
        let diff = diff(&left, &right);
        assert_eq!(diff.edges_removed.len(), 1);
        assert_eq!(diff.edges_removed[0].from, removed.from_id);
        assert_eq!(diff.edges_added.len(), 1);
        assert_eq!(diff.edges_added[0].to, "component:f");
        assert_eq!(diff.edges_added[0].env.as_deref(), Some("prod"));
        assert_eq!(diff.edges_changed.len(), 1);
        assert_eq!(diff.edges_changed[0].edge.from, changed.from_id);
        assert_eq!(
            diff.edges_changed[0].changes,
            vec!["status declared_only -> both".to_owned()]
        );
    }

    #[test]
    fn links_count_distinct_neighbours_and_flag_hubs() {
        let visit = |links| Visit {
            id: "resource:database:shared".into(),
            depth: 1,
            links,
        };
        assert!(!visit(HUB_LINKS - 1).is_hub());
        assert!(visit(HUB_LINKS).is_hub());
        let graph = fixture();
        let indexed = index(&graph, Some(Env::Prod), dependency_edge);
        let b = indexed.ids["component:b"];
        assert_eq!(indexed.links(b), 2);
    }
}
