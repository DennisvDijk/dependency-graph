use assert_cmd::Command;
use depgraph::normalize::merge::{Unresolved, UnresolvedReport};
use std::fs;

#[test]
fn suggestions_are_review_only_and_ignore_ambiguous_names() {
    let dir = tempfile::tempdir().unwrap();
    let report = UnresolvedReport {
        schema_version: "1.0.0".into(),
        records: vec![
            Unresolved {
                kind: "resource".into(),
                name: "orders-py-prod-sg".into(),
                reason: "unmatched".into(),
                volume: 0,
            },
            Unresolved {
                kind: "environment".into(),
                name: "shop-prod".into(),
                reason: "namespace is not mapped".into(),
                volume: 4,
            },
            Unresolved {
                kind: "resource".into(),
                name: "unknown,\"name".into(),
                reason: "unmatched".into(),
                volume: 0,
            },
        ],
    };
    let unresolved = dir.path().join("unresolved.json");
    let snapshot = dir.path().join("snapshot.json");
    let environments = dir.path().join("environments.yaml");
    fs::write(&unresolved, serde_json::to_vec(&report).unwrap()).unwrap();
    let source = fs::read_to_string("tests/fixtures/phase1/graph.json").unwrap();
    let mut graph: serde_json::Value = serde_json::from_str(&source).unwrap();
    let node = graph["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["type"] == "component")
        .unwrap();
    node["id"] = "component:orders-py".into();
    fs::write(&snapshot, serde_json::to_vec(&graph).unwrap()).unwrap();
    let before = fs::read(&unresolved).unwrap();
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .args(["suggest", "--unresolved"])
        .arg(&unresolved)
        .arg("--snapshot")
        .arg(&snapshot)
        .arg("--environments")
        .arg(&environments)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(
        text.contains("\"orders-py-prod-sg\": \"orders-py\""),
        "{text}"
    );
    assert!(text.contains("\"shop-prod\""), "{text}");
    assert!(!text.contains("unknown"), "{text}");
    assert_eq!(fs::read(&unresolved).unwrap(), before);
    assert!(!environments.exists());
}

#[cfg(unix)]
#[test]
fn bedrock_proposals_are_checked_against_the_input() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let report = UnresolvedReport {
        schema_version: "1.0.0".into(),
        records: vec![Unresolved {
            kind: "resource".into(),
            name: "legacy-sg".into(),
            reason: "unmatched".into(),
            volume: 0,
        }],
    };
    let unresolved = dir.path().join("unresolved.json");
    let snapshot = dir.path().join("snapshot.json");
    fs::write(&unresolved, serde_json::to_vec(&report).unwrap()).unwrap();
    let source = fs::read_to_string("tests/fixtures/phase1/graph.json").unwrap();
    let mut graph: serde_json::Value = serde_json::from_str(&source).unwrap();
    let node = graph["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["type"] == "component")
        .unwrap();
    node["id"] = "component:orders-py".into();
    fs::write(&snapshot, serde_json::to_vec(&graph).unwrap()).unwrap();
    let response = serde_json::json!({"output":{"message":{"content":[{"text": "{\"legacy-sg\":\"orders-py\",\"invented\":\"orders-py\"}"}]}}});
    let aws = dir.path().join("aws");
    fs::write(&aws, format!("#!/bin/sh\nprintf '%s' '{}'\n", response)).unwrap();
    fs::set_permissions(&aws, fs::Permissions::from_mode(0o755)).unwrap();
    let output = Command::cargo_bin("depgraph")
        .unwrap()
        .args(["suggest", "--unresolved"])
        .arg(&unresolved)
        .arg("--snapshot")
        .arg(&snapshot)
        .arg("--environments")
        .arg(dir.path().join("absent.yaml"))
        .args(["--llm", "--model-id", "test-model"])
        .env("PATH", dir.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("\"legacy-sg\": \"orders-py\" # LLM suggestion, check it"),
        "{text}"
    );
    assert!(!text.contains("invented"), "{text}");
}
