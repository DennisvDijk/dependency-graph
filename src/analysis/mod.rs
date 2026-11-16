//! Read-only questions over a snapshot.
//!
//! Analysis is pure once it has a graph. F0-1 accepts the questions and
//! returns a stub error instead of inventing answers. `petgraph` arrives with
//! the first algorithm (ADR-002).

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::json;

use crate::error::Error;
use crate::models::{Edge, EdgeType, Env, GraphSnapshot, Node};

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
}

/// Show one node.
///
/// # Errors
///
/// Returns an error when the snapshot or the node is missing.
pub fn show(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let node = find_node(&graph, request.target.as_deref())?;
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
    let node = find_node(&graph, request.target.as_deref())?;
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

/// What calls or depends on a component.
///
/// # Errors
///
/// Returns an error when the snapshot or the node is missing.
pub fn consumers(request: &ReadRequest) -> Result<(), Error> {
    let graph = load_graph(request)?;
    let node = find_node(&graph, request.target.as_deref())?;
    println!(
        "{}",
        render(
            request.format,
            &graph,
            Some(node),
            &related(&graph, node, false, request.env)
        )
    );
    Ok(())
}

fn load_graph(request: &ReadRequest) -> Result<GraphSnapshot, Error> {
    let path = request
        .snapshot
        .as_ref()
        .ok_or_else(|| Error::Validation("pass --snapshot with a graph json file".to_owned()))?;
    let text = fs::read_to_string(path)
        .map_err(|err| Error::Validation(format!("reading {}: {err}", path.display())))?;
    serde_json::from_str(&text)
        .map_err(|err| Error::Validation(crate::redact::redact(&err.to_string())))
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
                "edges": edges.iter().map(|edge| json!({
                    "type": edge.edge_type.as_str(),
                    "from": edge.from_id,
                    "to": edge.to_id,
                    "env": edge.env.map(|env| env.as_str()),
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
    reject(request)
}

/// A deploy order that respects hard edges.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn deploy_order(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Cycles in the graph.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn cycles(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
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

fn drift_markdown(graph: &GraphSnapshot, env: Env) -> String {
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
            EdgeType::DeclaredDependsOn => {
                if observed_here.contains(&edge_pair(edge)) {
                    both.push(edge);
                } else {
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

/// Edges marked hard.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn critical(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Nodes with no edges.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn orphans(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Difference between two snapshots.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn diff(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// The graph as of a point in time.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn at(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

fn reject(request: &ReadRequest) -> Result<(), Error> {
    let _ = (
        request.target.as_ref(),
        request.against.as_ref(),
        request.env,
        request.format,
        request.snapshot.as_ref(),
    );
    Err(Error::NotImplemented(request.command))
}
