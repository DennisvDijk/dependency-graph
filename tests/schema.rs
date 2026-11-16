mod common;

use std::fs;
use std::path::PathBuf;

use depgraph::models::graph_schema_value;

fn schema_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("schemas/graph.schema.json")
}

#[test]
fn graph_schema_matches_the_rust_models() {
    let generated = graph_schema_value();
    let path = schema_path();
    if std::env::var("UPDATE_SCHEMA").is_ok() {
        let mut text = serde_json::to_string_pretty(&generated).unwrap();
        text.push('\n');
        fs::write(&path, text).unwrap();
    }
    let on_disk: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        generated, on_disk,
        "schemas/graph.schema.json drifted from the Rust models; regenerate with UPDATE_SCHEMA=1"
    );
}

#[test]
fn jsonschema_accepts_canonical_graphs_and_rejects_garbage() {
    let schema: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(schema_path()).unwrap()).unwrap();
    let validator = jsonschema::validator_for(&schema).expect("schema compiles");
    let empty: serde_json::Value =
        serde_json::from_str(&depgraph::models::to_canonical_json(&common::empty_graph()).unwrap())
            .unwrap();
    let sample: serde_json::Value = serde_json::from_str(
        &depgraph::models::to_canonical_json(&common::sample_graph()).unwrap(),
    )
    .unwrap();
    assert!(validator.is_valid(&empty), "empty graph");
    assert!(validator.is_valid(&sample), "sample graph");
    assert!(!validator.is_valid(&serde_json::json!({"schema_version": 1})));
}

#[test]
fn catalog_stub_accepts_the_fixture_and_rejects_a_wrong_kind() {
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../schemas/catalog-info.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let document: serde_json::Value =
        yaml_serde::from_str(include_str!("fixtures/catalog-info.yaml")).unwrap();
    assert!(
        validator.is_valid(&document),
        "catalog fixture should match the stub schema"
    );
    let mut broken = document.clone();
    broken["kind"] = serde_json::json!("System");
    assert!(!validator.is_valid(&broken));
}
