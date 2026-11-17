//! Append-only snapshot runs.
//!
//! A build archives the snapshot under `runs/<generated_at>/`. Retention uses
//! the timestamp passed in, not the clock.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::models::{GraphSnapshot, Timestamp};

/// Copy `snapshot` into `runs/<timestamp>/snapshot.json` next to it.
///
/// # Errors
///
/// Returns an error when the snapshot cannot be read or the copy fails.
pub fn archive_snapshot(snapshot: &Path) -> Result<PathBuf, Error> {
    let text = fs::read_to_string(snapshot)?;
    let graph: GraphSnapshot = serde_json::from_str(&text)
        .map_err(|err| Error::Store(format!("reading {}: {err}", snapshot.display())))?;
    let runs = runs_dir(snapshot);
    let dest_dir = runs.join(run_key(graph.generated_at.as_str()));
    fs::create_dir_all(&dest_dir)?;
    let dest = dest_dir.join("snapshot.json");
    fs::write(&dest, text)?;
    Ok(dest)
}

/// Run directories, oldest first.
///
/// # Errors
///
/// Returns an error when `runs` cannot be read.
pub fn list_runs(runs: &Path) -> Result<Vec<String>, Error> {
    if !runs.exists() {
        return Ok(Vec::new());
    }
    let mut keys = Vec::new();
    for entry in fs::read_dir(runs)? {
        let entry = entry?;
        if entry.path().join("snapshot.json").is_file() {
            keys.push(entry.file_name().to_string_lossy().into_owned());
        }
    }
    keys.sort();
    Ok(keys)
}

/// Load one archived snapshot.
///
/// # Errors
///
/// Returns an error when the run is missing or the file is not a snapshot.
pub fn load_run(runs: &Path, key: &str) -> Result<GraphSnapshot, Error> {
    let path = runs.join(key).join("snapshot.json");
    let text = fs::read_to_string(&path)
        .map_err(|_| Error::Store(format!("no run `{key}`. next: depgraph build")))?;
    serde_json::from_str(&text).map_err(|err| Error::Store(err.to_string()))
}

/// Newest run whose `generated_at` is at or before `at`.
///
/// # Errors
///
/// Returns an error when no run is old enough.
pub fn run_at(runs: &Path, at: &Timestamp) -> Result<GraphSnapshot, Error> {
    let mut best: Option<(String, GraphSnapshot)> = None;
    for key in list_runs(runs)? {
        let graph = load_run(runs, &key)?;
        if graph.generated_at.as_str() <= at.as_str() {
            let replace = best
                .as_ref()
                .is_none_or(|(have, _)| graph.generated_at.as_str() > have.as_str());
            if replace {
                best = Some((graph.generated_at.as_str().to_owned(), graph));
            }
        }
    }
    best.map(|(_, graph)| graph).ok_or_else(|| {
        Error::Store(format!(
            "no snapshot at or before {}. next: depgraph build",
            at.as_str()
        ))
    })
}

/// Delete runs older than `retention_days` before `now`.
///
/// # Errors
///
/// Returns an error when a directory cannot be removed.
pub fn retain(runs: &Path, now: &Timestamp, retention_days: u32) -> Result<Vec<String>, Error> {
    let now = now.to_datetime().map_err(Error::Validation)?;
    let mut removed = Vec::new();
    for key in list_runs(runs)? {
        let graph = load_run(runs, &key)?;
        let generated = graph
            .generated_at
            .to_datetime()
            .map_err(Error::Validation)?;
        let age = (now - generated).as_hours() / 24;
        if age > i64::from(retention_days) {
            fs::remove_dir_all(runs.join(&key))?;
            removed.push(key);
        }
    }
    Ok(removed)
}

fn runs_dir(snapshot: &Path) -> PathBuf {
    snapshot
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("runs")
}

fn run_key(generated_at: &str) -> String {
    generated_at.replace(':', "-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::GraphSnapshot;

    fn graph(at: &str) -> String {
        let snapshot = GraphSnapshot {
            schema_version: "0.1.0".into(),
            run_id: "local".into(),
            generated_at: Timestamp::parse(at).unwrap(),
            sources: Default::default(),
            nodes: Vec::new(),
            edges: Vec::new(),
        };
        crate::models::to_canonical_json(&snapshot).unwrap()
    }

    #[test]
    fn archive_lists_and_retains_by_the_given_clock() {
        let dir = tempfile::tempdir().unwrap();
        let snapshot = dir.path().join("snapshot.json");
        fs::write(&snapshot, graph("2026-01-01T00:00:00Z")).unwrap();
        archive_snapshot(&snapshot).unwrap();
        fs::write(&snapshot, graph("2026-04-01T00:00:00Z")).unwrap();
        archive_snapshot(&snapshot).unwrap();
        let runs = dir.path().join("runs");
        assert_eq!(
            list_runs(&runs).unwrap(),
            vec![
                "2026-01-01T00-00-00Z".to_owned(),
                "2026-04-01T00-00-00Z".to_owned()
            ]
        );
        let at = run_at(&runs, &Timestamp::parse("2026-02-01T00:00:00Z").unwrap()).unwrap();
        assert_eq!(at.generated_at.as_str(), "2026-01-01T00:00:00Z");
        let removed = retain(
            &runs,
            &Timestamp::parse("2026-06-01T00:00:00Z").unwrap(),
            90,
        )
        .unwrap();
        assert_eq!(removed, vec!["2026-01-01T00-00-00Z".to_owned()]);
        assert!(
            list_runs(&runs)
                .unwrap()
                .contains(&"2026-04-01T00-00-00Z".into())
        );
    }
}
