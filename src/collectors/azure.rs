//! Opt-in live Azure reader.
//!
//! Allowed traffic: a network security group rule that lets one application
//! security group (ASG) reach another becomes a candidate call between the
//! services those ASGs belong to (their `app`/`service`/`component` tag, or
//! else their name). Each ASG is a `firewall-rule` node of its service.
//!
//! Real traffic: Application Insights dependencies (the application map),
//! read from Log Analytics workspaces. `SQL`, Cosmos DB and Redis targets are
//! databases, Service Bus a queue, Event Hubs a topic, an HTTP host inside
//! Azure a service, any other host an external API.
//!
//! Only `az ... list` and a read-only Log Analytics query are run, with the
//! login `az` already has. Names are matched loosely in the merge, and only
//! to components that exist; the rest is unresolved.

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::{Value, json};

use super::cloud::{Evidence, Seen, Target, count, kebab_name, observed, read_json, run_json};
use crate::error::Error;
use crate::raw::RawRecord;

/// The Log Analytics query for Application Insights dependencies.
pub const DEPENDENCY_QUERY: &str = "AppDependencies | where TimeGenerated > ago(7d) \
| summarize Calls = count(), Errors = countif(Success == false) by AppRoleName, Target, DependencyType";

/// VNet flow packets enriched with Azure VM identities by Traffic Analytics.
pub const VNET_FLOW_QUERY: &str = "NTANetAnalytics | where TimeGenerated > ago(6h) \
| where SubType == 'FlowLog' and FaSchemaVersion == '3' and FlowStatus == 'A' \
| where isnotempty(SrcVm) and isnotempty(DestVm) \
| project SrcVm, DestVm, SrcSubscription, DestSubscription, FlowEndTime, FlowStatus, PacketsSrcToDest, PacketsDestToSrc | take 10001";

/// What the read-only `az` calls returned for one subscription.
#[derive(Debug, Default)]
pub struct AzureSubscription {
    /// `az network asg list`.
    pub asgs: Option<Value>,
    /// `az network nsg list`.
    pub nsgs: Option<Value>,
    /// `az monitor log-analytics query` rows with [`DEPENDENCY_QUERY`], one per workspace.
    pub dependencies: Vec<Value>,
    /// NTANetAnalytics query results from discovered workspaces.
    pub network_flows: Vec<Value>,
}

/// Records from one subscription. `env` names its environment.
#[must_use]
pub fn subscription_records(subscription: &AzureSubscription, env: Option<&str>) -> Vec<RawRecord> {
    let mut records = Vec::new();
    // Azure ids are case-insensitive.
    let mut owners: BTreeMap<String, String> = BTreeMap::new();
    for asg in array(subscription.asgs.as_ref()) {
        let (Some(id), Some(name)) = (
            asg.get("id").and_then(Value::as_str),
            asg.get("name").and_then(Value::as_str),
        ) else {
            continue;
        };
        let owner = ["app", "service", "component", "App", "Service", "Component"]
            .iter()
            .find_map(|key| asg.pointer(&format!("/tags/{key}")).and_then(Value::as_str))
            .filter(|value| !value.is_empty())
            .unwrap_or(name);
        let (name, owner) = (kebab_name(name), kebab_name(owner));
        if name.is_empty() || owner.is_empty() {
            continue;
        }
        owners.insert(id.to_ascii_lowercase(), owner.clone());
        records.push(record(
            "resource",
            json!({"resource_kind": "firewall-rule", "name": name, "project_path": "azure", "component": owner, "loose": true}),
        ));
    }
    let mut pairs = std::collections::BTreeSet::new();
    for nsg in array(subscription.nsgs.as_ref()) {
        for rule in nsg
            .get("securityRules")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let field = |key: &str| {
                rule.get(key)
                    .or_else(|| rule.pointer(&format!("/properties/{key}")))
            };
            if field("access").and_then(Value::as_str) != Some("Allow") {
                continue;
            }
            let ids = |key: &str| -> Vec<String> {
                field(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|item| item.get("id").and_then(Value::as_str))
                    .filter_map(|id| owners.get(&id.to_ascii_lowercase()).cloned())
                    .collect()
            };
            for source in ids("sourceApplicationSecurityGroups") {
                for destination in ids("destinationApplicationSecurityGroups") {
                    if source != destination {
                        pairs.insert((source.clone(), destination));
                    }
                }
            }
        }
    }
    for (source, destination) in pairs {
        records.push(record(
            "candidate_call",
            json!({"source": source, "destination": destination, "loose": true}),
        ));
    }
    for rows in &subscription.dependencies {
        for row in array(Some(rows)) {
            let source = row.get("AppRoleName").and_then(Value::as_str).unwrap_or("");
            let target = row.get("Target").and_then(Value::as_str).unwrap_or("");
            let kind = row
                .get("DependencyType")
                .and_then(Value::as_str)
                .unwrap_or("");
            let Some(target) = dependency_target(kind, target) else {
                continue;
            };
            records.extend(observed(&Seen {
                source,
                target,
                calls: count(row.get("Calls")),
                errors: count(row.get("Errors")),
                env,
                window: "7d",
                evidence: Evidence::AppInsights,
            }));
        }
    }
    if !subscription.network_flows.is_empty() {
        records.push(RawRecord {
            collector: "azure_flow_logs".into(),
            kind: "network_flow_checkpoint".into(),
            payload: json!({}),
        });
    }
    for rows in &subscription.network_flows {
        records.extend(super::flow_logs::azure_flows(rows, env));
    }
    records
}

/// Map an Application Insights dependency to a target. `None` for in-process
/// calls and storage accounts.
fn dependency_target(kind: &str, target: &str) -> Option<Target> {
    let kind = kind.to_ascii_lowercase();
    let target = target.trim();
    if target.is_empty() {
        return None;
    }
    // `server.database.windows.net | ordersdb`, `sb://ns.servicebus.windows.net/ | orders`.
    let (host, detail) = match target.split_once('|') {
        Some((host, detail)) => (host.trim(), Some(detail.trim()).filter(|d| !d.is_empty())),
        None => (target, None),
    };
    let host = host
        .trim_start_matches("sb://")
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/');
    let host = host.split([':', '/']).next().unwrap_or(host);
    let first = host.split('.').next().unwrap_or(host).to_owned();
    let named =
        |kind: &'static str| Some(Target::Resource(kind, detail.unwrap_or(&first).to_owned()));
    match kind.as_str() {
        "sql" | "azure documentdb" | "azure cosmos db" | "redis" | "postgresql" | "mysql"
        | "mongodb" => named("database"),
        "azure service bus" | "queue message" => named("queue"),
        "azure event hubs" => named("topic"),
        "http" | "http (tracked component)" | "grpc" => {
            let internal = !host.contains('.')
                || [
                    ".azurewebsites.net",
                    ".azurecontainerapps.io",
                    ".svc.cluster.local",
                    ".internal",
                    ".local",
                ]
                .iter()
                .any(|suffix| host.ends_with(suffix));
            if internal {
                Some(Target::Service(first))
            } else {
                Some(Target::External(host.to_owned()))
            }
        }
        _ => None,
    }
}

fn array(value: Option<&Value>) -> Vec<&Value> {
    value
        .and_then(Value::as_array)
        .map(|items| items.iter().collect())
        .unwrap_or_default()
}

fn record(kind: &str, payload: Value) -> RawRecord {
    RawRecord {
        collector: "k8s_workloads".to_owned(),
        kind: kind.to_owned(),
        payload,
    }
}

/// Files read by [`records_from_azure_dir`], named after the CLI call.
pub const AZURE_FIXTURE_FILES: [&str; 4] = [
    "asgs.json",
    "nsgs.json",
    "dependencies.json",
    "vnet-flows.json",
];

/// Read saved `az` output from a directory (for tests and dry runs).
///
/// # Errors
///
/// Returns an error when a file exists but is not JSON.
pub fn records_from_azure_dir(dir: &Path, env: Option<&str>) -> Result<Vec<RawRecord>, Error> {
    let subscription = AzureSubscription {
        asgs: read_json(dir, "asgs.json")?,
        nsgs: read_json(dir, "nsgs.json")?,
        dependencies: read_json(dir, "dependencies.json")?.into_iter().collect(),
        network_flows: read_json(dir, "vnet-flows.json")?.into_iter().collect(),
    };
    Ok(subscription_records(&subscription, env))
}

/// How to reach Azure. Subscription and workspace ids are names, not secrets.
#[derive(Debug, Clone, Default)]
pub struct AzureLive {
    /// Subscriptions to read. Empty uses the `az` default subscription.
    pub subscriptions: Vec<String>,
    /// Log Analytics workspace ids that hold Application Insights data.
    pub workspaces: Vec<String>,
    /// Environment of these subscriptions.
    pub env: Option<String>,
    /// Discover VNet Traffic Analytics workspaces automatically.
    pub flow_logs: bool,
}

/// Read Azure with the `az` CLI. A failed dependency query is a warning.
///
/// # Errors
///
/// Returns an error when `az` is missing or the network lists cannot be read.
pub fn records_from_azure(live: &AzureLive) -> Result<(Vec<RawRecord>, Vec<String>), Error> {
    let mut records = Vec::new();
    let mut warnings = Vec::new();
    let mut flow_complete = true;
    let mut flow_issues = std::collections::BTreeSet::new();
    let mut found_flow_workspace = false;
    let subscriptions: Vec<Option<&str>> = if live.subscriptions.is_empty() {
        vec![None]
    } else {
        live.subscriptions
            .iter()
            .map(|id| Some(id.as_str()))
            .collect()
    };
    for (index, subscription) in subscriptions.iter().enumerate() {
        let call = |parts: &[&str]| -> Result<Value, Error> {
            let mut args: Vec<String> = parts.iter().map(|part| (*part).to_owned()).collect();
            args.extend(["--output".to_owned(), "json".to_owned()]);
            if let Some(id) = subscription {
                args.extend(["--subscription".to_owned(), (*id).to_owned()]);
            }
            run_json("az", &args)
        };
        let mut data = AzureSubscription {
            asgs: Some(call(&["network", "asg", "list"])?),
            nsgs: Some(call(&["network", "nsg", "list"])?),
            dependencies: Vec::new(),
            network_flows: Vec::new(),
        };
        // Workspaces are not tied to a subscription: query them once.
        if index == 0 {
            for workspace in &live.workspaces {
                match call(&[
                    "monitor",
                    "log-analytics",
                    "query",
                    "--workspace",
                    workspace,
                    "--analytics-query",
                    DEPENDENCY_QUERY,
                ]) {
                    Ok(rows) => data.dependencies.push(rows),
                    Err(err) => warnings.push(format!("azure workspace: {err}")),
                }
            }
        }
        if live.flow_logs {
            match call(&["monitor", "log-analytics", "workspace", "list"]) {
                Ok(workspaces) => {
                    if let Some(workspaces) = workspaces.as_array() {
                        for workspace in workspaces {
                            let name = workspace.get("name").and_then(Value::as_str);
                            let group = workspace.get("resourceGroup").and_then(Value::as_str);
                            let id = workspace.get("customerId").and_then(Value::as_str);
                            let (Some(name), Some(group), Some(id)) = (name, group, id) else {
                                flow_complete = false;
                                flow_issues.insert(
                                    "workspace metadata is missing; check workspace access"
                                        .to_owned(),
                                );
                                continue;
                            };
                            match call(&[
                                "monitor",
                                "log-analytics",
                                "workspace",
                                "table",
                                "list",
                                "--resource-group",
                                group,
                                "--workspace-name",
                                name,
                            ]) {
                                Ok(tables) => {
                                    let Some(tables) = tables.as_array() else {
                                        flow_complete = false;
                                        flow_issues.insert(format!("cannot read table list for workspace {name}; check Reader permissions"));
                                        continue;
                                    };
                                    if !tables.iter().any(|table| {
                                        table.get("name").and_then(Value::as_str).is_some_and(
                                            |name| name.eq_ignore_ascii_case("NTANetAnalytics"),
                                        )
                                    }) {
                                        continue;
                                    }
                                    found_flow_workspace = true;
                                    match call(&[
                                        "monitor",
                                        "log-analytics",
                                        "query",
                                        "--workspace",
                                        id,
                                        "--analytics-query",
                                        VNET_FLOW_QUERY,
                                    ]) {
                                        Ok(rows)
                                            if rows
                                                .as_array()
                                                .is_some_and(|rows| rows.len() <= 10000) =>
                                        {
                                            data.network_flows.push(rows)
                                        }
                                        Ok(_) => {
                                            flow_complete = false;
                                            flow_issues.insert(format!("workspace {name} returned more than 10000 rows or invalid results"));
                                        }
                                        Err(_) => {
                                            flow_complete = false;
                                            flow_issues.insert(format!("cannot query VNet traffic in workspace {name}; check Log Analytics read permission"));
                                        }
                                    }
                                }
                                Err(_) => {
                                    flow_complete = false;
                                    flow_issues.insert(format!("cannot list tables in workspace {name}; check Reader permissions"));
                                }
                            }
                        }
                    } else {
                        flow_complete = false;
                        flow_issues.insert("workspace discovery returned invalid data".to_owned());
                    }
                }
                Err(_) => {
                    flow_complete = false;
                    flow_issues.insert("cannot list Log Analytics workspaces; check subscription Reader permissions".to_owned());
                }
            }
            // A successful scan with no eligible workspace must replace old flow records.
            if data.network_flows.is_empty() {
                data.network_flows.push(json!([]));
            }
        }
        records.extend(subscription_records(&data, live.env.as_deref()));
    }
    if live.flow_logs && flow_complete && !found_flow_workspace {
        warnings.push("azure flow logs: no workspace with VNet Traffic Analytics found in the selected subscription; check that Traffic Analytics is enabled".into());
    }
    if live.flow_logs && !flow_complete {
        for issue in flow_issues {
            warnings.push(format!("azure flow logs: {issue}"));
        }
        records.retain(|record| record.collector != "azure_flow_logs");
        warnings.push(
            "azure flow logs: an input was incomplete; any previous flow records are retained"
                .into(),
        );
    }
    Ok((records, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dependency_targets_are_classified() {
        assert_eq!(
            dependency_target("SQL", "orders-sql.database.windows.net | orders-db"),
            Some(Target::Resource("database", "orders-db".to_owned()))
        );
        assert_eq!(
            dependency_target(
                "Azure Service Bus",
                "sb://shop.servicebus.windows.net/ | order-events"
            ),
            Some(Target::Resource("queue", "order-events".to_owned()))
        );
        assert_eq!(
            dependency_target("HTTP", "payments-prod.azurewebsites.net"),
            Some(Target::Service("payments-prod".to_owned()))
        );
        assert_eq!(
            dependency_target("HTTP", "api.stripe.com:443"),
            Some(Target::External("api.stripe.com".to_owned()))
        );
        assert_eq!(dependency_target("InProc", "work"), None);
    }

    #[test]
    fn asg_rules_become_candidates() {
        let id = |name: &str| {
            format!(
                "/subscriptions/x/resourceGroups/rg/providers/Microsoft.Network/applicationSecurityGroups/{name}"
            )
        };
        let subscription = AzureSubscription {
            asgs: Some(json!([
                {"id": id("asg-web"), "name": "asg-web", "tags": {"app": "storefront"}},
                {"id": id("orders-prod-asg"), "name": "orders-prod-asg"}
            ])),
            nsgs: Some(json!([{"securityRules": [
                {"access": "Allow", "direction": "Inbound",
                 "sourceApplicationSecurityGroups": [{"id": id("ASG-WEB")}],
                 "destinationApplicationSecurityGroups": [{"id": id("orders-prod-asg")}]},
                {"access": "Deny",
                 "sourceApplicationSecurityGroups": [{"id": id("orders-prod-asg")}],
                 "destinationApplicationSecurityGroups": [{"id": id("asg-web")}]}
            ]}])),
            dependencies: Vec::new(),
            network_flows: Vec::new(),
        };
        let records = subscription_records(&subscription, None);
        let candidates: Vec<String> = records
            .iter()
            .filter(|record| record.kind == "candidate_call")
            .map(|record| {
                format!(
                    "{}->{}",
                    record.payload["source"], record.payload["destination"]
                )
            })
            .collect();
        assert_eq!(candidates, vec![r#""storefront"->"orders-prod-asg""#]);
        assert_eq!(records.iter().filter(|r| r.kind == "resource").count(), 2);
    }
}
