//! `depgraph diff` on archived runs, through the real binary.

use std::fs;
use std::path::Path;
use std::process::Command;

fn snapshot(at: &str, nodes: &[&str]) -> String {
    let nodes: Vec<String> = nodes
        .iter()
        .map(|id| format!(r#"{{"id":"component:{id}","type":"component","attrs":{{}}}}"#))
        .collect();
    format!(
        r#"{{"schema_version":"0.1.0","run_id":"r","generated_at":"{at}","sources":{{}},"nodes":[{}],"edges":[]}}"#,
        nodes.join(",")
    )
}

fn archive(runs: &Path, key: &str, at: &str, nodes: &[&str]) {
    let dir = runs.join(key);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("snapshot.json"), snapshot(at, nodes)).unwrap();
}

fn diff(runs: &Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_depgraph"))
        .arg("diff")
        .args(args)
        .arg("--runs")
        .arg(runs)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

#[test]
fn diff_without_arguments_compares_the_two_newest_runs() {
    let dir = tempfile::tempdir().unwrap();
    let runs = dir.path();
    archive(runs, "2026-01-01T00-00-00Z", "2026-01-01T00:00:00Z", &["a"]);
    archive(
        runs,
        "2026-01-02T00-00-00Z",
        "2026-01-02T00:00:00Z",
        &["a", "b"],
    );
    archive(
        runs,
        "2026-01-03T00-00-00Z",
        "2026-01-03T00:00:00Z",
        &["b", "c"],
    );

    let (ok, out, err) = diff(runs, &[]);
    assert!(ok, "{err}");
    assert!(out.contains("nodes: +1 -1"), "{out}");
    assert!(
        out.contains("component:c") && out.contains("component:a"),
        "{out}"
    );

    let (ok, out, _) = diff(runs, &["--from", "2026-01-01T00-00-00Z", "--to", "latest"]);
    assert!(ok);
    assert!(out.contains("nodes: +2 -1"), "{out}");

    let (ok, out, _) = diff(
        runs,
        &["--from", "previous", "--to", "latest", "--format", "json"],
    );
    assert!(ok);
    let value: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["nodes_added"][0], "component:c");
    assert!(value["edges_changed"].as_array().unwrap().is_empty());
}

#[test]
fn diff_with_one_run_says_what_to_do() {
    let dir = tempfile::tempdir().unwrap();
    archive(
        dir.path(),
        "2026-01-01T00-00-00Z",
        "2026-01-01T00:00:00Z",
        &["a"],
    );
    let (ok, _, err) = diff(dir.path(), &[]);
    assert!(!ok);
    assert!(err.contains("depgraph runs"), "{err}");
}
