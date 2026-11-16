//! Conservative mapping of default-format VPC flow logs to running ECS tasks.
//! A flow is packet evidence, not proof of a request or its initiator.
use super::aws::AwsAccount;
use crate::raw::RawRecord;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Debug)]
struct Task {
    name: String,
    eni: String,
    started: i64,
}

fn timestamp(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_f64().map(|v| v as i64))
        .or_else(|| {
            value.as_str().and_then(|v| {
                chrono::DateTime::parse_from_rfc3339(v)
                    .ok()
                    .map(|t| t.timestamp())
            })
        })
}

/// Map traffic only when both IPs unambiguously belong to current service tasks
/// in the log interface's VPC for the entire reported flow interval.
pub(super) fn aws_flows(account: &AwsAccount, env: Option<&str>) -> Vec<RawRecord> {
    let mut interfaces = BTreeMap::<String, String>::new();
    if let Some(items) = account
        .network_interfaces
        .as_ref()
        .and_then(|v| v.get("NetworkInterfaces"))
        .and_then(Value::as_array)
    {
        for item in items {
            if let (Some(id), Some(vpc)) = (
                item.get("NetworkInterfaceId").and_then(Value::as_str),
                item.get("VpcId").and_then(Value::as_str),
            ) {
                interfaces.insert(id.to_owned(), vpc.to_owned());
            }
        }
    }
    let mut ips: BTreeMap<(String, String), Vec<Task>> = BTreeMap::new();
    for batch in &account.ecs_tasks {
        for task in batch
            .get("tasks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if task.get("lastStatus").and_then(Value::as_str) != Some("RUNNING") {
                continue;
            }
            let Some(name) = task
                .get("group")
                .and_then(Value::as_str)
                .and_then(|v| v.strip_prefix("service:"))
            else {
                continue;
            };
            let Some(started) = task.get("startedAt").and_then(timestamp) else {
                continue;
            };
            for attachment in task
                .get("attachments")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if attachment.get("type").and_then(Value::as_str) != Some("ElasticNetworkInterface")
                {
                    continue;
                }
                let mut eni = None;
                let mut ip = None;
                for detail in attachment
                    .get("details")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    match detail.get("name").and_then(Value::as_str) {
                        Some("networkInterfaceId") => {
                            eni = detail.get("value").and_then(Value::as_str)
                        }
                        Some("privateIPv4Address") => {
                            ip = detail.get("value").and_then(Value::as_str)
                        }
                        _ => {}
                    }
                }
                if let (Some(eni), Some(ip)) = (eni, ip)
                    && let Some(vpc) = interfaces.get(eni)
                {
                    ips.entry((vpc.clone(), ip.to_owned()))
                        .or_default()
                        .push(Task {
                            name: name.to_owned(),
                            eni: eni.to_owned(),
                            started,
                        });
                }
            }
        }
    }
    let mut latest = BTreeMap::<(String, String), i64>::new();
    for event in account
        .flow_events
        .as_ref()
        .and_then(|v| v.get("events"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(line) = event.get("message").and_then(Value::as_str) else {
            continue;
        };
        let fields: Vec<_> = line.split_whitespace().collect();
        // Only the documented default 14-field format, accepted traffic and complete records.
        if fields.len() != 14 || fields[0] != "2" || fields[12] != "ACCEPT" || fields[13] != "OK" {
            continue;
        }
        let (Ok(start), Ok(end)) = (fields[10].parse::<i64>(), fields[11].parse::<i64>()) else {
            continue;
        };
        if end < start {
            continue;
        }
        let Some(vpc) = interfaces.get(fields[2]) else {
            continue;
        };
        let Some(src) = ips.get(&(vpc.clone(), fields[3].to_owned())) else {
            continue;
        };
        let Some(dst) = ips.get(&(vpc.clone(), fields[4].to_owned())) else {
            continue;
        };
        if src.len() != 1 || dst.len() != 1 {
            continue;
        }
        let (src, dst) = (&src[0], &dst[0]);
        if start < src.started
            || start < dst.started
            || (fields[2] != src.eni && fields[2] != dst.eni)
            || src.name == dst.name
        {
            continue;
        }
        let key = (src.name.clone(), dst.name.clone());
        latest
            .entry(key)
            .and_modify(|t| *t = (*t).max(end))
            .or_insert(end);
    }
    latest.into_iter().filter_map(|((src, dst), time)| {
        let stamp = chrono::DateTime::from_timestamp(time, 0)?.to_rfc3339();
        Some(RawRecord { collector: "aws_flow_logs".into(), kind: "network_flow".into(), payload: json!({
            "source": super::cloud::kebab_name(&src), "destination": super::cloud::kebab_name(&dst),
            "env": env.unwrap_or(""), "last_seen": stamp,
        }) })
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn aws_flow_is_not_a_call_and_ignores_ambiguous_or_rejected_traffic() {
        let mut account = AwsAccount {
            network_interfaces: Some(
                json!({"NetworkInterfaces":[{"NetworkInterfaceId":"eni-a","VpcId":"vpc-a"},{"NetworkInterfaceId":"eni-b","VpcId":"vpc-a"}]}),
            ),
            ..Default::default()
        };
        account.ecs_tasks.push(json!({"tasks":[
            {"lastStatus":"RUNNING","group":"service:orders-py","startedAt":1000,"attachments":[{"type":"ElasticNetworkInterface","details":[{"name":"networkInterfaceId","value":"eni-a"},{"name":"privateIPv4Address","value":"10.0.0.1"}]}]},
            {"lastStatus":"RUNNING","group":"service:billing-ex","startedAt":1000,"attachments":[{"type":"ElasticNetworkInterface","details":[{"name":"networkInterfaceId","value":"eni-b"},{"name":"privateIPv4Address","value":"10.0.0.2"}]}]}
        ]}));
        account.flow_events = Some(json!({"events":[
            {"message":"2 123 eni-a 10.0.0.1 10.0.0.2 111 443 6 2 40 1001 1020 ACCEPT OK"},
            {"message":"2 123 eni-a 10.0.0.2 10.0.0.1 443 111 6 2 40 1001 1020 REJECT OK"},
            {"message":"2 123 eni-a 10.0.0.2 10.0.0.1 443 111 6 2 40 999 1020 ACCEPT OK"},
            {"message":"2 123 eni-a 10.0.0.1 10.0.0.3 111 443 6 2 40 1001 1020 ACCEPT OK"}
        ]}));
        let records = aws_flows(&account, Some("prod"));
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].kind, "network_flow");
        assert_eq!(records[0].payload["source"], "orders-py");
        assert_eq!(records[0].payload["destination"], "billing-ex");
        assert!(records[0].payload.get("calls").is_none());
        let duplicate = account.ecs_tasks[0]["tasks"][0].clone();
        account.ecs_tasks[0]["tasks"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(aws_flows(&account, Some("prod")).is_empty());
    }
}
