//! Read-only questions over a snapshot.
//!
//! Analysis is pure once it has a graph. F0-1 accepts the questions and
//! returns a stub error instead of inventing answers. `petgraph` arrives with
//! the first algorithm (ADR-002).

use std::fs;
use std::path::PathBuf;

use serde_json::json;

use crate::error::Error;
use crate::models::{Edge, Env, GraphSnapshot, Node};

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
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn library_impact(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Declared versus observed drift.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn drift(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
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
