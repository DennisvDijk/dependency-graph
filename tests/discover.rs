use assert_cmd::Command;
use std::fs;
use std::path::{Path, PathBuf};

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}
fn setup(root: &Path) {
    fs::create_dir_all(root.join("config")).unwrap();
    let landscape = manifest().join("tests/fixtures/landscape");
    for name in ["config.yaml", "pilot.yaml", "sdk_hints.yaml"] {
        fs::copy(
            manifest().join("config").join(name),
            root.join("config").join(name),
        )
        .unwrap();
    }
    for name in ["aliases.yaml", "environments.yaml"] {
        fs::copy(landscape.join(name), root.join("config").join(name)).unwrap();
    }
}

#[test]
fn noninteractive_discovery_requires_an_explicit_source_choice() {
    let dir = tempfile::tempdir().unwrap();
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(dir.path())
        .arg("discover")
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(String::from_utf8_lossy(&result.stderr).contains("no live reads were started"));
    assert!(!dir.path().join("data").exists());
}

#[test]
fn discovery_builds_a_fresh_graph_without_reusing_old_raw_data() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    setup(root);
    fs::create_dir_all(root.join("data/raw")).unwrap();
    fs::write(root.join("data/raw/old.jsonl"), "{\"collector\":\"old\",\"kind\":\"network_flow\",\"payload\":{\"source\":\"stale-component\",\"destination\":\"other\",\"last_seen\":\"2026-11-16T20:00:00Z\"}}\n").unwrap();
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(root)
        .args(["discover", "--repos-only", "--fixture"])
        .arg(manifest().join("tests/fixtures/landscape"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let stdout = String::from_utf8(result.stdout).unwrap();
    assert!(stdout.contains("Discovery:"), "{stdout}");
    assert!(stdout.contains("data/snapshot.json"), "{stdout}");
    assert!(root.join("data/snapshot.json").exists());
    assert!(root.join("data/raw/old.jsonl").exists());
    assert!(
        !fs::read_to_string(root.join("data/unresolved.json"))
            .unwrap()
            .contains("stale-component")
    );
    assert_eq!(
        fs::read_dir(root.join("data/discovery")).unwrap().count(),
        1
    );
}

#[test]
fn discovery_combines_the_selected_saved_sources() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    setup(root);
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(root)
        .args(["discover", "--aws", "--azure", "--fixture"])
        .arg(manifest().join("tests/fixtures/landscape"))
        .arg("--aws-fixture")
        .arg(manifest().join("tests/fixtures/cloud/aws-flows"))
        .arg("--azure-fixture")
        .arg(manifest().join("tests/fixtures/cloud/azure-flows"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(text.contains("saved AWS fixture"), "{text}");
    assert!(text.contains("saved Azure fixture"), "{text}");
    let graph: serde_json::Value =
        serde_json::from_slice(&fs::read(root.join("data/snapshot.json")).unwrap()).unwrap();
    let edges = graph["edges"].as_array().unwrap();
    assert!(edges.iter().any(|e| e["type"] == "network_traffic"
        && e["from_id"].as_str().unwrap().starts_with("component:")));
    assert!(edges.iter().any(|e| {
        e["type"] == "network_traffic"
            && e["from_id"]
                .as_str()
                .unwrap()
                .starts_with("resource:infra:azvm-")
    }));
}

#[test]
fn discovery_can_include_saved_prometheus_data() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    setup(root);
    fs::copy(
        manifest().join("tests/fixtures/phase1/aliases.yaml"),
        root.join("config/aliases.yaml"),
    )
    .unwrap();
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(root)
        .args(["discover", "--prometheus", "--fixture"])
        .arg(manifest().join("tests/fixtures/phase1"))
        .arg("--prometheus-fixture")
        .arg(manifest().join("tests/fixtures/phase2"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let run = fs::read_dir(root.join("data/discovery"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert!(run.join("raw/runtime_edges.jsonl").exists());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("Prometheus was not queried"));
}

#[test]
fn kubernetes_fixture_discovers_workloads_without_mappings_or_secrets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    setup(root);
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(root)
        .args(["discover", "--kubernetes", "--fixture"])
        .arg(manifest().join("tests/fixtures/landscape"))
        .arg("--kubernetes-fixture")
        .arg(manifest().join("tests/fixtures/kubernetes-live"))
        .env("PATH", "/usr/bin:/bin")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let output = String::from_utf8(result.stdout).unwrap();
    assert!(output.contains("1 cluster, 5 workload objects"), "{output}");
    let graph_text = fs::read_to_string(root.join("data/snapshot.json")).unwrap();
    assert!(!graph_text.contains("secret-value-never-include"));
    let graph: serde_json::Value = serde_json::from_str(&graph_text).unwrap();
    assert_eq!(graph["schema_version"], "0.2.0");
    let nodes = graph["nodes"].as_array().unwrap();
    let cluster = nodes.iter().find(|node| node["type"] == "cluster").unwrap();
    assert_eq!(cluster["attrs"]["display_name"], "sample-prod");
    let workloads: Vec<_> = nodes
        .iter()
        .filter(|node| node["type"] == "workload")
        .collect();
    assert_eq!(workloads.len(), 5);
    let same_name: Vec<_> = workloads
        .iter()
        .filter(|node| {
            node["attrs"]["display_name"]
                .as_str()
                .unwrap()
                .ends_with("/orders-web")
        })
        .collect();
    assert_eq!(same_name.len(), 2);
    assert_ne!(same_name[0]["id"], same_name[1]["id"]);
    let edges = graph["edges"].as_array().unwrap();
    assert_eq!(
        edges
            .iter()
            .filter(|edge| edge["type"] == "runs_in_cluster")
            .count(),
        5
    );
    let links: Vec<_> = edges
        .iter()
        .filter(|edge| edge["type"] == "has_workload")
        .collect();
    assert_eq!(links.len(), 2, "{links:#?}");
    assert!(
        links
            .iter()
            .any(|edge| edge["from_id"] == "component:orders-py")
    );
    assert!(
        links
            .iter()
            .any(|edge| edge["from_id"] == "component:billing-ex")
    );
    assert!(!links.iter().any(|edge| {
        workloads
            .iter()
            .find(|node| node["id"] == edge["to_id"])
            .unwrap()["attrs"]["display_name"]
            == "shop-prod/agent"
    }));
    let raw = fs::read_dir(root.join("data/discovery"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("raw/kubernetes_cluster.jsonl");
    assert!(
        !fs::read_to_string(raw)
            .unwrap()
            .contains("secret-value-never-include")
    );
}
