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
    assert!(String::from_utf8_lossy(&result.stderr).contains("no cloud reads were started"));
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
