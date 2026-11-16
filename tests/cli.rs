mod common;

use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn depgraph() -> Command {
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

#[test]
fn validate_accepts_the_empty_fixture() {
    depgraph()
        .args(["validate", "tests/fixtures/empty-graph.json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
}

#[test]
fn validate_rejects_a_secret_without_echoing_it() {
    let mut graph = common::empty_graph();
    graph.nodes.push(depgraph::models::Node {
        id: "component:payments-api".to_owned(),
        node_type: depgraph::models::NodeType::Component,
        attrs: [(
            "url".to_owned(),
            serde_json::Value::String("https://user:password@gitlab.example.com".to_owned()),
        )]
        .into_iter()
        .collect(),
    });
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("graph.json");
    std::fs::write(&path, depgraph::models::to_canonical_json(&graph).unwrap()).unwrap();
    let output = depgraph()
        .args([
            "validate",
            path.to_str().unwrap(),
            "--aliases",
            "config/aliases.yaml",
        ])
        .assert()
        .failure()
        .code(1)
        .get_output()
        .clone();
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("secret_material"), "{stderr}");
    assert!(!stderr.contains("password"), "{stderr}");
}

#[test]
fn stubs_exit_2() {
    depgraph()
        .args(["impact", "payments-api"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("not implemented in F0-1"));
    depgraph()
        .args(["publish", "drift-issue"])
        .assert()
        .failure()
        .code(2);
    depgraph()
        .arg("serve-mcp")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("not implemented in F0-1"));
}

#[test]
fn help_lists_validate() {
    depgraph()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("validate"));
}

#[test]
fn read_commands_accept_the_shared_flags() {
    let commands = [
        "impact",
        "deploy-order",
        "cycles",
        "library-impact",
        "drift",
        "critical",
        "orphans",
        "diff",
        "at",
    ];
    for command in commands {
        depgraph()
            .args([
                command,
                "payments-api",
                "--env",
                "acc",
                "--format",
                "mermaid",
                "--snapshot",
                "tests/fixtures/empty-graph.json",
            ])
            .assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("not implemented in F0-1"));
    }
}

#[test]
fn missing_set_is_usage() {
    depgraph().arg("collect").assert().failure().code(2);
}

#[test]
fn library_validate_matches_the_binary_for_the_sample() {
    let graph = common::sample_graph();
    assert!(depgraph::normalize::validate(&graph, None).is_ok());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("sample-graph.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, depgraph::models::to_canonical_json(&graph).unwrap()).unwrap();
    depgraph()
        .args(["validate", path.to_str().unwrap()])
        .assert()
        .success();
}
