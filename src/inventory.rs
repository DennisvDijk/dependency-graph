//! `depgraph inventory`: one CSV row per GitLab project.
//!
//! The token is resolved at call time and is not written to the CSV.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use chrono::{DateTime, Utc};

use crate::error::Error;
use crate::gitlab::{GitLabClient, Job, Project, ProjectFilter, resolve_token};
use crate::repo_scan::{self, JobHint, RepoFlags};

/// One inventory row. Column order is the CSV header order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InventoryRow {
    /// `group/project` path.
    pub path: String,
    /// Default branch, or empty.
    pub default_branch: String,
    /// Last activity timestamp, or empty.
    pub last_activity: String,
    /// Repository signals.
    pub flags: RepoFlags,
}

/// Inputs for one inventory run.
pub struct InventoryQuery<'a> {
    /// GitLab origin.
    pub base_url: &'a str,
    /// Group or subgroup paths. Empty together with `projects` means a full scan.
    pub groups: &'a [String],
    /// Explicit project paths.
    pub projects: &'a [String],
    /// Filters for the group or full-scan list.
    pub filter: &'a ProjectFilter,
    /// Catalog file name.
    pub catalog_file: &'a str,
    /// GitLab page size.
    pub per_page: u32,
    /// Parallel project inspections.
    pub max_concurrency: usize,
    /// Clock injected for `min_last_activity`.
    pub now: DateTime<Utc>,
}

/// Collect inventory.
///
/// Empty `groups` and `projects` lists every project the token can see.
/// A group or subgroup path lists that node and everything beneath it.
/// Explicit project paths are fetched individually and unioned in.
/// Filters apply to the group or full-scan list. Explicit projects skip
/// those filters, except archived projects, which are skipped with a warning.
///
/// # Errors
///
/// Returns an error when the token or the GitLab API is unavailable.
pub async fn collect_inventory(
    query: InventoryQuery<'_>,
) -> Result<(Vec<InventoryRow>, Vec<String>), Error> {
    let token = resolve_token(query.base_url)?;
    let client = GitLabClient::new(query.base_url, token, query.per_page)?;
    let (projects, warnings) = client
        .select_scope(query.groups, query.projects, query.filter, query.now)
        .await?;

    let semaphore = Arc::new(Semaphore::new(query.max_concurrency.max(1)));
    let mut tasks = JoinSet::new();
    for project in projects {
        let client = client.clone();
        let semaphore = semaphore.clone();
        let catalog_file = query.catalog_file.to_owned();
        tasks.spawn(async move {
            let _permit = semaphore
                .acquire()
                .await
                .map_err(|_| Error::GitLab("inventory concurrency limiter closed".to_owned()))?;
            inspect_project(&client, project, &catalog_file).await
        });
    }
    let mut rows = Vec::new();
    while let Some(joined) = tasks.join_next().await {
        rows.push(joined.map_err(|err| Error::GitLab(format!("inventory task failed: {err}")))??);
    }
    rows.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((rows, warnings))
}

async fn inspect_project(
    client: &GitLabClient,
    project: Project,
    catalog_file: &str,
) -> Result<InventoryRow, Error> {
    let branch = project
        .default_branch
        .clone()
        .unwrap_or_else(|| "main".to_owned());
    let tree = client.repository_tree(project.id, &branch).await?;
    let paths: Vec<&str> = tree
        .iter()
        .filter(|item| item.item_type == "blob")
        .map(|item| item.path.as_str())
        .collect();
    let ci_text = if paths
        .iter()
        .any(|path| *path == ".gitlab-ci.yml" || path.ends_with("/.gitlab-ci.yml"))
    {
        client
            .file_text(project.id, ".gitlab-ci.yml", &branch)
            .await?
    } else {
        None
    };
    let jobs = client.jobs(project.id).await?;
    let flags = repo_scan::classify(&paths, catalog_file, ci_text.as_deref(), &job_hints(&jobs));
    Ok(InventoryRow {
        path: project.path_with_namespace,
        default_branch: project.default_branch.unwrap_or_default(),
        last_activity: project.last_activity_at.unwrap_or_default(),
        flags,
    })
}

fn job_hints(jobs: &[Job]) -> Vec<JobHint> {
    jobs.iter()
        .map(|job| {
            let mut filenames = job
                .artifacts
                .iter()
                .filter_map(|artifact| artifact.filename.clone())
                .collect::<Vec<_>>();
            if let Some(file) = &job.artifacts_file {
                filenames.push(file.filename.clone());
            }
            JobHint {
                name: job.name.clone(),
                filenames,
            }
        })
        .collect()
}

/// Write rows as CSV, sorted by path. Returns the number of rows.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_inventory_csv(path: &Path, rows: &[InventoryRow]) -> Result<(), Error> {
    let mut lines = vec![
        "path,default_branch,last_activity,has_catalog_info,uses_environment,has_sbom_artifact,has_k8s_manifests,has_helm,has_kustomize,has_terraform,has_istio_config".to_owned(),
    ];
    for row in rows {
        let flags = &row.flags;
        lines.push(format!(
            "{},{},{},{},{},{},{},{},{},{},{}",
            csv_field(&row.path),
            csv_field(&row.default_branch),
            csv_field(&row.last_activity),
            flags.has_catalog_info,
            flags.uses_environment,
            flags.has_sbom_artifact,
            flags.has_k8s_manifests,
            flags.has_helm,
            flags.has_kustomize,
            flags.has_terraform,
            flags.has_istio_config,
        ));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, text)?;
    Ok(())
}

fn csv_field(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}
