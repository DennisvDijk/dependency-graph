//! GitLab HTTP client.
//!
//! Tokens come from `GITLAB_TOKEN` or `glab config get token --host <host>`.
//! The token stays in memory for the process and is never written to logs,
//! errors, config, or the graph.

use std::process::Command;
use std::time::Duration;

use base64::Engine;
use serde::Deserialize;
use serde_json::Value;

use crate::error::Error;
use crate::redact;

/// Read-only GitLab API client. There is no merge method.
#[derive(Clone)]
pub struct GitLabClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
    per_page: u32,
}

impl std::fmt::Debug for GitLabClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GitLabClient")
            .field("base_url", &self.base_url)
            .field("token", &"***")
            .field("per_page", &self.per_page)
            .finish()
    }
}

/// A project row from the groups API.
#[derive(Debug, Clone, Deserialize)]
pub struct Project {
    /// Numeric project id.
    pub id: u64,
    /// `group/subgroup/project`.
    pub path_with_namespace: String,
    /// Default branch, when the repo is not empty.
    #[serde(default)]
    pub default_branch: Option<String>,
    /// Last activity timestamp from GitLab.
    #[serde(default)]
    pub last_activity_at: Option<String>,
}

/// One repository tree entry.
#[derive(Debug, Clone, Deserialize)]
pub struct TreeItem {
    /// Path relative to the repository root.
    pub path: String,
    /// `blob` or `tree`.
    #[serde(rename = "type")]
    pub item_type: String,
}

/// A CI job, used only to notice SBOM artifacts.
#[derive(Debug, Clone, Deserialize)]
pub struct Job {
    /// Job name.
    pub name: String,
    /// Individual artifact entries, when GitLab returns them.
    #[serde(default)]
    pub artifacts: Vec<JobArtifact>,
    /// Archive metadata.
    #[serde(default)]
    pub artifacts_file: Option<JobArtifactFile>,
}

/// One artifact on a job.
#[derive(Debug, Clone, Deserialize)]
pub struct JobArtifact {
    /// File name, if present.
    #[serde(default)]
    pub filename: Option<String>,
    /// GitLab artifact type.
    #[serde(default)]
    pub file_type: Option<String>,
}

/// The job's artifact archive.
#[derive(Debug, Clone, Deserialize)]
pub struct JobArtifactFile {
    /// Archive file name.
    pub filename: String,
}

#[derive(Debug, Deserialize)]
struct GitlabFile {
    content: String,
    #[serde(default)]
    encoding: String,
}

/// Resolve a token without printing it.
///
/// `GITLAB_TOKEN` wins so tests and CI can inject a token without `glab`.
/// Otherwise the host is taken from `base_url` and read with
/// `glab config get token --host <host>`.
///
/// # Errors
///
/// Returns an error when neither source yields a token. The message does not
/// include token text.
pub fn resolve_token(base_url: &str) -> Result<String, Error> {
    if let Ok(token) = std::env::var("GITLAB_TOKEN") {
        let token = token.trim().to_owned();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let host = gitlab_host(base_url)?;
    let output = Command::new("glab")
        .args(["config", "get", "token", "--host", &host])
        .output()
        .map_err(|_| {
            Error::GitLab(
                "GitLab token not found. Set GITLAB_TOKEN or run `glab auth login`. depgraph does not store the token."
                    .to_owned(),
            )
        })?;
    if !output.status.success() {
        return Err(Error::GitLab(
            "GitLab token not found. Set GITLAB_TOKEN or run `glab auth login`. depgraph does not store the token."
                .to_owned(),
        ));
    }
    let token = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if token.is_empty() || token == "null" {
        return Err(Error::GitLab(
            "glab returned an empty token. Run `glab auth login`. depgraph does not store the token."
                .to_owned(),
        ));
    }
    Ok(token)
}

/// Host (and port, when present) of a GitLab base URL.
///
/// # Errors
///
/// Returns an error when `base_url` is not http(s).
pub fn gitlab_host(base_url: &str) -> Result<String, Error> {
    let rest = base_url
        .strip_prefix("https://")
        .or_else(|| base_url.strip_prefix("http://"))
        .ok_or_else(|| Error::Config(format!("gitlab base url `{base_url}` is not http(s)")))?;
    let host = rest.split('/').next().unwrap_or("").trim();
    if host.is_empty() {
        return Err(Error::Config("gitlab base url has no host".to_owned()));
    }
    Ok(host.to_owned())
}

impl GitLabClient {
    /// Build a client. `per_page` is clamped to 1..=100.
    ///
    /// # Errors
    ///
    /// Returns an error when the HTTP client cannot be built.
    pub fn new(
        base_url: impl Into<String>,
        token: impl Into<String>,
        per_page: u32,
    ) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|err| Error::GitLab(redact::redact(&err.to_string())))?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_owned(),
            token: token.into(),
            per_page: per_page.clamp(1, 100),
        })
    }

    /// List active projects in a group, including subgroups.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure. The token is not part of the message.
    pub async fn list_group_projects(&self, group: &str) -> Result<Vec<Project>, Error> {
        let group = encode_uri_component(group);
        let mut page = 1_u32;
        let mut all = Vec::new();
        loop {
            let path = format!(
                "/api/v4/groups/{group}/projects?include_subgroups=true&archived=false&per_page={}&page={page}",
                self.per_page
            );
            let (status, headers, bytes) = self.request(reqwest::Method::GET, &path, None).await?;
            if !status.is_success() {
                return Err(http_error("list group projects", status, &bytes));
            }
            let batch: Vec<Project> = serde_json::from_slice(&bytes)
                .map_err(|err| Error::GitLab(format!("gitlab project page was not JSON: {err}")))?;
            let count = batch.len();
            all.extend(batch);
            match next_page(&headers) {
                Some(next) => page = next,
                None if count < self.per_page as usize => break,
                None => break,
            }
        }
        Ok(all)
    }

    /// Recursive repository tree for `git_ref`.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure.
    pub async fn repository_tree(
        &self,
        project_id: u64,
        git_ref: &str,
    ) -> Result<Vec<TreeItem>, Error> {
        let git_ref = encode_uri_component(git_ref);
        let mut page = 1_u32;
        let mut all = Vec::new();
        loop {
            let path = format!(
                "/api/v4/projects/{project_id}/repository/tree?recursive=true&per_page={}&page={page}&ref={git_ref}",
                self.per_page
            );
            let (status, headers, bytes) = self.request(reqwest::Method::GET, &path, None).await?;
            if status == reqwest::StatusCode::NOT_FOUND {
                return Ok(Vec::new());
            }
            if !status.is_success() {
                return Err(http_error("repository tree", status, &bytes));
            }
            let batch: Vec<TreeItem> = serde_json::from_slice(&bytes)
                .map_err(|err| Error::GitLab(format!("gitlab tree page was not JSON: {err}")))?;
            let count = batch.len();
            all.extend(batch);
            match next_page(&headers) {
                Some(next) => page = next,
                None => {
                    if count < self.per_page as usize {
                        break;
                    }
                    break;
                }
            }
        }
        Ok(all)
    }

    /// Decoded text of one file, or `None` when GitLab returns 404.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure other than 404.
    pub async fn file_text(
        &self,
        project_id: u64,
        file_path: &str,
        git_ref: &str,
    ) -> Result<Option<String>, Error> {
        let path = format!(
            "/api/v4/projects/{project_id}/repository/files/{}?ref={}",
            encode_uri_component(file_path),
            encode_uri_component(git_ref)
        );
        let (status, _, bytes) = self.request(reqwest::Method::GET, &path, None).await?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            return Err(http_error("repository file", status, &bytes));
        }
        let file: GitlabFile = serde_json::from_slice(&bytes)
            .map_err(|err| Error::GitLab(format!("gitlab file payload was not JSON: {err}")))?;
        if file.encoding.eq_ignore_ascii_case("base64") {
            let cleaned: String = file
                .content
                .chars()
                .filter(|ch| !ch.is_whitespace())
                .collect();
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .map_err(|err| Error::GitLab(format!("gitlab file was not valid base64: {err}")))?;
            Ok(Some(String::from_utf8_lossy(&decoded).into_owned()))
        } else {
            Ok(Some(file.content))
        }
    }

    /// Recent jobs. Used to notice CycloneDX artifacts.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure.
    pub async fn jobs(&self, project_id: u64) -> Result<Vec<Job>, Error> {
        let path = format!(
            "/api/v4/projects/{project_id}/jobs?per_page={}&page=1",
            self.per_page
        );
        let (status, _, bytes) = self.request(reqwest::Method::GET, &path, None).await?;
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            return Err(http_error("jobs", status, &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|err| Error::GitLab(format!("gitlab jobs payload was not JSON: {err}")))
    }

    /// Look up one project by `group/project` path.
    ///
    /// # Errors
    ///
    /// Returns an error when the project does not exist or the call fails.
    pub async fn project_by_path(&self, path: &str) -> Result<Project, Error> {
        let url = format!("/api/v4/projects/{}", encode_uri_component(path));
        let (status, _, bytes) = self.request(reqwest::Method::GET, &url, None).await?;
        if !status.is_success() {
            return Err(http_error("project lookup", status, &bytes));
        }
        serde_json::from_slice(&bytes)
            .map_err(|err| Error::GitLab(format!("gitlab project payload was not JSON: {err}")))
    }

    /// Commit `catalog-info.yaml` on a new branch and open a merge request.
    ///
    /// This never calls the merge endpoint.
    ///
    /// # Errors
    ///
    /// Returns an error when the commit or the merge-request create fails.
    pub async fn open_catalog_mr(
        &self,
        project_id: u64,
        start_branch: &str,
        yaml: &str,
    ) -> Result<String, Error> {
        let commit = serde_json::json!({
            "branch": "depgraph/catalog-info",
            "start_branch": start_branch,
            "commit_message": "Add catalog-info.yaml draft",
            "actions": [{
                "action": "create",
                "file_path": "catalog-info.yaml",
                "content": yaml
            }]
        });
        let commit_path = format!("/api/v4/projects/{project_id}/repository/commits");
        let (status, _, bytes) = self
            .request(reqwest::Method::POST, &commit_path, Some(commit))
            .await?;
        if !status.is_success() {
            return Err(http_error("create catalog commit", status, &bytes));
        }
        let mr = serde_json::json!({
            "source_branch": "depgraph/catalog-info",
            "target_branch": start_branch,
            "title": "Add catalog-info.yaml draft",
            "description": "Generated by `depgraph propose-catalog`. Review before merging. depgraph does not merge.",
            "remove_source_branch": true
        });
        let mr_path = format!("/api/v4/projects/{project_id}/merge_requests");
        let (status, _, bytes) = self
            .request(reqwest::Method::POST, &mr_path, Some(mr))
            .await?;
        if !status.is_success() {
            return Err(http_error("open merge request", status, &bytes));
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|err| {
            Error::GitLab(format!("gitlab merge request payload was not JSON: {err}"))
        })?;
        Ok(value
            .get("web_url")
            .and_then(Value::as_str)
            .unwrap_or("merge request opened")
            .to_owned())
    }

    async fn request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Vec<u8>), Error> {
        let url = format!("{}{path}", self.base_url);
        for _attempt in 0..5 {
            let mut builder = self
                .http
                .request(method.clone(), &url)
                .header("PRIVATE-TOKEN", &self.token);
            if let Some(body) = &body {
                builder = builder.json(body);
            }
            let response = builder.send().await.map_err(|err| {
                Error::GitLab(redact::redact(&format!("gitlab request failed: {err}")))
            })?;
            if response.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let wait = retry_after(response.headers());
                tokio::time::sleep(wait).await;
                continue;
            }
            let status = response.status();
            let headers = response.headers().clone();
            let bytes = response.bytes().await.map_err(|err| {
                Error::GitLab(redact::redact(&format!(
                    "gitlab response body failed: {err}"
                )))
            })?;
            return Ok((status, headers, bytes.to_vec()));
        }
        Err(Error::GitLab(
            "gitlab kept returning 429 after retries".to_owned(),
        ))
    }
}

fn retry_after(headers: &reqwest::header::HeaderMap) -> Duration {
    let seconds = headers
        .get("retry-after")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(1);
    Duration::from_secs(seconds.min(30))
}

fn next_page(headers: &reqwest::header::HeaderMap) -> Option<u32> {
    let value = headers.get("x-next-page")?.to_str().ok()?;
    if value.is_empty() {
        return None;
    }
    value.parse().ok().filter(|page| *page > 0)
}

fn http_error(action: &str, status: reqwest::StatusCode, body: &[u8]) -> Error {
    let snippet = String::from_utf8_lossy(body);
    let snippet: String = snippet.chars().take(180).collect();
    Error::GitLab(redact::redact(&format!(
        "gitlab {action} failed with status {status}: {snippet}"
    )))
}

fn encode_uri_component(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_strips_scheme_and_path() {
        assert_eq!(
            gitlab_host("https://gitlab.example.com/api").unwrap(),
            "gitlab.example.com"
        );
        assert_eq!(gitlab_host("http://127.0.0.1:9").unwrap(), "127.0.0.1:9");
    }

    #[test]
    fn encoding_escapes_slashes() {
        assert_eq!(
            encode_uri_component("example-group/pilot"),
            "example-group%2Fpilot"
        );
    }
}
