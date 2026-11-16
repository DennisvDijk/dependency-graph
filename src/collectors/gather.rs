//! Turn a fixture directory or a live GitLab group into raw records.
//!
//! Fixture mode does not open a socket. Live mode uses the GitLab client.
//! `kubernetes.live_read` is the only path that may call kubectl, and only
//! when a [`LiveRead`] implementation is passed in.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::gitlab::GitLabClient;
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

/// Read GitLab. `live` is consulted only when `live_read` is true.
///
/// # Errors
///
/// Returns an error when the API call fails.
pub async fn records_from_gitlab(
    client: &GitLabClient,
    groups: &[String],
    catalog_file: &str,
    set: GatherSet,
    live: Option<&dyn LiveRead>,
) -> Result<Vec<RawRecord>, Error> {
    let mut projects = Vec::new();
    for group in groups {
        projects.extend(client.list_group_projects(group).await?);
    }
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
            for item in tree
                .into_iter()
                .filter(|item| item.item_type == "blob")
                .take(40)
            {
                if !interesting_manifest(&item.path) {
                    continue;
                }
                if let Some(text) = client.file_text(project.id, &item.path, &branch).await? {
                    records.extend(records_from_file(
                        &project.path_with_namespace,
                        &item.path,
                        &text,
                    ));
                }
            }
        }
    }
    if let Some(live) = live {
        records.extend(live.list_workloads()?);
    }
    Ok(records)
}

fn records_from_catalog(project_path: &str, yaml: &str) -> Vec<RawRecord> {
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
    vec![record(
        &collector,
        "component",
        json!({"name": name, "depends_on": depends_on, "project_path": project_path}),
    )]
}

fn records_from_tree(project_path: &str, root: &Path) -> Result<Vec<RawRecord>, Error> {
    let mut records = Vec::new();
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    for (path, text) in files {
        records.extend(records_from_file(project_path, &path, &text));
    }
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
                records.push(record(
                    collector,
                    "resource",
                    json!({"resource_kind": "firewall-rule", "name": name, "project_path": project_path}),
                ));
            }
            Some("Deployment") => {
                if let Some(component) = component {
                    records.push(record(
                        collector,
                        "component",
                        json!({"name": component, "depends_on": [], "project_path": project_path}),
                    ));
                }
            }
            _ => {}
        }
    }
    records
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
