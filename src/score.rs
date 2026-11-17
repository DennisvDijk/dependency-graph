//! Weighted paths and the team scorecard.
//!
//! `w(e) = calls(e) / sum of the client's outgoing calls`. A path score is the
//! product of `w * hardness` along the path. It ranks edges. It is not a probability.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::time::Instant;

use crate::analysis::algo;
use crate::models::{Criticality, Edge, Env, GraphSnapshot, NodeType};

/// Hardness factors. Defaults are assumptions, not measurements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hardness {
    /// Factor for a hard edge.
    pub hard: f64,
    /// Factor for an unknown edge.
    pub unknown: f64,
    /// Factor for a soft edge.
    pub soft: f64,
}

impl Default for Hardness {
    fn default() -> Self {
        Self {
            hard: 1.0,
            unknown: 0.5,
            soft: 0.2,
        }
    }
}

impl Hardness {
    fn factor(self, criticality: Criticality) -> f64 {
        match criticality {
            Criticality::Hard => self.hard,
            Criticality::Unknown => self.unknown,
            Criticality::Soft => self.soft,
        }
    }
}

/// Share of the client's outgoing calls carried by one edge.
#[must_use]
pub fn edge_weight(edge: &Edge, outgoing_calls: u64) -> f64 {
    if outgoing_calls == 0 {
        return 0.0;
    }
    calls_of(edge) as f64 / outgoing_calls as f64
}

/// Impact list with a path score for ranking.
///
/// # Errors
///
/// Returns an error when `component` is unknown.
pub fn ranked_impact(
    graph: &GraphSnapshot,
    component: &str,
    env: Option<Env>,
    hardness: Hardness,
) -> Result<Vec<ScoredVisit>, crate::error::Error> {
    let visits = algo::impact(graph, component, env)?;
    let weights = weights(graph, env);
    let target = visits
        .first()
        .map(|_| component_id(graph, component))
        .unwrap_or_else(|| component_id(graph, component));
    let scores = path_scores(graph, &target, env, hardness, &weights);
    let mut ranked: Vec<ScoredVisit> = visits
        .into_iter()
        .map(|visit| ScoredVisit {
            id: visit.id.clone(),
            depth: visit.depth,
            path_score: scores.get(&visit.id).copied().unwrap_or(0.0),
        })
        .collect();
    ranked.sort_by(|left, right| {
        right
            .path_score
            .partial_cmp(&left.path_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.depth.cmp(&right.depth))
            .then_with(|| left.id.cmp(&right.id))
    });
    Ok(ranked)
}

/// One impacted component and the best path score back to the target.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct ScoredVisit {
    /// Node id.
    pub id: String,
    /// Distance from the target.
    pub depth: usize,
    /// Product of weight times hardness. Not a probability.
    pub path_score: f64,
}

/// Sum of `weight * hardness` on active edges, for ranking.
#[must_use]
pub fn weight_score(
    graph: &GraphSnapshot,
    node_id: &str,
    env: Option<Env>,
    hardness: Hardness,
) -> f64 {
    let weights = weights(graph, env);
    graph
        .edges
        .iter()
        .filter(|edge| edge.active && env_ok(edge, env))
        .filter(|edge| edge.from_id == node_id || edge.to_id == node_id)
        .map(|edge| {
            weights.get(&edge.id).copied().unwrap_or(0.0) * hardness.factor(edge.criticality)
        })
        .sum()
}

/// Edge counts and a local query p95. The p95 is for this process, not production.
#[must_use]
pub fn measure(graph: &GraphSnapshot) -> GraphStats {
    let mut by_env = BTreeMap::new();
    let mut active = 0_u64;
    for edge in graph.edges.iter().filter(|edge| edge.active) {
        active += 1;
        let key = edge
            .env
            .map(|env| env.as_str().to_owned())
            .unwrap_or_else(|| "none".into());
        *by_env.entry(key).or_insert(0) += 1;
    }
    let target = graph
        .nodes
        .iter()
        .find(|node| node.node_type == NodeType::Component)
        .map(|node| node.id.as_str())
        .unwrap_or("component:none");
    let mut samples = Vec::new();
    for _ in 0..21 {
        let started = Instant::now();
        let _ = algo::impact(graph, target, None);
        samples.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    samples.sort_by(|left, right| left.partial_cmp(right).unwrap_or(std::cmp::Ordering::Equal));
    let index = (samples.len() as f64 * 0.95) as usize;
    let p95 = samples
        .get(index.min(samples.len() - 1))
        .copied()
        .unwrap_or(0.0);
    GraphStats {
        active_edges: active,
        active_edges_by_env: by_env,
        query_p95_ms: p95,
    }
}

/// Local measurement Dennis can paste into the graph-database ADR.
#[derive(Debug, Clone, PartialEq)]
pub struct GraphStats {
    /// Active edges in the snapshot.
    pub active_edges: u64,
    /// Active edges per environment, plus `none`.
    pub active_edges_by_env: BTreeMap<String, u64>,
    /// p95 of a local impact query, in milliseconds.
    pub query_p95_ms: f64,
}

/// One team's data-quality row. An aid, not a ranking for blame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamScore {
    /// `spec.owner`, or `(none)`.
    pub owner: String,
    /// Components attributed to the owner.
    pub components: u64,
    /// Components that also have `system`.
    pub with_system: u64,
    /// Components with at least one observed call.
    pub with_telemetry: u64,
}

/// Scorecard rows, owners sorted.
#[must_use]
pub fn scorecard(graph: &GraphSnapshot) -> Vec<TeamScore> {
    let mut owners: BTreeMap<String, TeamScore> = BTreeMap::new();
    let observed: BTreeSet<&str> = graph
        .edges
        .iter()
        .filter(|edge| edge.active && edge.edge_type == crate::models::EdgeType::ObservedCalls)
        .flat_map(|edge| [&edge.from_id, &edge.to_id])
        .map(String::as_str)
        .collect();
    for node in &graph.nodes {
        if node.node_type != NodeType::Component {
            continue;
        }
        let owner = node
            .attrs
            .get("owner")
            .and_then(|value| value.as_str())
            .filter(|value| !value.is_empty())
            .unwrap_or("(none)")
            .to_owned();
        let row = owners.entry(owner.clone()).or_insert(TeamScore {
            owner,
            components: 0,
            with_system: 0,
            with_telemetry: 0,
        });
        row.components += 1;
        if node
            .attrs
            .get("system")
            .and_then(|value| value.as_str())
            .is_some_and(|value| !value.is_empty())
        {
            row.with_system += 1;
        }
        if observed.contains(node.id.as_str()) {
            row.with_telemetry += 1;
        }
    }
    owners.into_values().collect()
}

fn calls_of(edge: &Edge) -> u64 {
    edge.attrs.calls_7d.or(edge.attrs.calls_total).unwrap_or(0)
}

fn weights(graph: &GraphSnapshot, env: Option<Env>) -> BTreeMap<String, f64> {
    let mut totals: BTreeMap<&str, u64> = BTreeMap::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.active && env_ok(edge, env))
    {
        *totals.entry(edge.from_id.as_str()).or_insert(0) += calls_of(edge);
    }
    let mut out = BTreeMap::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.active && env_ok(edge, env))
    {
        let total = totals.get(edge.from_id.as_str()).copied().unwrap_or(0);
        out.insert(edge.id.clone(), edge_weight(edge, total));
    }
    out
}

fn path_scores(
    graph: &GraphSnapshot,
    target: &str,
    env: Option<Env>,
    hardness: Hardness,
    weights: &BTreeMap<String, f64>,
) -> BTreeMap<String, f64> {
    let mut incoming: BTreeMap<&str, Vec<&Edge>> = BTreeMap::new();
    for edge in graph.edges.iter().filter(|edge| {
        edge.active && env_ok(edge, env) && crate::analysis::algo::dependency_edge(edge)
    }) {
        incoming.entry(edge.to_id.as_str()).or_default().push(edge);
    }
    // Best score per node over every path. A declared edge and an observed
    // edge can join the same pair; the stronger one must win, whatever the
    // edge order. Factors are at most 1, so a cycle never improves a score
    // and the relaxation ends.
    let mut best: BTreeMap<String, f64> = BTreeMap::new();
    let mut queue = VecDeque::from([(target.to_owned(), 1.0_f64, 0_usize)]);
    while let Some((node, score, depth)) = queue.pop_front() {
        if depth > graph.nodes.len() {
            continue;
        }
        if node != target && best.get(&node).is_some_and(|known| *known > score) {
            continue;
        }
        for edge in incoming.get(node.as_str()).into_iter().flatten() {
            if edge.from_id == target {
                continue;
            }
            let next = score
                * weights.get(&edge.id).copied().unwrap_or(0.0)
                * hardness.factor(edge.criticality);
            match best.get(&edge.from_id) {
                Some(known) if *known >= next => {}
                _ => {
                    best.insert(edge.from_id.clone(), next);
                    queue.push_back((edge.from_id.clone(), next, depth + 1));
                }
            }
        }
    }
    best
}

fn component_id(graph: &GraphSnapshot, name: &str) -> String {
    if graph.nodes.iter().any(|node| node.id == name) {
        name.to_owned()
    } else {
        format!("component:{name}")
    }
}

fn env_ok(edge: &Edge, env: Option<Env>) -> bool {
    match env {
        Some(env) => edge.env == Some(env) || edge.env.is_none(),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids;
    use crate::models::{EdgeAttrs, EdgeSource, EdgeStatus, EdgeType, Node, SourceKind, Timestamp};
    use std::collections::BTreeMap;

    fn edge(from: &str, to: &str, calls: u64, criticality: Criticality) -> Edge {
        let attrs = EdgeAttrs {
            calls_7d: Some(calls),
            ..EdgeAttrs::default()
        };
        Edge {
            id: ids::edge_id(EdgeType::ObservedCalls, from, to, Some(Env::Prod)),
            edge_type: EdgeType::ObservedCalls,
            from_id: from.into(),
            to_id: to.into(),
            env: Some(Env::Prod),
            sources: vec![EdgeSource {
                kind: SourceKind::Istio,
                first_seen: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                last_seen: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
                evidence_ref: None,
            }],
            status: EdgeStatus::ObservedOnly,
            criticality,
            criticality_source: Default::default(),
            active: true,
            attrs,
        }
    }

    #[test]
    fn weight_is_the_share_of_outgoing_calls_and_path_score_is_not_a_probability() {
        assert!((edge_weight(&edge("a", "b", 30, Criticality::Hard), 100) - 0.3).abs() < 1e-9);
        let graph = GraphSnapshot {
            schema_version: "0.1.0".into(),
            run_id: "t".into(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: vec![
                Node {
                    id: "component:a".into(),
                    node_type: NodeType::Component,
                    attrs: BTreeMap::new(),
                },
                Node {
                    id: "component:b".into(),
                    node_type: NodeType::Component,
                    attrs: BTreeMap::new(),
                },
                Node {
                    id: "component:c".into(),
                    node_type: NodeType::Component,
                    attrs: BTreeMap::new(),
                },
            ],
            edges: vec![
                edge("component:a", "component:b", 30, Criticality::Hard),
                edge("component:a", "component:c", 70, Criticality::Soft),
            ],
        };
        let ranked = ranked_impact(&graph, "b", Some(Env::Prod), Hardness::default()).unwrap();
        assert_eq!(ranked[0].id, "component:a");
        assert!((ranked[0].path_score - 0.3 * 1.0).abs() < 1e-9);
        let stats = measure(&graph);
        assert_eq!(stats.active_edges, 2);
        assert!(stats.query_p95_ms < 2000.0);
    }

    #[test]
    fn path_score_takes_the_strongest_edge_between_the_same_pair() {
        let node = |id: &str| Node {
            id: id.into(),
            node_type: NodeType::Component,
            attrs: BTreeMap::new(),
        };
        // A declared edge without calls comes first; the observed edge with
        // all of a's calls must still decide the score.
        let mut declared = edge("component:a", "component:b", 0, Criticality::Hard);
        declared.edge_type = EdgeType::DeclaredDependsOn;
        declared.attrs.calls_7d = None;
        declared.id = ids::edge_id(
            EdgeType::DeclaredDependsOn,
            "component:a",
            "component:b",
            Some(Env::Prod),
        );
        let observed = edge("component:a", "component:b", 100, Criticality::Hard);
        let graph = GraphSnapshot {
            schema_version: "0.1.0".into(),
            run_id: "t".into(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: vec![node("component:a"), node("component:b")],
            edges: vec![declared, observed],
        };
        let ranked = ranked_impact(&graph, "b", Some(Env::Prod), Hardness::default()).unwrap();
        assert_eq!(ranked[0].id, "component:a");
        assert!((ranked[0].path_score - 1.0).abs() < 1e-9);
    }

    #[test]
    fn scorecard_groups_by_owner_without_ranking_blame() {
        let mut attrs = BTreeMap::new();
        attrs.insert("owner".into(), serde_json::json!("platform"));
        attrs.insert("system".into(), serde_json::json!("payments"));
        let graph = GraphSnapshot {
            schema_version: "0.1.0".into(),
            run_id: "t".into(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: vec![Node {
                id: "component:a".into(),
                node_type: NodeType::Component,
                attrs,
            }],
            edges: vec![edge("component:a", "component:a", 1, Criticality::Unknown)],
        };
        let rows = scorecard(&graph);
        assert_eq!(rows[0].owner, "platform");
        assert_eq!(rows[0].with_system, 1);
        assert_eq!(rows[0].with_telemetry, 1);
    }
}
