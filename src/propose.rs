//! Draft `catalog-info.yaml` files from a local checkout.
//!
//! The default is a dry run that writes under `--output` and does not call
//! GitLab. `--open-mr` opens a merge request and never merges it.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::gitlab::{GitLabClient, resolve_token};
use crate::ids::is_kebab_case;
use crate::repo_scan::{self, kubernetes_app_names};

/// One generated draft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogDraft {
    /// Repository directory name or project path.
    pub repo: String,
    /// Suggested component name.
    pub name: String,
    /// YAML document.
    pub yaml: String,
}

/// Walk `from` and build drafts. A directory that itself looks like a repo is
/// one draft. Otherwise each child directory is a repo.
///
/// # Errors
///
/// Returns an error when `from` cannot be read.
pub fn draft_from_directory(from: &Path) -> Result<Vec<CatalogDraft>, Error> {
    let repos = repo_directories(from)?;
    let mut drafts = Vec::new();
    for repo in repos {
        drafts.push(draft_repo(&repo)?);
    }
    drafts.sort_by(|left, right| left.repo.cmp(&right.repo));
    Ok(drafts)
}

/// Write drafts to `output/<name>/catalog-info.yaml`. Does not touch `from`.
///
/// # Errors
///
/// Returns an error when a file cannot be written.
pub fn write_drafts(output: &Path, drafts: &[CatalogDraft]) -> Result<Vec<PathBuf>, Error> {
    let mut written = Vec::new();
    for draft in drafts {
        let dir = output.join(&draft.name);
        fs::create_dir_all(&dir)?;
        let path = dir.join("catalog-info.yaml");
        fs::write(&path, &draft.yaml)?;
        written.push(path);
    }
    Ok(written)
}

/// Open one merge request per draft. `project_paths` are GitLab paths, in the
/// same order as `drafts`. Never merges.
///
/// # Errors
///
/// Returns an error when the token or the API call fails.
pub async fn open_merge_requests(
    base_url: &str,
    per_page: u32,
    drafts: &[CatalogDraft],
    project_paths: &[String],
) -> Result<Vec<String>, Error> {
    if drafts.len() != project_paths.len() {
        return Err(Error::Config(
            "open-mr needs one GitLab project path per draft".to_owned(),
        ));
    }
    let token = resolve_token(base_url)?;
    let client = GitLabClient::new(base_url, token, per_page)?;
    let mut urls = Vec::new();
    for (draft, path) in drafts.iter().zip(project_paths) {
        let project = client.project_by_path(path).await?;
        let branch = project.default_branch.unwrap_or_else(|| "main".to_owned());
        let url = client
            .open_catalog_mr(project.id, &branch, &draft.yaml)
            .await?;
        urls.push(url);
    }
    Ok(urls)
}

fn repo_directories(from: &Path) -> Result<Vec<PathBuf>, Error> {
    if looks_like_repo(from) {
        return Ok(vec![from.to_path_buf()]);
    }
    let mut repos = Vec::new();
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && looks_like_repo(&entry.path()) {
            repos.push(entry.path());
        }
    }
    repos.sort();
    Ok(repos)
}

fn looks_like_repo(path: &Path) -> bool {
    path.join(".gitlab-ci.yml").is_file()
        || path.join("Chart.yaml").is_file()
        || path.join("catalog-info.yaml").is_file()
        || path.join("deploy").is_dir()
        || path.join("k8s").is_dir()
}

fn draft_repo(repo: &Path) -> Result<CatalogDraft, Error> {
    let repo_name = repo
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_else(|| "repo".to_owned());
    let mut texts = Vec::new();
    let mut chart_name = None;
    let mut environments = Vec::new();
    walk_text(repo, &mut texts, &mut chart_name, &mut environments)?;
    let mut app_name = None;
    for text in &texts {
        if let Some(name) = kubernetes_app_names(text).into_iter().next() {
            app_name = Some(name);
            break;
        }
    }
    let name = app_name
        .or(chart_name)
        .filter(|name| is_kebab_case(name))
        .unwrap_or(repo_name.clone());
    let env_annotation = if environments.is_empty() {
        String::new()
    } else {
        format!(
            "    depgraph/gitlab-environments: {}\n",
            environments.join(",")
        )
    };
    let yaml = format!(
        "apiVersion: backstage.io/v1alpha1\nkind: Component\nmetadata:\n  name: {name}\n  annotations:\n    gitlab.com/project-slug: {repo_name}\n    depgraph/k8s-app-name: {name}\n    depgraph/otel-service-name: {name}\n{env_annotation}spec:\n  type: service\n  lifecycle: experimental\n  owner: ai-platform\n  system: unknown\n"
    );
    Ok(CatalogDraft {
        repo: repo_name,
        name,
        yaml,
    })
}

fn walk_text(
    dir: &Path,
    texts: &mut Vec<String>,
    chart_name: &mut Option<String>,
    environments: &mut Vec<String>,
) -> Result<(), Error> {
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_text(&path, texts, chart_name, environments)?;
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let interesting = name == ".gitlab-ci.yml"
            || name == "Chart.yaml"
            || name.ends_with(".yaml")
            || name.ends_with(".yml");
        if !interesting {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap_or_default();
        if name == "Chart.yaml" {
            *chart_name = repo_scan::helm_chart_name(&text);
        }
        if name == ".gitlab-ci.yml" {
            for env in repo_scan::ci_environment_names(&text) {
                if !environments.contains(&env) {
                    environments.push(env);
                }
            }
        }
        texts.push(text);
    }
    Ok(())
}
