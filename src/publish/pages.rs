//! Static site published as GitLab Pages.
//!
//! The page is one HTML file. Cytoscape.js is loaded from a CDN. There is no
//! backend: the snapshot and the alias table are embedded in the page.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use crate::error::Error;
use crate::models::GraphSnapshot;

/// Write the filterable graph page.
///
/// # Errors
///
/// Returns an error when the directory cannot be written.
pub fn write_site(
    graph: &GraphSnapshot,
    aliases: &BTreeMap<String, String>,
    output: &Path,
) -> Result<(), Error> {
    fs::create_dir_all(output)?;
    let graph_json = serde_json::to_string(graph).map_err(|err| Error::Store(err.to_string()))?;
    let alias_json = serde_json::to_string(aliases).map_err(|err| Error::Store(err.to_string()))?;
    let page = include_str!("page.html")
        .replace("/*GRAPH*/", &graph_json.replace('<', "\\u003c"))
        .replace("/*ALIASES*/", &alias_json.replace('<', "\\u003c"));
    fs::write(output.join("index.html"), page)?;
    let mut score = String::from(
        "<!DOCTYPE html><meta charset=\"utf-8\"><title>depgraph scorecard</title><h1>Data quality</h1><p>An improvement aid, not a ranking for blame.</p><table><tr><th>owner</th><th>components</th><th>with system</th><th>with telemetry</th></tr>",
    );
    for row in crate::score::scorecard(graph) {
        score.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row.owner, row.components, row.with_system, row.with_telemetry
        ));
    }
    score.push_str("</table>\n");
    fs::write(output.join("scorecard.html"), score)?;
    Ok(())
}
