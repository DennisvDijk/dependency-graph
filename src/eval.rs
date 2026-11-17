//! Precision and recall of a snapshot against a hand-verified list of true edges.
//!
//! The truth file is written by a person who read the system's own manifests,
//! never copied from depgraph's output. Names are component names; any other
//! node is written with its full id (`resource:queue:orders`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::Deserialize;

use crate::error::Error;
use crate::models::{EdgeType, GraphSnapshot};

/// The truth file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Truth {
    /// True dependencies, written `caller -> callee`.
    pub edges: Vec<String>,
}

/// Counts for one slice of the result.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Slice {
    /// Predicted edges in this slice.
    pub predicted: usize,
    /// Predicted edges that are true.
    pub correct: usize,
    /// True edges this slice found.
    pub found: usize,
}

/// Result of comparing a snapshot with the truth.
#[derive(Debug, Default)]
pub struct Report {
    /// Number of true edges.
    pub truth: usize,
    /// Number of predicted dependency edges.
    pub predicted: usize,
    /// Predicted edges that are true.
    pub correct: usize,
    /// True edges the tool missed.
    pub missed: Vec<String>,
    /// Predicted edges that are not in the truth.
    pub extra: Vec<String>,
    /// Per edge type.
    pub by_type: BTreeMap<String, Slice>,
    /// Per evidence source.
    pub by_source: BTreeMap<String, Slice>,
}

impl Report {
    /// Share of predicted edges that are true. 1.0 when nothing is predicted.
    #[must_use]
    pub fn precision(&self) -> f64 {
        ratio(self.correct, self.predicted)
    }

    /// Share of true edges that were found. 1.0 when the truth is empty.
    #[must_use]
    pub fn recall(&self) -> f64 {
        ratio(self.correct, self.truth)
    }
}

#[allow(clippy::cast_precision_loss)]
fn ratio(part: usize, whole: usize) -> f64 {
    if whole == 0 {
        1.0
    } else {
        part as f64 / whole as f64
    }
}

/// Read a truth file.
///
/// # Errors
/// Returns an error when the file is unreadable or malformed.
pub fn load_truth(path: &Path) -> Result<Truth, Error> {
    let text = std::fs::read_to_string(path)
        .map_err(|err| Error::Validation(format!("reading {}: {err}", path.display())))?;
    yaml_serde::from_str(&text)
        .map_err(|err| Error::Validation(format!("{}: {err}", path.display())))
}

fn short(id: &str) -> &str {
    id.strip_prefix("component:").unwrap_or(id)
}

fn is_dependency(kind: EdgeType) -> bool {
    kind.is_service_tie()
}

fn parse_edge(text: &str) -> Result<(String, String), Error> {
    let (from, to) = text
        .split_once("->")
        .ok_or_else(|| Error::Validation(format!("truth edge `{text}` needs `a -> b`")))?;
    Ok((from.trim().to_string(), to.trim().to_string()))
}

/// Compare a snapshot with the truth.
///
/// # Errors
/// Returns an error when a truth line is not written `a -> b`.
pub fn score(graph: &GraphSnapshot, truth: &Truth) -> Result<Report, Error> {
    let truth_set: BTreeSet<(String, String)> = truth
        .edges
        .iter()
        .map(|line| parse_edge(line))
        .collect::<Result<_, _>>()?;

    // One predicted pair can carry several edge types and sources.
    let mut predicted: BTreeMap<(String, String), (BTreeSet<String>, BTreeSet<String>)> =
        BTreeMap::new();
    for edge in graph
        .edges
        .iter()
        .filter(|edge| edge.active && is_dependency(edge.edge_type))
    {
        let key = (
            short(&edge.from_id).to_string(),
            short(&edge.to_id).to_string(),
        );
        let entry = predicted.entry(key).or_default();
        entry.0.insert(edge.edge_type.as_str().to_string());
        for source in &edge.sources {
            entry.1.insert(
                serde_json::to_value(source.kind)
                    .ok()
                    .and_then(|v| v.as_str().map(str::to_owned))
                    .unwrap_or_default(),
            );
        }
    }

    let mut report = Report {
        truth: truth_set.len(),
        predicted: predicted.len(),
        ..Report::default()
    };
    for (pair, (types, sources)) in &predicted {
        let is_true = truth_set.contains(pair);
        if is_true {
            report.correct += 1;
        } else {
            report.extra.push(format!("{} -> {}", pair.0, pair.1));
        }
        for (names, table) in [
            (types, &mut report.by_type),
            (sources, &mut report.by_source),
        ] {
            for name in names {
                let slice = table.entry(name.clone()).or_default();
                slice.predicted += 1;
                if is_true {
                    slice.correct += 1;
                    slice.found += 1;
                }
            }
        }
    }
    for pair in &truth_set {
        if !predicted.contains_key(pair) {
            report.missed.push(format!("{} -> {}", pair.0, pair.1));
        }
    }
    Ok(report)
}

/// Plain-text report.
#[must_use]
pub fn render(report: &Report) -> String {
    let mut out = format!(
        "truth {}  predicted {}  correct {}\nprecision {:.2}  recall {:.2}\n",
        report.truth,
        report.predicted,
        report.correct,
        report.precision(),
        report.recall()
    );
    for (title, table) in [
        ("by edge type", &report.by_type),
        ("by source", &report.by_source),
    ] {
        out.push_str(&format!("\n{title}\n"));
        for (name, slice) in table {
            out.push_str(&format!(
                "  {name}: predicted {}, correct {}, precision {:.2}, recall {:.2}\n",
                slice.predicted,
                slice.correct,
                ratio(slice.correct, slice.predicted),
                ratio(slice.found, report.truth)
            ));
        }
    }
    if !report.missed.is_empty() {
        out.push_str("\nmissed (true, not found)\n");
        for line in &report.missed {
            out.push_str(&format!("  {line}\n"));
        }
    }
    if !report.extra.is_empty() {
        out.push_str("\nextra (found, not in the truth)\n");
        for line in &report.extra {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn graph(pairs: &[(&str, &str)]) -> GraphSnapshot {
        let edges: Vec<serde_json::Value> = pairs
            .iter()
            .enumerate()
            .map(|(i, (from, to))| {
                serde_json::json!({
                    "id": format!("e{i}"),
                    "type": "candidate_calls",
                    "from_id": format!("component:{from}"),
                    "to_id": format!("component:{to}"),
                    "sources": [{"kind": "static",
                        "first_seen": "2026-01-01T00:00:00Z",
                        "last_seen": "2026-01-01T00:00:00Z"}]
                })
            })
            .collect();
        serde_json::from_value(serde_json::json!({
            "schema_version": "0.1.0",
            "run_id": "t",
            "generated_at": "2026-01-01T00:00:00Z",
            "sources": {},
            "nodes": [],
            "edges": edges
        }))
        .expect("snapshot")
    }

    fn truth(lines: &[&str]) -> Truth {
        Truth {
            edges: lines.iter().map(ToString::to_string).collect(),
        }
    }

    #[test]
    fn counts_hits_misses_and_extras() {
        let g = graph(&[("a", "b"), ("a", "c")]);
        let r = score(&g, &truth(&["a -> b", "a -> d"])).expect("score");
        assert_eq!((r.truth, r.predicted, r.correct), (2, 2, 1));
        assert_eq!(r.missed, vec!["a -> d"]);
        assert_eq!(r.extra, vec!["a -> c"]);
        assert!((r.precision() - 0.5).abs() < f64::EPSILON);
        assert!((r.recall() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn a_malformed_truth_line_is_an_error() {
        let g = graph(&[]);
        assert!(score(&g, &truth(&["a to b"])).is_err());
    }

    #[test]
    fn an_empty_prediction_has_full_precision_and_no_recall() {
        let g = graph(&[]);
        let r = score(&g, &truth(&["a -> b"])).expect("score");
        assert!((r.precision() - 1.0).abs() < f64::EPSILON);
        assert!(r.recall().abs() < f64::EPSILON);
    }
}
