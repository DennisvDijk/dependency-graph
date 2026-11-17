//! Read cluster identity and workload metadata from the selected kubectl context.
//! Never serialize kubeconfig, pod specs, container environment or command output.
use crate::error::Error;
use crate::raw::RawRecord;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

fn valid_uid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn hash(value: &str) -> String {
    hex::encode(Sha256::digest(value.as_bytes()))
}
fn safe_context(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty()
        || value.len() > 200
        || value.chars().any(char::is_control)
        || crate::redact::secret_kind(value).is_some()
    {
        None
    } else {
        Some(value)
    }
}
fn kubectl(args: &[&str]) -> Result<String, Error> {
    let output = crate::proc::output(Command::new("kubectl").args(args))
        .map_err(|_| Error::Config("cannot run kubectl; install it and select a context".into()))?;
    if !output.status.success() {
        return Err(Error::Config(format!(
            "kubectl {} failed; check the current context and read permissions",
            args.first().unwrap_or(&"command")
        )));
    }
    String::from_utf8(output.stdout)
        .map_err(|_| Error::Config("kubectl returned invalid text".into()))
}
/// Report the selected context without printing the kubeconfig.
///
/// # Errors
/// Returns an error when kubectl cannot identify a safe current context.
pub fn current_context() -> Result<String, Error> {
    let text = kubectl(&["config", "current-context"])?;
    safe_context(&text)
        .map(str::to_owned)
        .ok_or_else(|| Error::Config("kubectl current context is empty or invalid".into()))
}
/// Read the current cluster, without enumerating or switching contexts.
///
/// # Errors
/// Returns an error on missing access, malformed Kubernetes responses or absent cluster identity.
pub fn read_current(now: jiff::Timestamp) -> Result<Vec<RawRecord>, Error> {
    let context = current_context()?;
    read_context(&context, now)
}
/// Read a specific already-selected context without switching it.
///
/// # Errors
/// Returns an error on missing access, malformed responses or absent cluster identity.
pub fn read_context(context: &str, now: jiff::Timestamp) -> Result<Vec<RawRecord>, Error> {
    let context = safe_context(context)
        .ok_or_else(|| Error::Config("invalid Kubernetes context name".into()))?;
    let namespace = kubectl(&[
        "--context",
        context,
        "get",
        "namespace",
        "kube-system",
        "-o",
        "json",
        "--request-timeout=30s",
    ])?;
    let workloads = kubectl(&[
        "--context",
        context,
        "get",
        "deployments,statefulsets,daemonsets",
        "--all-namespaces",
        "-o",
        "json",
        "--request-timeout=30s",
    ])?;
    parse(context, &namespace, &workloads, &now.to_string())
}
/// Read a saved cluster fixture, for repeatable discovery demos and tests.
///
/// # Errors
/// Returns an error on malformed or missing fixture files.
pub fn read_fixture(dir: &Path) -> Result<Vec<RawRecord>, Error> {
    let context = std::fs::read_to_string(dir.join("context.txt"))?;
    let namespace = std::fs::read_to_string(dir.join("namespace.json"))?;
    let workloads = std::fs::read_to_string(dir.join("workloads.json"))?;
    let when = std::fs::read_to_string(dir.join("collected-at.txt"))?;
    parse(&context, &namespace, &workloads, when.trim())
}
fn parse(
    context: &str,
    namespace: &str,
    workloads: &str,
    collected_at: &str,
) -> Result<Vec<RawRecord>, Error> {
    let context = safe_context(context)
        .ok_or_else(|| Error::Config("invalid Kubernetes context name".into()))?;
    crate::models::Timestamp::parse(collected_at)
        .map_err(|_| Error::Config("invalid Kubernetes collection time".into()))?;
    let namespace: Value = serde_json::from_str(namespace)
        .map_err(|_| Error::Config("invalid Kubernetes namespace response".into()))?;
    if namespace.get("kind").and_then(Value::as_str) != Some("Namespace")
        || namespace.pointer("/metadata/name").and_then(Value::as_str) != Some("kube-system")
    {
        return Err(Error::Config(
            "Kubernetes identity response is not the kube-system namespace".into(),
        ));
    }
    let uid = namespace
        .pointer("/metadata/uid")
        .and_then(Value::as_str)
        .filter(|v| valid_uid(v))
        .ok_or_else(|| {
            Error::Config(
                "kube-system namespace has no usable UID; no cluster identity was guessed".into(),
            )
        })?;
    let cluster_id = format!("cluster:k8s-{}", hash(uid));
    let value: Value = serde_json::from_str(workloads)
        .map_err(|_| Error::Config("invalid Kubernetes workload response".into()))?;
    let items = value
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Config("Kubernetes workload response has no items list".into()))?;
    let mut records = vec![RawRecord {
        collector: "kubernetes_cluster".into(),
        kind: "cluster".into(),
        payload: json!({"id":cluster_id,"display_name":context,"last_seen":collected_at}),
    }];
    for item in items {
        let kind = item.get("kind").and_then(Value::as_str).unwrap_or("");
        if !matches!(kind, "Deployment" | "StatefulSet" | "DaemonSet") {
            continue;
        }
        let (Some(name), Some(namespace), Some(uid)) = (
            item.pointer("/metadata/name").and_then(Value::as_str),
            item.pointer("/metadata/namespace").and_then(Value::as_str),
            item.pointer("/metadata/uid").and_then(Value::as_str),
        ) else {
            continue;
        };
        if !valid_uid(uid)
            || name.is_empty()
            || namespace.is_empty()
            || name.len() > 253
            || namespace.len() > 63
            || name.chars().any(char::is_control)
            || namespace.chars().any(char::is_control)
        {
            continue;
        }
        let uid_key = format!("{cluster_id}\n{uid}");
        let workload_id = format!("workload:{}", hash(&uid_key));
        let named = [
            (
                "app.kubernetes.io/name",
                "/metadata/labels/app.kubernetes.io~1name",
            ),
            (
                "app.kubernetes.io/name",
                "/spec/template/metadata/labels/app.kubernetes.io~1name",
            ),
            ("app", "/metadata/labels/app"),
            ("app", "/spec/template/metadata/labels/app"),
        ];
        let seen: std::collections::BTreeSet<&str> = named
            .iter()
            .filter_map(|(_, path)| item.pointer(path).and_then(Value::as_str))
            .filter(|s| !s.is_empty())
            .collect();
        let label = if seen.len() == 1 {
            seen.first()
                .copied()
                .filter(|s| crate::ids::is_kebab_case(s) && crate::redact::secret_kind(s).is_none())
        } else {
            None
        };
        let label_key = label.and_then(|value| {
            named
                .iter()
                .find(|(_, path)| item.pointer(path).and_then(Value::as_str) == Some(value))
                .map(|(key, _)| *key)
        });
        records.push(RawRecord { collector: "kubernetes_cluster".into(), kind: "cluster_workload".into(), payload: json!({"id":workload_id,"cluster_id":cluster_id,"name":name,"namespace":namespace,"workload_kind":kind,"component_label":label,"component_label_key":label_key,"last_seen":collected_at}) });
    }
    Ok(records)
}
