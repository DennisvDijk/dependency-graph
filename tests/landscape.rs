//! End-to-end check of nodes and edges on a mixed landscape: a Python
//! service, a Terraform-only repo, open-source apps in one deploy repo, a
//! soft-only dependency, an external API, AWS security groups and an
//! unmapped namespace.
//! See `tests/fixtures/landscape/README.md`.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use predicates::prelude::*;
use serde_json::Value;

struct Built {
    _dir: tempfile::TempDir,
    root: PathBuf,
    graph: Value,
    build_stderr: String,
}

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/landscape")
}

fn depgraph(root: &Path) -> Command {
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command.current_dir(root);
    command
}

fn build() -> Built {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    let repo_config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config");
    for name in ["config.yaml", "pilot.yaml", "sdk_hints.yaml"] {
        fs::copy(repo_config.join(name), config.join(name)).unwrap();
    }
    for name in ["aliases.yaml", "environments.yaml"] {
        fs::copy(fixture().join(name), config.join(name)).unwrap();
    }
    depgraph(&root)
        .args(["collect", "--set", "full", "--fixture"])
        .arg(fixture())
        .arg("--prometheus-fixture")
        .arg(fixture().join("prometheus"))
        .args(["--output", "data"])
        .assert()
        .success();
    let output = depgraph(&root).arg("build").assert().success();
    let build_stderr = String::from_utf8(output.get_output().stderr.clone()).unwrap();
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(root.join("data/snapshot.json")).unwrap())
            .unwrap();
    Built {
        _dir: dir,
        root,
        graph,
        build_stderr,
    }
}

fn text(value: &Value) -> String {
    match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

/// `type from -> to env status criticality sources`
fn edge_lines(graph: &Value) -> BTreeSet<String> {
    graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .map(|edge| {
            let sources: Vec<String> = edge["sources"]
                .as_array()
                .unwrap()
                .iter()
                .map(|source| text(&source["kind"]))
                .collect();
            format!(
                "{} {} -> {} {} {} {} {}",
                text(&edge["type"]),
                text(&edge["from_id"]),
                text(&edge["to_id"]),
                text(&edge["env"]),
                text(&edge["status"]),
                text(&edge["criticality"]),
                sources.join(",")
            )
        })
        .collect()
}

#[test]
fn every_node_is_expected() {
    let built = build();
    let nodes: BTreeSet<String> = built.graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| text(&node["id"]))
        .collect();
    let expected: BTreeSet<String> = [
        "component:billing-ex",
        "component:edge-proxy-rs",
        "component:keycloak",
        "component:notifications",
        "component:orders-py",
        "deployment:keycloak@prod",
        "deployment:orders-py@acc",
        "deployment:orders-py@prod",
        "resource:database:orders-db",
        "resource:gateway:public-edge",
        "resource:queue:notify-jobs",
        "resource:topic:order-events",
        "resource:database:redis-cache",
        "resource:external-api:api-openai-com",
        "resource:external-api:api-stripe-com",
        "resource:firewall-rule:billing-egress",
        "resource:firewall-rule:edge-public-sg",
        "resource:firewall-rule:notifications-prod-sg",
        "resource:firewall-rule:orders-egress",
        "resource:firewall-rule:orders-py-prod-sg",
        "resource:firewall-rule:orders-worker-sg",
        "resource:firewall-rule:payments-sg",
        "resource:firewall-rule:prod-keycloak",
        "resource:infra:aws-platform",
        "resource:infra:infra-network",
        "resource:infra:orders-py",
        "resource:network-policy:orders-net",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(nodes, expected);
}

#[test]
fn every_edge_is_expected() {
    let built = build();
    let expected: BTreeSet<String> = [
        // Catalog dependsOn: criticality stays unknown until evidence says otherwise.
        "declared_depends_on component:orders-py -> component:keycloak - both unknown catalog",
        "declared_depends_on component:orders-py -> resource:database:redis-cache - declared_only unknown catalog",
        "declared_depends_on component:orders-py -> resource:infra:infra-network - n/a unknown catalog",
        // Named only in depgraph/soft-dependencies, by project path.
        "declared_depends_on component:orders-py -> component:notifications - both soft catalog",
        // Found in the repo: label match, or the only component of the repo.
        "declared_depends_on component:orders-py -> resource:infra:orders-py - n/a unknown static",
        "declared_depends_on component:orders-py -> resource:network-policy:orders-net - n/a unknown static",
        "declared_depends_on component:orders-py -> resource:firewall-rule:orders-egress - n/a unknown static",
        // Istio. The `orders` workload is aliased to orders-py; the soft edge keeps soft.
        "observed_calls component:orders-py -> component:keycloak prod both unknown istio",
        "observed_calls component:orders-py -> component:keycloak acc both unknown istio",
        "observed_calls component:orders-py -> component:notifications prod both soft istio",
        "observed_calls component:orders-py -> resource:external-api:api-openai-com prod observed_only unknown istio",
        // Elixir service: firewall rule endpoints and docker-compose depends_on
        // are candidates (allowed or wired, not seen). CIDRs and the
        // `postgres` compose service add nothing.
        "declared_depends_on component:billing-ex -> resource:firewall-rule:billing-egress - n/a unknown static",
        "candidate_calls component:billing-ex -> resource:external-api:api-stripe-com - candidate unknown static",
        "candidate_calls component:billing-ex -> component:notifications - candidate unknown static",
        "candidate_calls component:billing-ex -> component:orders-py - candidate unknown static",
        // AWS security groups (Terraform and CloudFormation). Each group belongs
        // to the service it is attached to (ECS, Lambda) or named after
        // (`orders-py-prod-sg`, tag `app`); a rule between two groups is a
        // candidate call. The RDS group turns into a database node.
        "declared_depends_on component:orders-py -> resource:firewall-rule:orders-py-prod-sg - n/a unknown static",
        "declared_depends_on component:orders-py -> resource:firewall-rule:orders-worker-sg - n/a unknown static",
        "declared_depends_on component:edge-proxy-rs -> resource:firewall-rule:edge-public-sg - n/a unknown static",
        "declared_depends_on component:billing-ex -> resource:firewall-rule:payments-sg - n/a unknown static",
        "declared_depends_on component:keycloak -> resource:firewall-rule:prod-keycloak - n/a unknown static",
        "declared_depends_on component:notifications -> resource:firewall-rule:notifications-prod-sg - n/a unknown static",
        "candidate_calls component:edge-proxy-rs -> component:orders-py - candidate unknown static",
        "candidate_calls component:billing-ex -> component:keycloak - candidate unknown static",
        "candidate_calls component:orders-py -> resource:database:orders-db - candidate unknown static",
        // Application source: orders-py publishes to a topic and calls billing-ex;
        // billing-ex consumes the topic. Literal names only.
        "publishes_to component:orders-py -> resource:topic:order-events - candidate unknown static",
        "consumes_from component:billing-ex -> resource:topic:order-events - candidate unknown static",
        "consumes_from component:notifications -> resource:queue:notify-jobs - candidate unknown static",
        "routes_to resource:gateway:public-edge -> component:orders-py - declared_only unknown static",
        "consumes_from resource:queue:notify-jobs -> resource:topic:order-events - candidate unknown static",
        "candidate_calls component:orders-py -> component:billing-ex - candidate unknown static",
        "candidate_calls component:orders-py -> component:notifications - candidate soft static",
        // GitLab deployments; `staging` maps to acc.
        "deployed_as component:orders-py -> deployment:orders-py@prod prod n/a unknown static",
        "deployed_as component:orders-py -> deployment:orders-py@acc acc n/a unknown static",
        "deployed_as component:keycloak -> deployment:keycloak@prod prod n/a unknown static",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    let actual = edge_lines(&built.graph);
    let missing: Vec<_> = expected.difference(&actual).collect();
    let extra: Vec<_> = actual.difference(&expected).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "missing: {missing:#?}\nextra: {extra:#?}"
    );
}

#[test]
fn no_edge_points_at_a_missing_node() {
    let built = build();
    let nodes: BTreeSet<String> = built.graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| text(&node["id"]))
        .collect();
    for edge in built.graph["edges"].as_array().unwrap() {
        assert!(nodes.contains(&text(&edge["from_id"])), "{edge}");
        assert!(nodes.contains(&text(&edge["to_id"])), "{edge}");
    }
}

#[test]
fn helm_templates_add_nothing_and_unmapped_namespaces_are_reported() {
    let built = build();
    let unresolved_json = fs::read_to_string(built.root.join("data/unresolved.json")).unwrap();
    let report: depgraph::normalize::merge::UnresolvedReport =
        serde_json::from_str(&unresolved_json).unwrap();
    let unresolved = report
        .records
        .iter()
        .map(|r| format!("{},{},{},{}", r.kind, r.name, r.reason, r.volume))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!unresolved.contains("{{"), "{unresolved}");
    assert!(
        unresolved.contains("environment,qa,namespace is not mapped,5"),
        "{unresolved}"
    );
    assert!(
        unresolved.contains("candidate_call,postgres,name is not a known component"),
        "{unresolved}"
    );
    // A security group named after a service that does not exist is not drawn.
    assert!(
        unresolved.contains("resource,legacy-batch-sg,no component matches this cloud resource"),
        "{unresolved}"
    );
    assert!(
        built.build_stderr.contains("qa (5 calls)"),
        "{}",
        built.build_stderr
    );
}

#[test]
fn impact_scores_follow_call_share_and_hardness() {
    let built = build();
    // 300 of orders' 500 prod calls go to keycloak, criticality unknown (x0.5).
    depgraph(&built.root)
        .args([
            "impact",
            "keycloak",
            "--env",
            "prod",
            "--snapshot",
            "data/snapshot.json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1\t0.300\tcomponent:orders-py"));
    // 100 of 500 go to notifications, declared soft (x0.2).
    depgraph(&built.root)
        .args([
            "impact",
            "notifications",
            "--env",
            "prod",
            "--snapshot",
            "data/snapshot.json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("1\t0.040\tcomponent:orders-py"));
}

#[test]
fn drift_lists_each_pair_once_and_skips_structural_edges() {
    let built = build();
    depgraph(&built.root)
        .args([
            "drift",
            "--env",
            "prod",
            "--snapshot",
            "data/snapshot.json",
            "--output",
            "drift",
        ])
        .assert()
        .success();
    let report = fs::read_to_string(built.root.join("drift/prod.md")).unwrap();
    let expected = "# Drift prod\n\n## both\n\n- component:orders-py -> component:keycloak calls_7d=300\n- component:orders-py -> component:notifications calls_7d=100\n\n## observed_only\n\n- component:orders-py -> resource:external-api:api-openai-com calls_7d=100\n\n## declared_only\n\n- component:orders-py -> resource:database:redis-cache\n";
    assert!(report.starts_with(expected), "{report}");
}

#[test]
fn consumers_of_the_network_stack_include_the_python_service() {
    let built = build();
    depgraph(&built.root)
        .args([
            "consumers",
            "resource:infra:infra-network",
            "--snapshot",
            "data/snapshot.json",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("component:orders-py"));
}

#[test]
fn languages_and_containers_come_from_file_names() {
    let built = build();
    let attrs = |id: &str| -> Value {
        built.graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == id)
            .unwrap()["attrs"]
            .clone()
    };
    assert_eq!(
        attrs("component:orders-py")["languages"],
        serde_json::json!(["python", "terraform"])
    );
    assert_eq!(
        attrs("component:billing-ex")["languages"],
        serde_json::json!(["elixir"])
    );
    // `target/` is build output and is ignored.
    assert_eq!(
        attrs("component:edge-proxy-rs")["languages"],
        serde_json::json!(["rust"])
    );
    assert_eq!(attrs("component:edge-proxy-rs")["container"], true);
    assert!(attrs("component:keycloak").get("languages").is_none());
    depgraph(&built.root)
        .args(["show", "billing-ex", "--snapshot", "data/snapshot.json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("  languages: elixir"));
}
