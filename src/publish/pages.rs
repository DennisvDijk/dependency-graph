//! Static site published as GitLab Pages when this tool runs inside GitLab.

use std::fs;
use std::path::Path;

use crate::error::Error;
use crate::models::GraphSnapshot;

/// Write a minimal static site.
///
/// # Errors
///
/// Returns an error when the directory cannot be written.
pub fn write_site(graph: &GraphSnapshot, output: &Path) -> Result<(), Error> {
    fs::create_dir_all(output)?;
    let mut body = String::from(
        "<!DOCTYPE html><meta charset=\"utf-8\"><title>depgraph</title><h1>depgraph</h1>",
    );
    body.push_str("<h2>Nodes</h2><ul>");
    for node in &graph.nodes {
        body.push_str(&format!(
            "<li>{} {}</li>",
            escape(node.node_type.as_str()),
            escape(&node.id)
        ));
    }
    body.push_str("</ul><h2>Edges</h2><ul>");
    for edge in &graph.edges {
        let env = edge.env.map(|env| env.as_str()).unwrap_or("");
        body.push_str(&format!(
            "<li>{} {} -&gt; {} {}</li>",
            escape(edge.edge_type.as_str()),
            escape(&edge.from_id),
            escape(&edge.to_id),
            escape(env)
        ));
    }
    body.push_str("</ul>\n");
    fs::write(output.join("index.html"), body)?;
    Ok(())
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
