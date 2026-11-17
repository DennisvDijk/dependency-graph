//! Turn a fixture directory or a live GitLab selection into raw records.
//!
//! Fixture mode does not open a socket. Live mode uses the GitLab client.
//! `kubernetes.live_read` is the only path that may call kubectl, and only
//! when a [`LiveRead`] implementation is passed in.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::gitlab::{GitLabClient, ProjectFilter};
use crate::normalize::merge::version_rank;
use crate::raw::RawRecord;
use crate::repo_scan::kubernetes_app_names;

/// Which collectors a run includes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatherSet {
    /// Inventory and GitLab deployments.
    Fast,
    /// Fast, plus manifests and IaC from the repositories.
    Full,
}

/// Optional read-only cluster access. The default build does not call this.
pub trait LiveRead {
    /// List workloads as raw records.
    ///
    /// # Errors
    ///
    /// Returns an error when the cluster cannot be read. Do not include
    /// kubeconfig contents in the error.
    fn list_workloads(&self) -> Result<Vec<RawRecord>, Error>;
}

/// kubectl read. Used only when `kubernetes.live_read` is true.
#[derive(Debug, Clone)]
pub struct KubectlRead {
    /// Path passed to `kubectl --kubeconfig`. Not logged.
    pub kubeconfig: PathBuf,
}

impl LiveRead for KubectlRead {
    fn list_workloads(&self) -> Result<Vec<RawRecord>, Error> {
        let output = std::process::Command::new("kubectl")
            .args([
                "--kubeconfig",
                &self.kubeconfig.to_string_lossy(),
                "get",
                "deploy,networkpolicy",
                "-A",
                "-o",
                "json",
            ])
            .output()
            .map_err(|_| {
                Error::Config("kubectl failed. The kubeconfig contents were not logged.".to_owned())
            })?;
        if !output.status.success() {
            return Err(Error::Config(
                "kubectl get failed. The kubeconfig contents were not logged.".to_owned(),
            ));
        }
        let text = String::from_utf8_lossy(&output.stdout);
        Ok(records_from_kubectl_json(&text))
    }
}

/// Read a fixture directory. Layout: `fixture.yaml`, `deployments.yaml`, and one directory per project.
///
/// # Errors
///
/// Returns an error when the fixture files are missing or malformed.
pub fn records_from_fixture(dir: &Path, set: GatherSet) -> Result<Vec<RawRecord>, Error> {
    let fixture: FixtureFile = yaml_serde::from_str(&fs::read_to_string(dir.join("fixture.yaml"))?)
        .map_err(|err| Error::Config(format!("fixture.yaml: {err}")))?;
    let deployments: Vec<FixtureDeployment> =
        yaml_serde::from_str(&fs::read_to_string(dir.join("deployments.yaml"))?)
            .map_err(|err| Error::Config(format!("deployments.yaml: {err}")))?;
    let mut records = Vec::new();
    for project in &fixture.projects {
        let root = dir.join(&project.dir);
        if let Some(text) = read_if_exists(&root.join("catalog-info.yaml")) {
            records.extend(records_from_catalog(&project.path, &text));
        }
        if set == GatherSet::Full {
            records.extend(records_from_tree(&project.path, &root)?);
        }
    }
    records.extend(records_from_deployment_list(&deployments));
    Ok(records)
}

/// Which GitLab projects a live collect reads.
///
/// Empty `groups` and `projects` scan every project the token can see.
/// The same rules as `depgraph inventory` apply, including the explicit-project
/// opt-in and the archived-project warnings.
pub struct GitlabScope<'a> {
    /// Group or subgroup paths.
    pub groups: &'a [String],
    /// Explicit project paths.
    pub projects: &'a [String],
    /// Filters for the group or full-scan list.
    pub filter: &'a ProjectFilter,
    /// Clock for `min_last_activity`.
    pub now: jiff::Timestamp,
}

/// Read GitLab. `live` is consulted only when `live_read` is true.
///
/// The returned warnings are archived explicit projects that were skipped.
///
/// # Errors
///
/// Returns an error when the API call fails.
pub async fn records_from_gitlab(
    client: &GitLabClient,
    scope: GitlabScope<'_>,
    catalog_file: &str,
    set: GatherSet,
    live: Option<&dyn LiveRead>,
) -> Result<(Vec<RawRecord>, Vec<String>), Error> {
    let (projects, warnings) = client
        .select_scope(scope.groups, scope.projects, scope.filter, scope.now)
        .await?;
    let mut records = Vec::new();
    for project in projects {
        let branch = project
            .default_branch
            .clone()
            .unwrap_or_else(|| "main".to_owned());
        if let Some(text) = client.file_text(project.id, catalog_file, &branch).await? {
            records.extend(records_from_catalog(&project.path_with_namespace, &text));
        }
        let deployments = client.deployments(project.id).await?;
        records.extend(records_from_gitlab_deployments(
            &project.path_with_namespace,
            &deployments,
        ));
        if set == GatherSet::Full {
            let tree = client.repository_tree(project.id, &branch).await?;
            let paths: Vec<String> = tree
                .iter()
                .filter(|item| item.item_type == "blob")
                .map(|item| item.path.clone())
                .collect();
            records.extend(project_facts(&project.path_with_namespace, &paths));
            let mut sources: Vec<(String, String)> = Vec::new();
            for path in super::source::select_paths(&paths) {
                if let Some(text) = client.file_text(project.id, &path, &branch).await? {
                    sources.push((path, text));
                }
            }
            records.extend(super::source::source_records(
                &project.path_with_namespace,
                &sources,
            ));
            let mut terraform: Vec<(String, String)> = Vec::new();
            for item in manifest_items(tree) {
                if let Some(text) = client.file_text(project.id, &item.path, &branch).await? {
                    if item.path.ends_with(".tf") {
                        terraform.push((item.path.clone(), text.clone()));
                    }
                    records.extend(records_from_file(
                        &project.path_with_namespace,
                        &item.path,
                        &text,
                    ));
                }
            }
            records.extend(super::aws::terraform_records_from_files(
                &project.path_with_namespace,
                &terraform,
            ));
        }
    }
    if let Some(live) = live {
        records.extend(live.list_workloads()?);
    }
    Ok((records, warnings))
}

/// Manifest files read per project in a full run. The cap applies after the
/// filter, so a large source tree (Python, Terraform, monorepos) does not push
/// its deploy manifests past it.
const MAX_MANIFESTS_PER_PROJECT: usize = 200;

/// Blobs worth reading, capped after filtering.
fn manifest_items(tree: Vec<crate::gitlab::TreeItem>) -> Vec<crate::gitlab::TreeItem> {
    tree.into_iter()
        .filter(|item| item.item_type == "blob" && interesting_manifest(&item.path))
        .take(MAX_MANIFESTS_PER_PROJECT)
        .collect()
}

fn records_from_catalog(project_path: &str, yaml: &str) -> Vec<RawRecord> {
    crate::catalog::yaml_documents(yaml)
        .into_iter()
        .flat_map(|doc| records_from_catalog_doc(project_path, doc))
        .collect()
}

fn records_from_catalog_doc(project_path: &str, yaml: &str) -> Vec<RawRecord> {
    let value: Value = yaml_serde::from_str(yaml).unwrap_or(Value::Null);
    let kind = value
        .get("kind")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let name = value
        .pointer("/metadata/name")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    if name.is_empty() {
        return Vec::new();
    }
    let collector = "gitlab_inventory".to_owned();
    if kind == "Group" {
        let parent = value
            .pointer("/spec/parent")
            .and_then(|item| item.as_str())
            .map(owner_name)
            .unwrap_or_default();
        return vec![record(
            &collector,
            "team",
            json!({"name": owner_name(name), "parent": parent, "project_path": project_path}),
        )];
    }
    if kind == "Resource" {
        let resource_kind = value
            .pointer("/spec/type")
            .and_then(|item| item.as_str())
            .unwrap_or("other");
        return vec![record(
            &collector,
            "resource",
            json!({"resource_kind": resource_kind, "name": name, "project_path": project_path}),
        )];
    }
    let depends_on = value
        .pointer("/spec/dependsOn")
        .cloned()
        .unwrap_or(json!([]));
    let soft = annotation_list(&value, "depgraph/soft-dependencies");
    let heuristics = annotation_list(&value, "depgraph/heuristics");
    let owner = value
        .pointer("/spec/owner")
        .and_then(|item| item.as_str())
        .map(owner_name)
        .unwrap_or_default();
    let lifecycle = value
        .pointer("/spec/lifecycle")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let system = value
        .pointer("/spec/system")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    vec![record(
        &collector,
        "component",
        json!({
            "name": name,
            "depends_on": depends_on,
            "project_path": project_path,
            "soft_dependencies": soft,
            "heuristics": heuristics,
            "owner": owner,
            "system": system,
            "lifecycle": lifecycle,
        }),
    )]
}

/// A Backstage entity reference reduced to its name:
/// `group:default/team-a` and `Team A` both become kebab-case names.
/// A name that is not usable stays as written, so the owner is never lost.
fn owner_name(reference: &str) -> String {
    let name = reference.rsplit('/').next().unwrap_or(reference);
    let name = name.split_once(':').map_or(name, |(_, rest)| rest).trim();
    let kebab = super::cloud::kebab_name(name);
    if kebab.is_empty() {
        name.to_owned()
    } else {
        kebab
    }
}

fn annotation_list(value: &Value, key: &str) -> Vec<String> {
    let key = key.replace("~", "~0").replace("/", "~1");
    value
        .pointer(&format!("/metadata/annotations/{key}"))
        .and_then(|item| item.as_str())
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_owned)
        .collect()
}

fn records_from_tree(project_path: &str, root: &Path) -> Result<Vec<RawRecord>, Error> {
    let mut records = Vec::new();
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    let mut paths = Vec::new();
    collect_paths(root, root, &mut paths)?;
    records.extend(project_facts(project_path, &paths));
    let mut sources: Vec<(String, String)> = Vec::new();
    for path in super::source::select_paths(&paths) {
        if let Ok(text) = fs::read_to_string(root.join(&path)) {
            sources.push((path, text));
        }
    }
    records.extend(super::source::source_records(project_path, &sources));
    let mut terraform: Vec<(String, String)> = Vec::new();
    for (path, text) in files {
        if path.ends_with(".tf") {
            terraform.push((path.clone(), text.clone()));
        }
        records.extend(records_from_file(project_path, &path, &text));
    }
    records.extend(super::aws::terraform_records_from_files(
        project_path,
        &terraform,
    ));
    Ok(records)
}

fn records_from_file(project_path: &str, path: &str, text: &str) -> Vec<RawRecord> {
    let mut records = Vec::new();
    let collector = "k8s_workloads";
    if path.ends_with(".tf") {
        let name = project_path.rsplit('/').next().unwrap_or(project_path);
        if crate::ids::is_kebab_case(name) {
            records.push(record(
                collector,
                "resource",
                json!({"resource_kind": "infra", "name": name, "project_path": project_path, "component": name}),
            ));
        }
        return records;
    }
    // Helm and other Go templates are not rendered; their names are not names.
    if text.contains("{{") {
        return records;
    }
    if is_compose_file(path) {
        return compose_records(text);
    }
    if text.contains("AWS::") && text.contains("Resources:") {
        return super::aws::cloudformation_records(project_path, text);
    }
    for doc in text.split("\n---\n") {
        let kind = yaml_kind(doc);
        let name = yaml_metadata_name(doc).unwrap_or_default();
        let labels = kubernetes_app_names(doc);
        let component = labels.first().cloned();
        match kind.as_deref() {
            Some("NetworkPolicy") if crate::ids::is_kebab_case(&name) => {
                records.push(record(
                    collector,
                    "resource",
                    json!({"resource_kind": "network-policy", "name": name, "project_path": project_path, "component": component}),
                ));
            }
            Some("FirewallRule") if crate::ids::is_kebab_case(&name) => {
                let (sources, destinations) = firewall_endpoints(doc);
                let owner = sources.first().cloned().or(component);
                records.push(record(
                    collector,
                    "resource",
                    json!({"resource_kind": "firewall-rule", "name": name, "project_path": project_path, "component": owner}),
                ));
                for source in &sources {
                    for (destination, external) in &destinations {
                        if source != destination {
                            records.push(record(
                                collector,
                                "candidate_call",
                                json!({"source": source, "destination": destination, "external": external}),
                            ));
                        }
                    }
                }
            }
            Some("Deployment" | "StatefulSet" | "DaemonSet" | "Job" | "CronJob" | "Rollout") => {
                let value: serde_json::Value =
                    yaml_serde::from_str(doc).unwrap_or(serde_json::Value::Null);
                // `app.kubernetes.io/name` first; then the plain `app` label and
                // the workload's own name, which most manifests use instead.
                if let Some(component) = component.or_else(|| workload_name(&value)) {
                    records.push(record(
                        collector,
                        "component",
                        json!({"name": component, "depends_on": [], "project_path": project_path}),
                    ));
                    for destination in env_service_hosts(&value) {
                        if destination != component {
                            records.push(record(
                                collector,
                                "candidate_call",
                                json!({"source": component, "destination": destination, "loose": true}),
                            ));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    records.extend(candidate_records(text));
    records.extend(gateway_records(text));
    records
}

/// The name of a workload: the `app` label of its pods or of the object, or
/// else the object's own name. Only kebab-case names count.
fn workload_name(value: &serde_json::Value) -> Option<String> {
    [
        "/spec/template/metadata/labels/app",
        "/metadata/labels/app",
        "/metadata/name",
    ]
    .iter()
    .filter_map(|pointer| value.pointer(pointer).and_then(|v| v.as_str()))
    .find(|name| crate::ids::is_kebab_case(name))
    .map(str::to_owned)
}

/// Services named by environment values of the form `host:port`
/// (`productcatalogservice:3550`, `redis-cart.shop.svc:6379`). A candidate,
/// not an observation: the setting says where a client would connect.
fn env_service_hosts(value: &serde_json::Value) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    let containers = value
        .pointer("/spec/template/spec/containers")
        .or_else(|| value.pointer("/spec/jobTemplate/spec/template/spec/containers"))
        .and_then(|v| v.as_array());
    for container in containers.into_iter().flatten() {
        for entry in container
            .get("env")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let Some(text) = entry.get("value").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some((host, port)) = text.rsplit_once(':') else {
                continue;
            };
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                continue;
            }
            let name = host.split('.').next().unwrap_or(host);
            if name != "localhost"
                && crate::ids::is_kebab_case(name)
                && !found.iter().any(|existing| existing == name)
            {
                found.push(name.to_owned());
            }
        }
    }
    found
}

/// Gateways read from Kubernetes `Ingress` and Gateway API `HTTPRoute` objects.
///
/// Each becomes a `gateway` node with a `routes_to` edge to every service it
/// forwards to. The manifest is a declaration, so these are not candidates.
#[must_use]
pub fn gateway_records(text: &str) -> Vec<RawRecord> {
    let mut records = Vec::new();
    for doc in text.split("\n---\n") {
        let Ok(value) = yaml_serde::from_str::<serde_json::Value>(doc) else {
            continue;
        };
        let kind = value.get("kind").and_then(|v| v.as_str()).unwrap_or("");
        let Some(name) = value
            .pointer("/metadata/name")
            .and_then(|v| v.as_str())
            .filter(|name| crate::ids::is_kebab_case(name))
        else {
            continue;
        };
        let mut backends: Vec<String> = Vec::new();
        let mut hosts: Vec<String> = Vec::new();
        match kind {
            "Ingress" => {
                let spec = value.get("spec").unwrap_or(&serde_json::Value::Null);
                if let Some(name) = spec
                    .pointer("/defaultBackend/service/name")
                    .and_then(|v| v.as_str())
                {
                    backends.push(name.to_owned());
                }
                for rule in spec
                    .get("rules")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    if let Some(host) = rule.get("host").and_then(|v| v.as_str()) {
                        hosts.push(host.to_owned());
                    }
                    for path in rule
                        .pointer("/http/paths")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                    {
                        if let Some(name) = path
                            .pointer("/backend/service/name")
                            .and_then(|v| v.as_str())
                        {
                            backends.push(name.to_owned());
                        }
                    }
                }
            }
            "HTTPRoute" | "GRPCRoute" => {
                for host in value
                    .pointer("/spec/hostnames")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                {
                    hosts.push(host.to_owned());
                }
                for rule in value
                    .pointer("/spec/rules")
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                {
                    for backend in rule
                        .get("backendRefs")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                    {
                        if let Some(name) = backend.get("name").and_then(|v| v.as_str()) {
                            backends.push(name.to_owned());
                        }
                    }
                }
            }
            _ => continue,
        }
        hosts.sort();
        hosts.dedup();
        for host in hosts.iter().filter(|h| !h.contains('*')) {
            records.push(record(
                "k8s_workloads",
                "gateway_host",
                json!({"gateway": name, "host": host}),
            ));
        }
        backends.sort();
        backends.dedup();
        for backend in backends {
            if crate::ids::is_kebab_case(&backend) {
                records.push(record(
                    "k8s_workloads",
                    "resource_link",
                    json!({"edge": "routes_to", "from_kind": "gateway", "from": name,
                           "to_kind": "component", "to": backend}),
                ));
            }
        }
    }
    records
}

/// Candidate calls from Istio and NetworkPolicy manifests.
///
/// These are candidates, never observations. A host inside the cluster becomes
/// the service's first label. Other hosts become a kebab-case name.
#[must_use]
pub fn candidate_records(text: &str) -> Vec<RawRecord> {
    let mut records = Vec::new();
    for doc in text.split("\n---\n") {
        let kind = yaml_kind(doc).unwrap_or_default();
        if !matches!(
            kind.as_str(),
            "VirtualService" | "DestinationRule" | "AuthorizationPolicy" | "NetworkPolicy"
        ) {
            continue;
        }
        let source = if kind == "NetworkPolicy" {
            kubernetes_app_names(doc)
                .into_iter()
                .find(|name| crate::ids::is_kebab_case(name))
        } else {
            manifest_source(doc)
        };
        let Some(source) = source else {
            continue;
        };
        let mut destinations = match kind.as_str() {
            "AuthorizationPolicy" => principals(doc),
            "NetworkPolicy" => extra_app_names(doc, &source),
            _ => hosts(doc),
        };
        destinations.sort();
        destinations.dedup();
        for destination in destinations {
            if destination == source || !crate::ids::is_kebab_case(&destination) {
                continue;
            }
            records.push(record(
                "k8s_workloads",
                "candidate_call",
                json!({"source": source, "destination": destination}),
            ));
        }
    }
    records
}

fn manifest_source(doc: &str) -> Option<String> {
    yaml_metadata_name(doc)
        .filter(|name| crate::ids::is_kebab_case(name))
        .or_else(|| {
            kubernetes_app_names(doc)
                .into_iter()
                .find(|name| crate::ids::is_kebab_case(name))
        })
}

fn hosts(doc: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in doc.lines() {
        let trimmed = line.trim().trim_start_matches("- ").trim();
        let Some(value) = trimmed.strip_prefix("host:") else {
            continue;
        };
        names.push(host_to_name(value.trim().trim_matches(['"', '\''])));
    }
    names
}

fn host_to_name(host: &str) -> String {
    let host = host.split(':').next().unwrap_or(host);
    if host.ends_with(".svc.cluster.local") {
        return host.split('.').next().unwrap_or(host).to_owned();
    }
    if crate::ids::is_kebab_case(host) {
        return host.to_owned();
    }
    let mut out = String::new();
    for byte in host.bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push((byte as char).to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn principals(doc: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in doc.lines() {
        let trimmed = line.trim().trim_start_matches("- ").trim();
        let Some(value) = trimmed
            .strip_prefix("principals:")
            .or_else(|| trimmed.strip_prefix("principal:"))
        else {
            continue;
        };
        let value = value.trim().trim_matches(['"', '\'']);
        if value.is_empty() {
            continue;
        }
        let name = value.rsplit('/').next().unwrap_or(value);
        names.push(name.to_owned());
    }
    names
}

fn extra_app_names(doc: &str, source: &str) -> Vec<String> {
    kubernetes_app_names(doc)
        .into_iter()
        .filter(|name| name != source && crate::ids::is_kebab_case(name))
        .collect()
}

fn records_from_deployment_list(items: &[FixtureDeployment]) -> Vec<RawRecord> {
    items
        .iter()
        .map(|item| {
            let (rank, version) = version_rank(&item.reference);
            record(
                "gitlab_deployments",
                "gitlab_deployment",
                json!({
                    "project_path": item.project,
                    "component": item.component,
                    "environment": item.environment,
                    "version": version,
                    "rank": rank,
                    "created_at": item.created_at,
                }),
            )
        })
        .collect()
}

fn records_from_gitlab_deployments(
    project_path: &str,
    deployments: &[crate::gitlab::GitlabDeployment],
) -> Vec<RawRecord> {
    let component = project_path.rsplit('/').next().unwrap_or(project_path);
    deployments
        .iter()
        .map(|item| {
            let reference = if item.reference.is_empty() {
                item.sha.clone()
            } else {
                item.reference.clone()
            };
            let (rank, version) = version_rank(&reference);
            record(
                "gitlab_deployments",
                "gitlab_deployment",
                json!({
                    "project_path": project_path,
                    "component": component,
                    "environment": item.environment,
                    "version": version,
                    "rank": rank,
                    "created_at": item.created_at,
                }),
            )
        })
        .collect()
}

fn records_from_kubectl_json(text: &str) -> Vec<RawRecord> {
    let value: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let Some(items) = value.get("items").and_then(|item| item.as_array()) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for item in items {
        let kind = item
            .get("kind")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let name = item
            .pointer("/metadata/name")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let component = item
            .pointer("/metadata/labels/app.kubernetes.io~1name")
            .and_then(|value| value.as_str())
            .map(str::to_owned);
        if kind == "NetworkPolicy" && crate::ids::is_kebab_case(name) {
            records.push(record(
                "k8s_workloads",
                "resource",
                json!({"resource_kind": "network-policy", "name": name, "project_path": "live", "component": component}),
            ));
        }
    }
    records
}

/// `spec.from`/`source(s)` and `spec.to`/`destination(s)` of a firewall rule.
///
/// Entries are names, hosts, or maps with `app`, `name`, `component`,
/// `service` or `host`. CIDRs and wildcards are skipped. A destination is
/// external when it is a host outside the cluster.
fn firewall_endpoints(doc: &str) -> (Vec<String>, Vec<(String, bool)>) {
    let Ok(value) = yaml_serde::from_str::<Value>(doc) else {
        return (Vec::new(), Vec::new());
    };
    let spec = value.get("spec").unwrap_or(&Value::Null);
    let pick = |keys: &[&str]| -> Vec<String> {
        let mut out = Vec::new();
        for key in keys {
            match spec.get(*key) {
                Some(Value::Array(items)) => out.extend(items.iter().filter_map(endpoint_text)),
                Some(item) => out.extend(endpoint_text(item)),
                None => {}
            }
        }
        out
    };
    let mut sources: Vec<String> = pick(&["from", "source", "sources"])
        .iter()
        .filter_map(|entry| match endpoint_name(entry) {
            Some((name, false)) => Some(name),
            _ => None,
        })
        .collect();
    sources.dedup();
    let mut destinations: Vec<(String, bool)> = pick(&["to", "destination", "destinations"])
        .iter()
        .filter_map(|entry| endpoint_name(entry))
        .collect();
    destinations.sort();
    destinations.dedup();
    (sources, destinations)
}

fn endpoint_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Object(map) => ["app", "component", "service", "name", "host"]
            .iter()
            .find_map(|key| map.get(*key).and_then(Value::as_str))
            .map(str::to_owned),
        _ => None,
    }
}

/// `(name, external)` for a firewall endpoint, or `None` for CIDRs and wildcards.
fn endpoint_name(entry: &str) -> Option<(String, bool)> {
    let entry = entry.trim();
    if entry.is_empty()
        || entry.contains('*')
        || entry.contains('/')
        || entry
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.' || c == ':')
    {
        return None;
    }
    let host = entry.split(':').next().unwrap_or(entry);
    let external = host.contains('.') && !host.ends_with(".svc.cluster.local");
    let name = host_to_name(entry);
    crate::ids::is_kebab_case(&name).then_some((name, external))
}

fn is_compose_file(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    matches!(
        file.as_str(),
        "docker-compose.yml" | "docker-compose.yaml" | "compose.yml" | "compose.yaml"
    ) || (file.starts_with("docker-compose.")
        && (file.ends_with(".yml") || file.ends_with(".yaml")))
}

/// Candidate calls from docker-compose `depends_on`. Only pairs where both
/// services are known components end up in the graph.
fn compose_records(text: &str) -> Vec<RawRecord> {
    let Ok(value) = yaml_serde::from_str::<Value>(text) else {
        return Vec::new();
    };
    let Some(services) = value.get("services").and_then(Value::as_object) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for (service, body) in services {
        let deps: Vec<String> = match body.get("depends_on") {
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            Some(Value::Object(map)) => map.keys().cloned().collect(),
            _ => Vec::new(),
        };
        for dep in deps {
            if crate::ids::is_kebab_case(service) && crate::ids::is_kebab_case(&dep) {
                records.push(record(
                    "k8s_workloads",
                    "candidate_call",
                    json!({"source": service, "destination": dep}),
                ));
            }
        }
    }
    records
}

/// Languages and container use of a repository, from its file names.
#[must_use]
pub fn project_facts(project_path: &str, paths: &[String]) -> Option<RawRecord> {
    let mut languages = std::collections::BTreeSet::new();
    let mut container = false;
    for path in paths {
        if path.split('/').any(|part| {
            matches!(
                part,
                "node_modules" | "vendor" | "deps" | "_build" | "target" | ".venv" | "venv"
            )
        }) {
            continue;
        }
        let file = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
        let language = match file.as_str() {
            "requirements.txt" | "pyproject.toml" | "setup.py" | "pipfile" | "poetry.lock" => {
                Some("python")
            }
            "mix.exs" => Some("elixir"),
            "cargo.toml" => Some("rust"),
            "go.mod" => Some("go"),
            "package.json" => Some("javascript"),
            "pom.xml" | "build.gradle" | "build.gradle.kts" => Some("java"),
            "gemfile" => Some("ruby"),
            "composer.json" => Some("php"),
            _ if file.ends_with(".csproj") => Some("dotnet"),
            _ if file.ends_with(".tf") => Some("terraform"),
            _ => None,
        };
        if let Some(language) = language {
            languages.insert(language);
        }
        if file == "dockerfile"
            || file.ends_with(".dockerfile")
            || file.starts_with("dockerfile.")
            || is_compose_file(&file)
        {
            container = true;
        }
    }
    if languages.is_empty() && !container {
        return None;
    }
    Some(record(
        "k8s_workloads",
        "project_facts",
        json!({"project_path": project_path, "languages": languages, "container": container}),
    ))
}

fn record(collector: &str, kind: &str, payload: Value) -> RawRecord {
    RawRecord {
        collector: collector.to_owned(),
        kind: kind.to_owned(),
        payload,
    }
}

fn interesting_manifest(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".yaml")
        || lower.ends_with(".yml")
        || lower.ends_with(".tf")
        || lower.ends_with("catalog-info.yaml")
}

fn yaml_kind(doc: &str) -> Option<String> {
    for line in doc.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || trimmed.starts_with(' ') || trimmed.starts_with('-') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("kind:") {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn yaml_metadata_name(doc: &str) -> Option<String> {
    let mut in_metadata = false;
    for line in doc.lines() {
        if line.starts_with("metadata:") {
            in_metadata = true;
            continue;
        }
        if in_metadata && !line.starts_with(' ') && !line.starts_with('\t') && !line.is_empty() {
            break;
        }
        if in_metadata {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("name:") {
                return Some(value.trim().trim_matches('"').to_owned());
            }
        }
    }
    None
}

fn collect_files(root: &Path, dir: &Path, files: &mut Vec<(String, String)>) -> Result<(), Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_files(root, &path, files)?;
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(&path);
        let relative = relative.to_string_lossy().replace('\\', "/");
        if interesting_manifest(&relative) && relative != "catalog-info.yaml" {
            let text = fs::read_to_string(&path).unwrap_or_default();
            files.push((relative, text));
        }
    }
    Ok(())
}

fn collect_paths(root: &Path, dir: &Path, paths: &mut Vec<String>) -> Result<(), Error> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_paths(root, &path, paths)?;
        } else {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            paths.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    paths.sort();
    Ok(())
}

fn read_if_exists(path: &Path) -> Option<String> {
    fs::read_to_string(path).ok()
}

#[derive(Debug, Deserialize)]
struct FixtureFile {
    projects: Vec<FixtureProject>,
}

#[derive(Debug, Deserialize)]
struct FixtureProject {
    path: String,
    dir: String,
}

#[derive(Debug, Deserialize)]
struct FixtureDeployment {
    project: String,
    component: String,
    environment: String,
    #[serde(rename = "ref")]
    reference: String,
    created_at: String,
}

#[cfg(test)]
mod manifest_selection_tests {
    #[test]
    fn ingress_and_http_route_become_gateways_with_routes() {
        let text = "apiVersion: networking.k8s.io/v1\nkind: Ingress\nmetadata:\n  name: edge\nspec:\n  defaultBackend:\n    service:\n      name: web\n  rules:\n    - http:\n        paths:\n          - backend:\n              service:\n                name: api\n---\napiVersion: gateway.networking.k8s.io/v1\nkind: HTTPRoute\nmetadata:\n  name: mesh-route\nspec:\n  rules:\n    - backendRefs:\n        - name: api\n";
        let mut found: Vec<String> = gateway_records(text)
            .iter()
            .filter(|r| r.kind == "resource_link")
            .map(|r| format!("{}->{}", r.payload["from"], r.payload["to"]))
            .collect();
        found.sort();
        assert_eq!(
            found,
            [
                "\"edge\"->\"api\"",
                "\"edge\"->\"web\"",
                "\"mesh-route\"->\"api\""
            ]
        );
    }

    #[test]
    fn workloads_are_named_by_app_label_then_object_name() {
        let labelled: serde_json::Value = serde_json::json!({
            "metadata": {"name": "web-deploy"},
            "spec": {"template": {"metadata": {"labels": {"app": "web"}}}}
        });
        assert_eq!(workload_name(&labelled).as_deref(), Some("web"));
        let plain: serde_json::Value = serde_json::json!({"metadata": {"name": "worker"}});
        assert_eq!(workload_name(&plain).as_deref(), Some("worker"));
    }

    #[test]
    fn host_port_env_values_become_service_names_without_localhost() {
        let value: serde_json::Value = serde_json::json!({"spec": {"template": {"spec": {
            "containers": [{"env": [
                {"name": "CART_ADDR", "value": "cartservice:7070"},
                {"name": "SAME", "value": "cartservice:7070"},
                {"name": "LOCAL", "value": "localhost:8080"},
                {"name": "MODE", "value": "production"}
            ]}]
        }}}});
        assert_eq!(env_service_hosts(&value), vec!["cartservice".to_string()]);
    }

    #[test]
    fn gateway_hosts_are_read_and_wildcards_skipped() {
        let text = "kind: Ingress\nmetadata:\n  name: edge\nspec:\n  rules:\n    - host: shop.example.com\n    - host: '*.example.com'\n---\nkind: HTTPRoute\nmetadata:\n  name: r\nspec:\n  hostnames: [api.example.com]\n  rules:\n    - backendRefs:\n        - name: api\n";
        let mut hosts: Vec<String> = gateway_records(text)
            .iter()
            .filter(|r| r.kind == "gateway_host")
            .map(|r| {
                format!(
                    "{}={}",
                    r.payload["gateway"].as_str().unwrap(),
                    r.payload["host"].as_str().unwrap()
                )
            })
            .collect();
        hosts.sort();
        assert_eq!(hosts, ["edge=shop.example.com", "r=api.example.com"]);
    }

    use super::*;
    use crate::gitlab::TreeItem;

    fn blob(path: &str) -> TreeItem {
        TreeItem {
            path: path.to_owned(),
            item_type: "blob".to_owned(),
        }
    }

    #[test]
    fn manifests_after_many_source_files_are_still_read() {
        let mut tree: Vec<TreeItem> = (0..500)
            .map(|index| blob(&format!("src/app/module_{index}.py")))
            .collect();
        tree.push(TreeItem {
            path: "deploy".to_owned(),
            item_type: "tree".to_owned(),
        });
        tree.push(blob("deploy/deployment.yaml"));
        tree.push(blob("terraform/main.tf"));
        let paths: Vec<String> = manifest_items(tree)
            .into_iter()
            .map(|item| item.path)
            .collect();
        assert_eq!(paths, ["deploy/deployment.yaml", "terraform/main.tf"]);
    }

    #[test]
    fn manifest_count_is_capped_per_project() {
        let tree = (0..MAX_MANIFESTS_PER_PROJECT + 50)
            .map(|index| blob(&format!("k8s/{index}.yaml")))
            .collect();
        assert_eq!(manifest_items(tree).len(), MAX_MANIFESTS_PER_PROJECT);
    }

    #[test]
    fn templated_manifests_are_skipped() {
        let text = "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {{ .Release.Name }}\n  labels:\n    app.kubernetes.io/name: {{ .Chart.Name }}\n";
        assert!(records_from_file("group/app", "chart/templates/deployment.yaml", text).is_empty());
    }

    #[test]
    fn a_catalog_file_with_two_entities_yields_both() {
        let yaml = "# third-party apps run by the platform team\n---\napiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: keycloak\nspec:\n  type: service\n---\napiVersion: backstage.io/v1alpha1\nkind: Resource\nmetadata:\n  name: redis-cache\nspec:\n  type: database\n";
        let records = records_from_catalog("example-group/platform/third-party", yaml);
        let names: Vec<&str> = records
            .iter()
            .filter_map(|record| record.payload["name"].as_str())
            .collect();
        assert_eq!(names, ["keycloak", "redis-cache"]);
    }

    #[test]
    fn owners_are_reduced_to_names_and_groups_name_their_parent() {
        let yaml = "kind: Component\nmetadata:\n  name: billing\nspec:\n  owner: group:default/Team-Payments\n  lifecycle: production\n---\nkind: Group\nmetadata:\n  name: team-payments\nspec:\n  parent: group:default/finance\n";
        let records = records_from_catalog("org/billing", yaml);
        assert_eq!(records[0].payload["owner"], "team-payments");
        assert_eq!(records[0].payload["lifecycle"], "production");
        assert_eq!(records[1].kind, "team");
        assert_eq!(records[1].payload["name"], "team-payments");
        assert_eq!(records[1].payload["parent"], "finance");
    }
}
