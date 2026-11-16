mod common;

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use depgraph::collectors::{GatherSet, records_from_fixture};
use depgraph::config::{parse_aliases, parse_environments};
use depgraph::models::{Timestamp, to_canonical_json};
use depgraph::normalize::merge::merge_records;
use predicates::prelude::*;

fn built() -> depgraph::normalize::merge::BuildOutput {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase1");
    let records = records_from_fixture(&root, GatherSet::Full).unwrap();
    let aliases = parse_aliases(&fs::read_to_string(root.join("aliases.yaml")).unwrap()).unwrap();
    let environments = parse_environments(include_str!("../config/environments.yaml"))
        .unwrap()
        .index()
        .unwrap();
    let generated_at = Timestamp::parse("2026-01-15T00:00:00Z").unwrap();
    merge_records(
        &records,
        &depgraph::normalize::merge::MergeOptions {
            aliases: &aliases,
            environments: &environments,
            generated_at: &generated_at,
            run_id: "local",
            libraries_enabled: false,
            edge_expiry_days: 30,
            retention_days: 90,
        },
    )
    .unwrap()
}

#[test]
fn graph_is_golden_and_idempotent() {
    let first = built();
    let second = built();
    let json = to_canonical_json(&first.graph).unwrap();
    assert_eq!(json, to_canonical_json(&second.graph).unwrap());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase1/graph.json");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::write(&path, &json).unwrap();
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        json, expected,
        "graph.json drifted; regenerate with UPDATE_GOLDEN=1"
    );
    assert!(
        first
            .mismatches
            .iter()
            .any(|item| item.chosen == "1.4.0" && item.discarded == vec!["1.2.0".to_owned()])
    );
    assert!(
        first
            .unresolved
            .iter()
            .any(|row| row.name.contains("unknown-api"))
    );
    let unresolved = depgraph::normalize::merge::unresolved_csv(&first.unresolved);
    assert!(unresolved.starts_with("kind,name,reason,volume\n"));
}

#[test]
fn show_deps_and_consumers_read_the_golden_snapshot() {
    let snapshot = "tests/fixtures/phase1/graph.json";
    let run = |args: &[&str]| {
        Command::cargo_bin("depgraph")
            .unwrap()
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .args(args)
            .assert()
    };
    run(&[
        "show",
        "payments-api",
        "--snapshot",
        snapshot,
        "--format",
        "json",
    ])
    .success()
    .stdout(predicate::str::contains("component:payments-api"));
    run(&[
        "deps",
        "payments-api",
        "--snapshot",
        snapshot,
        "--env",
        "prod",
    ])
    .success()
    .stdout(predicate::str::contains("billing-api"));
    run(&[
        "consumers",
        "billing-api",
        "--snapshot",
        snapshot,
        "--format",
        "mermaid",
    ])
    .success()
    .stdout(predicate::str::contains("flowchart LR"));
}

#[test]
fn site_and_publish_write_local_files() {
    let dir = tempfile::tempdir().unwrap();
    let snapshot =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase1/graph.json");
    let site = dir.path().join("site");
    Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "site",
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--output",
            site.to_str().unwrap(),
        ])
        .assert()
        .success();
    let html = fs::read_to_string(site.join("index.html")).unwrap();
    assert!(html.contains("component:payments-api"));
    assert!(!html.contains("glpat-"));
    let published = dir.path().join("published.json");
    Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "publish",
            "snapshot",
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--output",
            published.to_str().unwrap(),
        ])
        .assert()
        .success();
    assert!(published.is_file());
}
