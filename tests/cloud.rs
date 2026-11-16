//! Opt-in live cloud discovery, checked on saved `aws` and `az` CLI output
//! (`tests/fixtures/cloud`) on top of the mixed landscape. Each test builds
//! the landscape with and without the cloud data and compares exactly what
//! the cloud adds, so nothing is configured per service.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn depgraph(root: &Path) -> Command {
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command.current_dir(root);
    command
}

struct Built {
    _dir: tempfile::TempDir,
    root: PathBuf,
    nodes: BTreeSet<String>,
    edges: BTreeSet<String>,
    stderr: String,
}

fn build(extra: &[&str], environments: Option<&str>) -> Built {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    for name in ["config.yaml", "pilot.yaml", "sdk_hints.yaml"] {
        fs::copy(manifest().join("config").join(name), config.join(name)).unwrap();
    }
    let landscape = manifest().join("tests/fixtures/landscape");
    fs::copy(landscape.join("aliases.yaml"), config.join("aliases.yaml")).unwrap();
    let env_text = environments.map_or_else(
        || fs::read_to_string(landscape.join("environments.yaml")).unwrap(),
        str::to_owned,
    );
    fs::write(config.join("environments.yaml"), env_text).unwrap();
    let mut collect = depgraph(&root);
    collect
        .args(["collect", "--set", "full", "--fixture"])
        .arg(&landscape)
        .arg("--prometheus-fixture")
        .arg(landscape.join("prometheus"))
        .args(["--output", "data"]);
    for arg in extra {
        collect.arg(arg.replace(
            "{cloud}",
            &manifest().join("tests/fixtures/cloud").to_string_lossy(),
        ));
    }
    collect.assert().success();
    let output = depgraph(&root).arg("build").assert().success();
    let stderr = String::from_utf8(output.get_output().stderr.clone()).unwrap();
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(root.join("data/snapshot.json")).unwrap())
            .unwrap();
    let text = |value: &Value| match value {
        Value::Null => "-".to_owned(),
        Value::String(text) => text.clone(),
        other => other.to_string(),
    };
    let nodes = graph["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| text(&node["id"]))
        .collect();
    let edges = graph["edges"]
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
                "{} {} -> {} {} {} {}",
                text(&edge["type"]),
                text(&edge["from_id"]),
                text(&edge["to_id"]),
                text(&edge["env"]),
                text(&edge["status"]),
                sources.join(",")
            )
        })
        .collect();
    for edge in graph["edges"].as_array().unwrap() {
        let ids: &BTreeSet<String> = &graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| text(&node["id"]))
            .collect();
        assert!(ids.contains(&text(&edge["to_id"])), "dangling edge {edge}");
    }
    Built {
        _dir: dir,
        root,
        nodes,
        edges,
        stderr,
    }
}

fn added(base: &BTreeSet<String>, with: &BTreeSet<String>) -> BTreeSet<String> {
    assert!(base.is_subset(with), "cloud data removed something");
    with.difference(base).cloned().collect()
}

fn set(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|item| (*item).to_owned()).collect()
}

#[test]
fn aws_account_ties_services_databases_and_real_calls() {
    let base = build(&[], None);
    let cloud = build(&["--aws-fixture", "{cloud}/aws"], None);
    assert_eq!(
        added(&base.nodes, &cloud.nodes),
        set(&[
            "resource:database:orders-aurora",
            "resource:database:sessions-redis",
            "resource:external-api:api-mailgun-net",
            "resource:firewall-rule:keycloak-prod",
            "resource:firewall-rule:notifications-ecs",
            "resource:firewall-rule:orders-py-prod-ecs-sg",
            "resource:queue:orders-events",
        ])
    );
    assert_eq!(
        added(&base.edges, &cloud.edges),
        set(&[
            // Security groups belong to the ECS service they are attached to.
            "declared_depends_on component:orders-py -> resource:firewall-rule:orders-py-prod-ecs-sg - n/a static",
            "declared_depends_on component:notifications -> resource:firewall-rule:notifications-ecs - n/a static",
            "declared_depends_on component:keycloak -> resource:firewall-rule:keycloak-prod - n/a static",
            // Allowed by a rule between two groups.
            "candidate_calls component:orders-py -> resource:database:orders-aurora - candidate static",
            "candidate_calls component:notifications -> component:keycloak - candidate static",
            "candidate_calls component:keycloak -> resource:database:sessions-redis - candidate static",
            // Seen by X-Ray. The env comes from `-prod` in the service name.
            "observed_calls component:orders-py -> resource:database:orders-aurora prod observed_only cloud",
            "observed_calls component:orders-py -> resource:external-api:api-mailgun-net prod observed_only cloud",
            "observed_calls component:orders-py -> resource:queue:orders-events prod observed_only cloud",
            "observed_calls component:notifications -> component:keycloak prod observed_only cloud",
        ])
    );
    let unresolved_json = fs::read_to_string(cloud.root.join("data/unresolved.json")).unwrap();
    let report: depgraph::normalize::merge::UnresolvedReport =
        serde_json::from_str(&unresolved_json).unwrap();
    let unresolved = report
        .records
        .iter()
        .map(|r| format!("{},{},{},{}", r.kind, r.name, r.reason, r.volume))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        unresolved.contains("resource,batch-runner-sg,no component matches this cloud resource"),
        "{unresolved}"
    );
    assert!(
        unresolved.contains("observed_call,legacy-batch-prod,source is not a known component"),
        "{unresolved}"
    );
}

#[test]
fn azure_subscription_ties_asgs_and_application_insights_calls() {
    let base = build(&[], None);
    let cloud = build(&["--azure-fixture", "{cloud}/azure"], None);
    assert_eq!(
        added(&base.nodes, &cloud.nodes),
        set(&[
            "resource:database:billing-db",
            "resource:firewall-rule:asg-storefront",
            "resource:firewall-rule:billing-ex-asg",
            "resource:queue:invoice-events",
        ])
    );
    assert_eq!(
        added(&base.edges, &cloud.edges),
        set(&[
            // An ASG belongs to its `app` tag, or else to its name.
            "declared_depends_on component:edge-proxy-rs -> resource:firewall-rule:asg-storefront - n/a static",
            "declared_depends_on component:billing-ex -> resource:firewall-rule:billing-ex-asg - n/a static",
            // An Allow rule between two ASGs; Deny rules and address prefixes add nothing.
            "candidate_calls component:edge-proxy-rs -> component:billing-ex - candidate static",
            // Application Insights dependencies.
            "observed_calls component:billing-ex -> resource:database:billing-db prod observed_only cloud",
            "observed_calls component:billing-ex -> component:notifications prod observed_only cloud",
            "observed_calls component:billing-ex -> resource:queue:invoice-events prod observed_only cloud",
            "observed_calls component:billing-ex -> resource:external-api:api-stripe-com prod observed_only cloud",
        ])
    );
    let unresolved_json = fs::read_to_string(cloud.root.join("data/unresolved.json")).unwrap();
    let report: depgraph::normalize::merge::UnresolvedReport =
        serde_json::from_str(&unresolved_json).unwrap();
    let unresolved = report
        .records
        .iter()
        .map(|r| format!("{},{},{},{}", r.kind, r.name, r.reason, r.volume))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        unresolved.contains(
            "environment,edge-proxy-rs,environment is not known; set env in the cloud config"
        ),
        "{unresolved}"
    );
}

#[test]
fn cloud_reads_are_off_by_default() {
    let text = fs::read_to_string(manifest().join("config/config.yaml")).unwrap();
    let config = depgraph::config::parse_config(&text).unwrap();
    assert!(!config.aws.enabled);
    assert!(!config.azure.enabled);
}

#[test]
fn guessed_namespaces_are_used_only_when_switched_on() {
    let landscape = manifest().join("tests/fixtures/landscape");
    let base_env = fs::read_to_string(landscape.join("environments.yaml")).unwrap();
    // Unmap `shop-acc`: its calls are left out, then guessed back in.
    let unmapped = base_env.replace("k8s_namespaces: [shop-acc]", "k8s_namespaces: []");
    let off = build(&[], Some(&unmapped));
    let on = build(&[], Some(&format!("guess_namespaces: true\n{unmapped}")));
    let acc = "observed_calls component:orders-py -> component:keycloak acc both istio";
    assert!(!off.edges.contains(acc), "{:#?}", off.edges);
    assert!(off.stderr.contains("shop-acc"), "{}", off.stderr);
    assert!(on.edges.contains(acc), "{:#?}", on.edges);
    assert!(on.stderr.contains("shop-acc -> acc"), "{}", on.stderr);
    // `qa` has no env word, so it is not guessed either way.
    let unresolved_json = fs::read_to_string(on.root.join("data/unresolved.json")).unwrap();
    let report: depgraph::normalize::merge::UnresolvedReport =
        serde_json::from_str(&unresolved_json).unwrap();
    let unresolved = report
        .records
        .iter()
        .map(|r| format!("{},{},{},{}", r.kind, r.name, r.reason, r.volume))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        unresolved.contains("environment,qa,namespace is not mapped"),
        "{unresolved}"
    );
}

#[test]
fn vpc_flow_records_become_network_evidence_not_calls() {
    let built = build(&["--aws-fixture", "{cloud}/aws-flows"], None);
    assert!(
        built
            .edges
            .contains("network_traffic component:orders-py -> component:billing-ex prod n/a cloud"),
        "{:#?}",
        built.edges
    );
    assert!(
        built
            .edges
            .contains("network_traffic component:billing-ex -> component:orders-py prod n/a cloud"),
        "{:#?}",
        built.edges
    );
    let graph: Value =
        serde_json::from_str(&fs::read_to_string(built.root.join("data/snapshot.json")).unwrap())
            .unwrap();
    let flows: Vec<_> = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["type"] == "network_traffic")
        .collect();
    assert_eq!(flows.len(), 2);
    for flow in flows {
        assert!(flow["attrs"].get("calls_7d").is_none());
        assert!(flow["attrs"].get("calls_total").is_none());
    }
}

#[cfg(unix)]
#[test]
fn incomplete_flow_scan_preserves_previous_flow_records() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("config")).unwrap();
    fs::create_dir_all(root.join("data/raw")).unwrap();
    let config = fs::read_to_string(manifest().join("config/config.yaml"))
        .unwrap()
        .replace(
            "  enabled: false\n  # profile: readonly",
            "  enabled: false\n  flow_log_group: test-flow\n  # profile: readonly",
        );
    fs::write(root.join("config/config.yaml"), config).unwrap();
    for name in [
        "pilot.yaml",
        "sdk_hints.yaml",
        "aliases.yaml",
        "environments.yaml",
    ] {
        fs::copy(
            manifest().join("config").join(name),
            root.join("config").join(name),
        )
        .unwrap();
    }
    let previous =
        "{\"collector\":\"aws_flow_logs\",\"kind\":\"network_flow_checkpoint\",\"payload\":{}}\n";
    fs::write(root.join("data/raw/aws_flow_logs.jsonl"), previous).unwrap();
    let aws = root.join("aws");
    fs::write(
        &aws,
        r##"#!/bin/sh
case "$1 $2" in
  'ec2 describe-security-groups') printf '%s' '{"SecurityGroups":[]}' ;;
  'ecs list-clusters') printf '%s' '{"clusterArns":["cluster"]}' ;;
  'ecs list-tasks') exit 1 ;;
  'ecs list-services') printf '%s' '{"serviceArns":[]}' ;;
  'ec2 describe-network-interfaces') printf '%s' '{"NetworkInterfaces":[]}' ;;
  'logs filter-log-events') printf '%s' '{"events":[]}' ;;
  *) printf '%s' '{}' ;;
esac
"##,
    )
    .unwrap();
    fs::set_permissions(&aws, fs::Permissions::from_mode(0o755)).unwrap();
    let result = depgraph(root)
        .args(["collect", "--set", "full", "--fixture"])
        .arg(manifest().join("tests/fixtures/landscape"))
        .args(["--aws", "--output", "data"])
        .env("PATH", root)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(String::from_utf8_lossy(&result.stderr).contains("previous flow records are retained"));
    assert_eq!(
        fs::read_to_string(root.join("data/raw/aws_flow_logs.jsonl")).unwrap(),
        previous
    );
}

#[cfg(unix)]
#[test]
fn flow_log_groups_are_discovered_without_a_name() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    fs::create_dir_all(root.join("config")).unwrap();
    let config = fs::read_to_string(manifest().join("config/config.yaml"))
        .unwrap()
        .replace("  flow_logs: false", "  flow_logs: true")
        .replace(
            "  # env: prod                # environment",
            "  env: prod                  # environment",
        );
    fs::write(root.join("config/config.yaml"), config).unwrap();
    for name in [
        "pilot.yaml",
        "sdk_hints.yaml",
        "aliases.yaml",
        "environments.yaml",
    ] {
        fs::copy(
            manifest().join("config").join(name),
            root.join("config").join(name),
        )
        .unwrap();
    }
    let aws = root.join("aws");
    fs::write(&aws, r##"#!/bin/sh
case "$1 $2" in
  'ec2 describe-security-groups') printf '%s' '{"SecurityGroups":[]}' ;;
  'ecs list-clusters') printf '%s' '{"clusterArns":["cluster"]}' ;;
  'ecs list-tasks') printf '%s' '{"taskArns":["task1","task2"]}' ;;
  'ecs describe-tasks') /bin/cat "$FLOW_FIXTURES/ecs-tasks.json" ;;
  'ecs list-services') printf '%s' '{"serviceArns":[]}' ;;
  'ec2 describe-flow-logs') printf '%s' '{"FlowLogs":[{"FlowLogStatus":"ACTIVE","LogDestinationType":"cloud-watch-logs","LogGroupName":"autodiscovered","LogFormat":"${version} ${account-id} ${interface-id} ${srcaddr} ${dstaddr} ${srcport} ${dstport} ${protocol} ${packets} ${bytes} ${start} ${end} ${action} ${log-status}"}]}' ;;
  'ec2 describe-network-interfaces') /bin/cat "$FLOW_FIXTURES/network-interfaces.json" ;;
  'logs filter-log-events') /bin/cat "$FLOW_FIXTURES/flow-events.json" ;;
  *) printf '%s' '{}' ;;
esac
"##).unwrap();
    fs::set_permissions(&aws, fs::Permissions::from_mode(0o755)).unwrap();
    let result = depgraph(root)
        .args(["collect", "--set", "full", "--fixture"])
        .arg(manifest().join("tests/fixtures/landscape"))
        .args(["--aws", "--output", "data"])
        .env("PATH", root)
        .env(
            "FLOW_FIXTURES",
            manifest().join("tests/fixtures/cloud/aws-flows"),
        )
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let raw = fs::read_to_string(root.join("data/raw/aws_flow_logs.jsonl")).unwrap();
    assert!(raw.contains("network_flow"), "{raw}");
    assert!(raw.contains("orders-py-prod"), "{raw}");
    assert!(raw.contains("billing-ex-prod"), "{raw}");
}
