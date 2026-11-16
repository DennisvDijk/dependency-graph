mod common;

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use depgraph::collectors::{RuntimeFilter, candidate_records, records_from_prometheus_dir};
use depgraph::config::{parse_aliases, parse_environments};
use depgraph::models::{EdgeStatus, EdgeType, Timestamp, to_canonical_json};
use depgraph::normalize::merge::{MergeOptions, merge_records};
use depgraph::raw::RawRecord;
use serde_json::json;

fn environments() -> depgraph::config::EnvIndex {
    parse_environments(
        r"
dev:
  gitlab: [development]
  otel: [dev]
  k8s_namespaces: [payments-dev]
acc:
  gitlab: [acceptance]
  otel: [acc]
  k8s_namespaces: [payments-acc]
prod:
  gitlab: [production]
  otel: [prod]
  k8s_namespaces: [payments-prod]
",
    )
    .unwrap()
    .index()
    .unwrap()
}

fn aliases() -> depgraph::config::Aliases {
    parse_aliases("aliases:\n  payments: payments-api\n").unwrap()
}

fn component(name: &str, depends_on: &[&str]) -> RawRecord {
    RawRecord {
        collector: "gitlab_inventory".to_owned(),
        kind: "component".to_owned(),
        payload: json!({
            "name": name,
            "depends_on": depends_on.iter().map(|item| format!("component:default/{item}")).collect::<Vec<_>>(),
        }),
    }
}

fn phase2_records() -> Vec<RawRecord> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2");
    let filter = RuntimeFilter {
        health_paths: vec!["/health".to_owned()],
        test_clients: vec!["k6".to_owned()],
    };
    let mut records = records_from_prometheus_dir(&root, "7d", &filter).unwrap();
    let manifest = fs::read_to_string(root.join("virtualservice.yaml")).unwrap();
    records.extend(candidate_records(&manifest));
    records.push(component("payments-api", &["billing-api"]));
    records.push(component("billing-api", &[]));
    records
}

fn built(records: &[RawRecord], generated_at: &str) -> depgraph::normalize::merge::BuildOutput {
    let generated_at = Timestamp::parse(generated_at).unwrap();
    let aliases = aliases();
    let environments = environments();
    merge_records(
        records,
        &MergeOptions {
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
fn runtime_graph_is_golden_and_idempotent() {
    let records = phase2_records();
    let first = built(&records, "2026-01-15T00:00:00Z");
    let second = built(&records, "2026-01-15T00:00:00Z");
    let json = to_canonical_json(&first.graph).unwrap();
    assert_eq!(json, to_canonical_json(&second.graph).unwrap());
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase2/graph.json");
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        fs::write(&path, &json).unwrap();
    }
    let expected = fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        json, expected,
        "graph.json drifted; regenerate with UPDATE_GOLDEN=1"
    );

    let observed = first
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.edge_type == EdgeType::ObservedCalls
                && edge.from_id == "component:payments-api"
                && edge.to_id == "component:billing-api"
        })
        .unwrap();
    assert_eq!(observed.status, EdgeStatus::Both);
    assert_eq!(observed.attrs.calls_7d, Some(100));
    assert_eq!(observed.attrs.error_rate_7d, Some(0.25));
    assert_eq!(observed.attrs.p95_ms_7d, Some(40.0));
    assert_eq!(observed.attrs.protocol.as_deref(), Some("http"));
    assert!(observed.attrs.calls_total.is_none());

    let declared = first
        .graph
        .edges
        .iter()
        .find(|edge| edge.edge_type == EdgeType::DeclaredDependsOn)
        .unwrap();
    assert_eq!(declared.status, EdgeStatus::Both);

    let candidate = first
        .graph
        .edges
        .iter()
        .find(|edge| edge.edge_type == EdgeType::CandidateCalls)
        .unwrap();
    assert_eq!(candidate.status, EdgeStatus::Candidate);
    assert_ne!(candidate.edge_type, EdgeType::ObservedCalls);

    let external = first
        .graph
        .nodes
        .iter()
        .find(|node| node.id == "resource:external-api:api-stripe-com")
        .unwrap();
    assert_eq!(
        external
            .attrs
            .get("inferred")
            .and_then(|value| value.as_bool()),
        Some(true)
    );

    assert!(
        first
            .unresolved
            .iter()
            .any(|row| { row.name == "mystery" && row.volume == 9 && row.kind == "observed_call" })
    );
    assert!(
        first
            .unresolved
            .iter()
            .any(|row| row.kind == "environment" && row.name == "qa" && row.volume == 3)
    );
    assert!(!json.contains("k6"));
    assert!(!json.contains("/health"));
    assert_eq!(first.coverage.observed_edges, 2);
    assert!(first.coverage.candidates >= 1);
    assert!(first.coverage.unresolved >= 2);
    assert_eq!(first.coverage.inferred_resources, 1);
}

#[test]
fn one_day_window_is_a_running_counter_and_expiry_uses_the_given_time() {
    let mut records = phase2_records();
    records.retain(|record| record.kind != "observed_call");
    records.push(RawRecord {
        collector: "runtime_edges".to_owned(),
        kind: "observed_call".to_owned(),
        payload: json!({
            "source": "payments-api",
            "destination": "billing-api",
            "source_namespace": "payments-prod",
            "destination_namespace": "payments-prod",
            "protocol": "http",
            "calls": 10,
            "errors": 0,
            "window": "1d",
            "last_seen": "2026-01-14T00:00:00Z",
            "external": false
        }),
    });
    records.push(RawRecord {
        collector: "runtime_edges".to_owned(),
        kind: "observed_call".to_owned(),
        payload: json!({
            "source": "payments-api",
            "destination": "billing-api",
            "source_namespace": "payments-prod",
            "destination_namespace": "payments-prod",
            "protocol": "http",
            "calls": 4,
            "errors": 0,
            "window": "1d",
            "last_seen": "2026-01-15T00:00:00Z",
            "external": false
        }),
    });
    let daily = built(&records, "2026-01-15T00:00:00Z");
    let edge = daily
        .graph
        .edges
        .iter()
        .find(|edge| edge.edge_type == EdgeType::ObservedCalls)
        .unwrap();
    assert_eq!(edge.attrs.calls_total, Some(14));
    assert!(edge.attrs.calls_7d.is_none());
    assert!(edge.active);

    let mut aged = phase2_records();
    for record in &mut aged {
        if record.kind == "observed_call"
            && record.payload["destination"] == "billing-api"
            && record.payload["source_namespace"] == "payments-prod"
        {
            record.payload["last_seen"] = json!("2025-12-06T00:00:00Z");
        }
        if record.payload["external"].as_bool() == Some(true) {
            record.payload["last_seen"] = json!("2025-10-07T00:00:00Z");
        }
    }
    let expired = built(&aged, "2026-01-15T00:00:00Z");
    let stale = expired
        .graph
        .edges
        .iter()
        .find(|edge| {
            edge.edge_type == EdgeType::ObservedCalls && edge.to_id == "component:billing-api"
        })
        .unwrap();
    assert!(!stale.active);
    assert!(
        expired
            .graph
            .edges
            .iter()
            .all(|edge| edge.to_id != "resource:external-api:api-stripe-com")
    );
}

#[test]
fn drift_writes_one_file_per_env_and_library_impact_is_disabled() {
    let records = phase2_records();
    let output = built(&records, "2026-01-15T00:00:00Z");
    let dir = tempfile::tempdir().unwrap();
    let snapshot = dir.path().join("snapshot.json");
    fs::write(&snapshot, to_canonical_json(&output.graph).unwrap()).unwrap();
    let report = dir.path().join("drift_report");
    Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "drift",
            "--snapshot",
            snapshot.to_str().unwrap(),
            "--output",
            report.to_str().unwrap(),
        ])
        .assert()
        .success();
    let prod = fs::read_to_string(report.join("prod.md")).unwrap();
    assert!(prod.contains("## both"));
    assert!(prod.contains("component:payments-api -> component:billing-api"));
    assert!(prod.contains("calls_7d=100"));
    assert!(prod.contains("resource:external-api:api-stripe-com"));
    assert!(report.join("dev.md").is_file());
    assert!(report.join("acc.md").is_file());
    Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .arg("library-impact")
        .assert()
        .success()
        .stdout(predicates::str::contains("libraries disabled"));
}
