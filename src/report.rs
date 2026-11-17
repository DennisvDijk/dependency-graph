//! A short summary of one discovery run that is safe to share: counts and the
//! kinds of names that could not be resolved, without repository, host or
//! service names unless the person asks for them.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::models::GraphSnapshot;
use crate::normalize::merge::Unresolved;

/// Most unresolved names listed with `include_names`.
const NAME_LIMIT: usize = 40;

/// What an unresolved name looks like, without saying what it is.
fn shape(name: &str) -> &'static str {
    if name.contains('$') || name.contains("var.") || name.contains("local.") {
        "built from a variable or interpolation"
    } else if name.contains("module.") {
        "comes from another module"
    } else if name.contains('.') || name.contains(':') || name.contains('/') {
        "host name, path or reference"
    } else if name.chars().any(|c| c.is_ascii_uppercase()) {
        "mixed case"
    } else if name.contains('_') {
        "contains underscores"
    } else if crate::ids::is_kebab_case(name) {
        "plain name with no matching service"
    } else {
        "other non-canonical name"
    }
}

fn hint(shape: &str) -> &'static str {
    match shape {
        "built from a variable or interpolation" => {
            "The name depends on a variable or function that could not be filled in."
        }
        "comes from another module" => "The name comes from a module output that is not followed.",
        "host name, path or reference" => {
            "A host or reference that matches no known database or service."
        }
        "plain name with no matching service" => {
            "Likely a service whose repository or catalog entry is missing, or that needs an alias."
        }
        _ => "A name that is not in the canonical form; an alias may resolve it.",
    }
}

fn table(out: &mut String, header: &str, rows: &BTreeMap<String, u64>) {
    let _ = writeln!(out, "| {header} | Count |\n|---|---|");
    for (name, count) in rows {
        let _ = writeln!(out, "| {name} | {count} |");
    }
    out.push('\n');
}

/// Markdown report for one snapshot and its unresolved names.
#[must_use]
pub fn build(graph: &GraphSnapshot, unresolved: &[Unresolved], include_names: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# depgraph run report\n");
    let _ = writeln!(
        out,
        "Built {}. {}\n",
        graph.generated_at.as_str(),
        if include_names {
            "Includes up to 40 unresolved names, with secrets removed."
        } else {
            "Counts only: no repository, host or service names."
        }
    );

    let mut nodes: BTreeMap<String, u64> = BTreeMap::new();
    for node in &graph.nodes {
        let mut parts = node.id.split(':');
        let kind = parts.next().unwrap_or("");
        let key = if kind == "resource" {
            format!("resource: {}", parts.next().unwrap_or("other"))
        } else {
            kind.to_owned()
        };
        *nodes.entry(key).or_default() += 1;
    }
    let _ = writeln!(out, "## Nodes ({})\n", graph.nodes.len());
    table(&mut out, "Kind", &nodes);

    let mut edges: BTreeMap<String, u64> = BTreeMap::new();
    let mut evidence: BTreeMap<String, u64> = BTreeMap::new();
    for edge in &graph.edges {
        *edges.entry(edge.edge_type.as_str().to_owned()).or_default() += 1;
        for source in &edge.sources {
            let kind = serde_json::to_value(source.kind)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".into());
            *evidence.entry(kind).or_default() += 1;
        }
    }
    let _ = writeln!(out, "## Edges ({})\n", graph.edges.len());
    table(&mut out, "Type", &edges);
    let _ = writeln!(out, "Evidence behind the edges:\n");
    table(&mut out, "Source", &evidence);

    let mut sources: BTreeMap<String, u64> = BTreeMap::new();
    for (name, summary) in &graph.sources {
        let status = serde_json::to_value(summary.status)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .unwrap_or_default();
        sources.insert(format!("{name} ({status})"), summary.count);
    }
    let _ = writeln!(out, "## Sources\n");
    table(&mut out, "Collector", &sources);

    let _ = writeln!(out, "## Unresolved ({})\n", unresolved.len());
    if unresolved.is_empty() {
        let _ = writeln!(out, "Every name was resolved.\n");
        return out;
    }
    let mut groups: BTreeMap<(String, String, &'static str), u64> = BTreeMap::new();
    for row in unresolved {
        *groups
            .entry((row.kind.clone(), row.reason.clone(), shape(&row.name)))
            .or_default() += 1;
    }
    let _ = writeln!(
        out,
        "| Kind | Reason | Looks like | Count |\n|---|---|---|---|"
    );
    for ((kind, reason, shape), count) in &groups {
        let _ = writeln!(out, "| {kind} | {reason} | {shape} | {count} |");
    }
    let _ = writeln!(out, "\nWhat that usually means:\n");
    let mut shapes: Vec<&str> = groups.keys().map(|(_, _, shape)| *shape).collect();
    shapes.sort_unstable();
    shapes.dedup();
    for shape in shapes {
        let _ = writeln!(out, "- **{shape}**: {}", hint(shape));
    }
    if include_names {
        let _ = writeln!(out, "\nNames (first {NAME_LIMIT}):\n");
        for row in unresolved.iter().take(NAME_LIMIT) {
            let _ = writeln!(out, "- {} `{}`", row.kind, crate::redact::redact(&row.name));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, name: &str, reason: &str) -> Unresolved {
        Unresolved {
            kind: kind.into(),
            name: name.into(),
            reason: reason.into(),
            volume: 0,
        }
    }

    #[test]
    fn names_stay_out_unless_asked_for() {
        let graph: GraphSnapshot = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/empty-graph.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let rows = vec![
            row("resource", "acme-payments-sg", "no component matches"),
            row("resource", "${var.env}-db", "no component matches"),
            row("depends_on", "internal.example.com", "not kebab-case"),
        ];
        let private = build(&graph, &rows, false);
        assert!(!private.contains("acme-payments"));
        assert!(!private.contains("internal.example.com"));
        assert!(private.contains("built from a variable or interpolation"));
        assert!(private.contains("plain name with no matching service"));
        assert!(private.contains("Unresolved (3)"));
        let named = build(&graph, &rows, true);
        assert!(named.contains("acme-payments-sg"));
    }

    #[test]
    fn a_clean_run_says_so() {
        let graph: GraphSnapshot = serde_json::from_str(
            &std::fs::read_to_string(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/empty-graph.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert!(build(&graph, &[], false).contains("Every name was resolved."));
    }
}
