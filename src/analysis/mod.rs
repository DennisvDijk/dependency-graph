//! Read-only questions over a snapshot.
//!
//! The algorithms live in [`algo`] and use petgraph. Printing is the only
//! side effect of the functions below.

pub mod algo;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::error::Error;
use crate::models::{Edge, EdgeType, Env, GraphSnapshot, Node, Timestamp};

/// How a read command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Aligned text.
    Table,
    /// One JSON document.
    Json,
    /// A Mermaid diagram.
    Mermaid,
}

/// Arguments shared by the read commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    /// Command name, used in the stub error.
    pub command: &'static str,
    /// Optional component, library, or other target.
    pub target: Option<String>,
    /// Second target, reserved for `diff`.
    pub against: Option<String>,
    /// Canonical environment filter.
    pub env: Option<Env>,
    /// Requested output format.
    pub format: OutputFormat,
    /// Snapshot file. `None` means the store default, once that exists.
    pub snapshot: Option<PathBuf>,
    /// Walk limit. `None` walks until the graph ends.
    pub depth: Option<usize>,
    /// Left run key for `diff`.
    pub from_run: Option<String>,
    /// Right run key for `diff`.
    pub to_run: Option<String>,
    /// Date for stored time travel.
    pub date: Option<String>,
    /// Archive directory.
    pub runs: Option<PathBuf>,
}

/// Show one node.
///
/// # Errors
///
/// Returns an error when the snapshot or the node is missing.
pub fn show(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let node = find_node(&graph, request.target.as_deref())?;
    if request.format == OutputFormat::Table {
        println!("{}", node.id);
        for (key, value) in &node.attrs {
            let text = match value {
                serde_json::Value::String(text) => text.clone(),
                serde_json::Value::Array(items) => items
                    .iter()
                    .map(|item| {
                        item.as_str()
                            .map_or_else(|| item.to_string(), str::to_owned)
                    })
                    .collect::<Vec<_>>()
                    .join(", "),
                other => other.to_string(),
            };
            println!("  {key}: {text}");
        }
        println!();
    }
    println!(
        "{}",
        render(
            request.format,
            &graph,
            Some(node),
            &related(&graph, node, true, request.env)
        )
    );
    Ok(())
}

/// Dependencies of a component.
///
/// # Errors
///
/// Returns an error when the snapshot or the node is missing.
pub fn deps(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let name = request
        .target
        .as_deref()
        .ok_or_else(|| Error::Validation("missing target".to_owned()))?;
    let visits = algo::dependencies(&graph, name, request.env, request.depth)?;
    println!("{}", render_visits(request.format, &visits));
    Ok(())
}

/// What calls or depends on a component.
///
/// # Errors
///
/// Returns an error when the snapshot or the node is missing.
pub fn consumers(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let name = request
        .target
        .as_deref()
        .ok_or_else(|| Error::Validation("missing target".to_owned()))?;
    let visits = algo::consumers(&graph, name, request.env, request.depth)?;
    println!("{}", render_visits(request.format, &visits));
    Ok(())
}

fn load_graph(request: &ReadRequest) -> Result<GraphSnapshot, Error> {
    let path = request
        .snapshot
        .as_ref()
        .ok_or_else(|| Error::Validation("pass --snapshot with a graph json file".to_owned()))?;
    read_snapshot(path)
}

fn read_snapshot(path: &Path) -> Result<GraphSnapshot, Error> {
    let text = fs::read_to_string(path)
        .map_err(|err| Error::Validation(format!("reading {}: {err}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|err| Error::Validation(crate::redact::redact(&err.to_string())))
}

fn component_set(graph: &GraphSnapshot, target: Option<&str>) -> Vec<String> {
    match target {
        Some(value) if !value.is_empty() => value
            .split(',')
            .map(|item| item.trim().to_owned())
            .filter(|item| !item.is_empty())
            .collect(),
        _ => graph
            .nodes
            .iter()
            .filter(|node| node.id.starts_with("component:"))
            .map(|node| node.id.clone())
            .collect(),
    }
}

fn render_scored(format: OutputFormat, visits: &[crate::score::ScoredVisit]) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "visits": visits.iter().map(|visit| json!({
                "id": visit.id,
                "depth": visit.depth,
                "path_score": visit.path_score,
            })).collect::<Vec<_>>()
        }))
        .unwrap_or_else(|_| "{}".into()),
        OutputFormat::Mermaid => {
            let mut lines = vec!["flowchart LR".to_owned()];
            for visit in visits {
                lines.push(format!("  {}", mermaid_id(&visit.id)));
            }
            lines.join("\n")
        }
        OutputFormat::Table => {
            let mut lines = vec!["depth\tpath_score\tid".to_owned()];
            for visit in visits {
                lines.push(format!(
                    "{}\t{:.3}\t{}",
                    visit.depth, visit.path_score, visit.id
                ));
            }
            lines.join("\n")
        }
    }
}

fn render_visits(format: OutputFormat, visits: &[algo::Visit]) -> String {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&json!({"visits": visits.iter().map(|visit| {
            let mut entry = json!({"id": visit.id, "depth": visit.depth});
            if visit.is_hub() {
                entry["hub"] = json!(true);
                entry["links"] = json!(visit.links);
            }
            entry
        }).collect::<Vec<_>>()}))
            .unwrap_or_else(|_| "{}".into())
        }
        OutputFormat::Mermaid => {
            let mut lines = vec!["flowchart LR".to_owned()];
            for visit in visits {
                lines.push(format!("  {}", mermaid_id(&visit.id)));
            }
            lines.join("\n")
        }
        OutputFormat::Table => {
            let mut lines = vec!["depth\tid".to_owned()];
            for visit in visits {
                let hub = if visit.is_hub() {
                    format!("\t(hub, {} links)", visit.links)
                } else {
                    String::new()
                };
                lines.push(format!("{}\t{}{hub}", visit.depth, visit.id));
            }
            lines.join("\n")
        }
    }
}

fn render_deploy(format: OutputFormat, order: &algo::DeployOrder) -> String {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&json!({"order": order.order, "cycles": order.cycles}))
                .unwrap_or_else(|_| "{}".into())
        }
        OutputFormat::Mermaid => format!("flowchart LR\n  %% order {}", order.order.join(" ")),
        OutputFormat::Table => {
            if order.cycles.is_empty() {
                order.order.join("\n")
            } else {
                format!(
                    "cycles\n{}",
                    order
                        .cycles
                        .iter()
                        .map(|cycle| cycle.join(" -> "))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            }
        }
    }
}

fn render_cycles(format: OutputFormat, cycles: &[Vec<String>]) -> String {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&json!({"cycles": cycles})).unwrap_or_else(|_| "{}".into())
        }
        OutputFormat::Mermaid => "flowchart LR".to_owned(),
        OutputFormat::Table => {
            if cycles.is_empty() {
                "no cycles".to_owned()
            } else {
                cycles
                    .iter()
                    .map(|cycle| cycle.join(" -> "))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
        }
    }
}

fn render_ids(format: OutputFormat, ids: &[String]) -> String {
    match format {
        OutputFormat::Json => {
            serde_json::to_string_pretty(&json!({"nodes": ids})).unwrap_or_else(|_| "{}".into())
        }
        OutputFormat::Mermaid => "flowchart LR".to_owned(),
        OutputFormat::Table => {
            if ids.is_empty() {
                "none".to_owned()
            } else {
                ids.join("\n")
            }
        }
    }
}

fn render_critical(
    format: OutputFormat,
    graph: &GraphSnapshot,
    env: Option<Env>,
    hardness: crate::score::Hardness,
    ranked: &[algo::CriticalNode],
) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&json!({"nodes": ranked.iter().map(|node| json!({"id": node.id, "in_degree": node.in_degree, "betweenness": node.betweenness, "articulation": node.articulation, "weight_score": crate::score::weight_score(graph, &node.id, env, hardness)})).collect::<Vec<_>>()})).unwrap_or_else(|_| "{}".into()),
        OutputFormat::Mermaid => "flowchart LR".to_owned(),
        OutputFormat::Table => {
            let mut lines = vec!["id\tin_degree\tbetweenness\tarticulation".to_owned()];
            for node in ranked {
                lines.push(format!(
                    "{}\t{}\t{}\t{}",
                    node.id, node.in_degree, node.betweenness, node.articulation
                ));
            }
            lines.join("\n")
        }
    }
}

fn render_diff(format: OutputFormat, diff: &algo::SnapshotDiff) -> String {
    match format {
        OutputFormat::Json => serde_json::to_string_pretty(&json!({
            "nodes_added": diff.nodes_added,
            "nodes_removed": diff.nodes_removed,
            "edges_added": diff.edges_added,
            "edges_removed": diff.edges_removed,
        }))
        .unwrap_or_else(|_| "{}".into()),
        OutputFormat::Mermaid => "flowchart LR".to_owned(),
        OutputFormat::Table => format!(
            "nodes added: {}\nnodes removed: {}\nedges added: {}\nedges removed: {}",
            diff.nodes_added.len(),
            diff.nodes_removed.len(),
            diff.edges_added.len(),
            diff.edges_removed.len()
        ),
    }
}

fn find_node<'a>(graph: &'a GraphSnapshot, target: Option<&str>) -> Result<&'a Node, Error> {
    let target = target.ok_or_else(|| Error::Validation("missing target".to_owned()))?;
    let component = format!("component:{target}");
    graph
        .nodes
        .iter()
        .find(|node| node.id == target || node.id == component)
        .ok_or_else(|| Error::Validation(format!("unknown node `{target}`")))
}

fn related<'a>(
    graph: &'a GraphSnapshot,
    node: &Node,
    outgoing: bool,
    env: Option<Env>,
) -> Vec<&'a Edge> {
    graph
        .edges
        .iter()
        .filter(|edge| {
            let end = if outgoing { &edge.from_id } else { &edge.to_id };
            end == &node.id
        })
        .filter(|edge| match env {
            Some(env) => edge.env == Some(env) || edge.env.is_none(),
            None => true,
        })
        .collect()
}

fn render(
    format: OutputFormat,
    graph: &GraphSnapshot,
    node: Option<&Node>,
    edges: &[&Edge],
) -> String {
    match format {
        OutputFormat::Json => {
            let value = json!({
                "node": node.map(|node| &node.id),
                "attrs": node.map(|node| &node.attrs),
                "edges": edges.iter().map(|edge| json!({
                    "type": edge.edge_type.as_str(),
                    "from": edge.from_id,
                    "to": edge.to_id,
                    "env": edge.env.map(|env| env.as_str()),
                    "confidence": crate::analysis::algo::confidence(edge),
                })).collect::<Vec<_>>(),
            });
            serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_owned())
        }
        OutputFormat::Mermaid => {
            let mut lines = vec!["flowchart LR".to_owned()];
            for edge in edges {
                lines.push(format!(
                    "  {} -->|{}| {}",
                    mermaid_id(&edge.from_id),
                    edge.edge_type.as_str(),
                    mermaid_id(&edge.to_id)
                ));
            }
            if edges.is_empty()
                && let Some(node) = node
            {
                lines.push(format!("  {}", mermaid_id(&node.id)));
            }
            let _ = graph;
            lines.join("\n")
        }
        OutputFormat::Table => {
            let mut lines = vec!["type\tfrom\tto\tenv".to_owned()];
            if let Some(node) = node
                && edges.is_empty()
            {
                lines.push(format!("node\t{}\t\t", node.id));
            }
            for edge in edges {
                let env = edge.env.map(|env| env.as_str()).unwrap_or("");
                lines.push(format!(
                    "{}\t{}\t{}\t{env}",
                    edge.edge_type.as_str(),
                    edge.from_id,
                    edge.to_id
                ));
            }
            lines.join("\n")
        }
    }
}

fn mermaid_id(value: &str) -> String {
    value.replace([':', '@', '/'], "_")
}

/// Transitive impact of a node.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn impact(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let name = request
        .target
        .as_deref()
        .ok_or_else(|| Error::Validation("missing target".to_owned()))?;
    let hardness = crate::config::load_project(Path::new("."))
        .ok()
        .and_then(|project| project.config.scoring.hardness().ok())
        .unwrap_or_default();
    let ranked = crate::score::ranked_impact(&graph, name, request.env, hardness)?;
    println!("{}", render_scored(request.format, &ranked));
    Ok(())
}

/// A deploy order that respects hard edges.
///
/// # Errors
///
/// Returns an error when the snapshot cannot be read.
pub fn deploy_order(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let set = component_set(&graph, request.target.as_deref());
    let order = algo::deploy_order(&graph, &set, request.env);
    println!("{}", render_deploy(request.format, &order));
    Ok(())
}

/// Cycles in the graph.
///
/// # Errors
///
/// Returns an error when the snapshot cannot be read.
pub fn cycles(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let cycles = algo::cycles(&graph, request.env);
    println!("{}", render_cycles(request.format, &cycles));
    Ok(())
}

/// Components affected by a library.
///
/// Libraries are disabled, so this does not read the graph.
///
/// # Errors
///
/// This function does not fail.
pub fn library_impact(request: &ReadRequest) -> Result<(), Error> {
    let _ = request;
    println!("libraries disabled");
    Ok(())
}

/// Declared versus observed drift, one markdown file per environment.
///
/// # Errors
///
/// Returns an error when the snapshot cannot be read or the directory cannot
/// be written.
pub fn drift(request: &ReadRequest, output: &Path) -> Result<(), Error> {
    let graph = load_graph(request)?;
    fs::create_dir_all(output)?;
    for env in [Env::Dev, Env::Acc, Env::Prod] {
        let path = output.join(format!("{env}.md"));
        fs::write(&path, drift_markdown(&graph, env))?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

pub(crate) fn drift_markdown(graph: &GraphSnapshot, env: Env) -> String {
    let declared: BTreeSet<(String, String)> = graph
        .edges
        .iter()
        .filter(|edge| edge.edge_type == EdgeType::DeclaredDependsOn)
        .map(edge_pair)
        .collect();
    let observed_here: BTreeSet<(String, String)> = graph
        .edges
        .iter()
        .filter(|edge| edge.edge_type == EdgeType::ObservedCalls && edge.env == Some(env))
        .map(edge_pair)
        .collect();
    let mut both = Vec::new();
    let mut observed_only = Vec::new();
    let mut declared_only = Vec::new();
    let mut candidates = Vec::new();
    let mut inactive = Vec::new();
    for edge in &graph.edges {
        let in_env = edge.env == Some(env) || edge.env.is_none();
        if !edge.active && in_env {
            inactive.push(edge);
            continue;
        }
        if !edge.active {
            continue;
        }
        match edge.edge_type {
            EdgeType::ObservedCalls if edge.env == Some(env) => {
                if declared.contains(&edge_pair(edge)) {
                    both.push(edge);
                } else {
                    observed_only.push(edge);
                }
            }
            // Structural links (IaC, network policy, firewall) are never in
            // mesh traffic. A declared pair that is observed is listed once,
            // by its observed edge, which carries the call count.
            EdgeType::DeclaredDependsOn if crate::normalize::merge::is_structural(&edge.to_id) => {}
            EdgeType::DeclaredDependsOn => {
                if !observed_here.contains(&edge_pair(edge)) {
                    declared_only.push(edge);
                }
            }
            EdgeType::CandidateCalls if in_env => candidates.push(edge),
            _ => {}
        }
    }
    let mut lines = vec![format!("# Drift {env}"), String::new()];
    push_section(&mut lines, "both", &both);
    push_section(&mut lines, "observed_only", &observed_only);
    push_section(&mut lines, "declared_only", &declared_only);
    push_section(&mut lines, "candidate", &candidates);
    push_section(&mut lines, "inactive", &inactive);
    lines.join("\n")
}

fn edge_pair(edge: &Edge) -> (String, String) {
    (edge.from_id.clone(), edge.to_id.clone())
}

fn push_section(lines: &mut Vec<String>, title: &str, edges: &[&Edge]) {
    let mut edges = edges.to_vec();
    edges.sort_by(|left, right| {
        (&left.from_id, &left.to_id, left.edge_type.as_str()).cmp(&(
            &right.from_id,
            &right.to_id,
            right.edge_type.as_str(),
        ))
    });
    lines.push(format!("## {title}"));
    lines.push(String::new());
    if edges.is_empty() {
        lines.push("_none_".to_owned());
    }
    for edge in edges {
        let calls = edge
            .attrs
            .calls_7d
            .map(|value| format!(" calls_7d={value}"))
            .or_else(|| {
                edge.attrs
                    .calls_total
                    .map(|value| format!(" calls_total={value}"))
            })
            .unwrap_or_default();
        lines.push(format!("- {} -> {}{calls}", edge.from_id, edge.to_id));
    }
    lines.push(String::new());
}

/// Difference between two snapshots.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn diff(request: &ReadRequest) -> Result<(), Error> {
    let (left, right) = if let (Some(from), Some(to)) = (&request.from_run, &request.to_run) {
        let runs = request
            .runs
            .clone()
            .unwrap_or_else(|| PathBuf::from("data/runs"));
        (
            crate::store::load_run(&runs, from)?,
            crate::store::load_run(&runs, to)?,
        )
    } else {
        let left = load_graph(request)?;
        let against = request.against.as_deref().ok_or_else(|| {
            Error::Validation(
                "diff needs a second snapshot or --from and --to. next: depgraph diff --from <run> --to <run>"
                    .into(),
            )
        })?;
        (left, read_snapshot(Path::new(against))?)
    };
    let diff = algo::diff(&left, &right);
    println!("{}", render_diff(request.format, &diff));
    Ok(())
}

/// The graph as of a point in time.
///
/// The target is an RFC 3339 timestamp.
///
/// # Errors
///
/// Returns an error when the snapshot or the timestamp cannot be read.
pub fn at(request: &ReadRequest) -> Result<(), Error> {
    if let Some(date) = &request.date {
        let stamp = Timestamp::parse(date).map_err(Error::Validation)?;
        let runs = request
            .runs
            .clone()
            .unwrap_or_else(|| PathBuf::from("data/runs"));
        let graph = crate::store::run_at(&runs, &stamp)?;
        println!(
            "run {} ({} nodes, {} edges)",
            graph.generated_at.as_str(),
            graph.nodes.len(),
            graph.edges.len()
        );
        println!(
            "{}",
            render(
                request.format,
                &graph,
                None,
                &graph.edges.iter().collect::<Vec<_>>()
            )
        );
        return Ok(());
    }
    let graph = load_graph(request)?;
    let raw = request
        .target
        .as_deref()
        .ok_or_else(|| Error::Validation("at needs an RFC 3339 timestamp or --date".to_owned()))?;
    let stamp = Timestamp::parse(raw).map_err(Error::Validation)?;
    let edges = algo::edges_at(&graph, &stamp);
    println!(
        "{}",
        render(
            request.format,
            &graph,
            None,
            &edges.iter().collect::<Vec<_>>()
        )
    );
    Ok(())
}

/// Nodes with no active edge.
///
/// # Errors
///
/// Returns an error when the snapshot cannot be read.
pub fn orphans(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let orphans = algo::orphans(&graph);
    println!("{}", render_ids(request.format, &orphans));
    Ok(())
}

/// Critical nodes for one environment.
///
/// # Errors
///
/// Returns an error when the snapshot cannot be read.
pub fn critical(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let ranked = algo::critical_nodes(&graph, request.env);
    let hardness = crate::config::load_project(Path::new("."))
        .ok()
        .and_then(|project| project.config.scoring.hardness().ok())
        .unwrap_or_default();
    println!(
        "{}",
        render_critical(request.format, &graph, request.env, hardness, &ranked)
    );
    Ok(())
}
