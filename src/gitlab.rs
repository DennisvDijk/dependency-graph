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

/// A project row from the groups or projects API.
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
    /// Present when the list payload includes it. Simple responses may omit it.
    #[serde(default)]
    pub archived: bool,
    /// `public`, `internal`, or `private`, when the payload includes it.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Project topics. Empty when the payload omits them.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Set when this project is a fork. Absent means "not a fork" for filtering.
    #[serde(default)]
    pub forked_from_project: Option<serde_json::Value>,
}

/// Filters applied after a project list is fetched. Empty collections do not filter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectFilter {
    /// Keep projects whose path matches any of these globs. Empty keeps all.
    pub include_paths: Vec<String>,
    /// Drop projects whose path matches any of these globs.
    pub exclude_paths: Vec<String>,
    /// Keep projects that have at least one of these topics. Empty keeps all.
    pub topics: Vec<String>,
    /// Drop projects that expose `forked_from_project`.
    pub exclude_forks: bool,
    /// Drop projects marked archived.
    pub exclude_archived: bool,
    /// Drop projects whose last activity is older than this, relative to `now`.
    pub min_last_activity: Option<chrono::Duration>,
    /// Keep only this visibility. Empty keeps all.
    pub visibility: Option<String>,
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

    /// List projects in one group, including subgroups.
    ///
    /// Uses keyset pagination when GitLab sends a `Link: rel="next"` header,
    /// and falls back to `x-next-page`.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure. The token is not part of the message.
    pub async fn list_group_projects(
        &self,
        group: &str,
        exclude_archived: bool,
    ) -> Result<Vec<Project>, Error> {
        let group = encode_uri_component(group);
        let archived = if exclude_archived {
            "&archived=false"
        } else {
            ""
        };
        let path = format!(
            "/api/v4/groups/{group}/projects?include_subgroups=true{archived}&pagination=keyset&order_by=id&sort=asc&per_page={}&page=1",
            self.per_page
        );
        self.list_paginated(&path).await
    }

    /// Every project this token can see.
    ///
    /// `GET /projects?membership=false&simple=true`, with keyset pagination.
    /// `archived=false` is sent only when `exclude_archived` is set; the
    /// caller still filters the payload afterwards.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure. The token is not part of the message.
    pub async fn list_visible_projects(
        &self,
        exclude_archived: bool,
    ) -> Result<Vec<Project>, Error> {
        let archived = if exclude_archived {
            "&archived=false"
        } else {
            ""
        };
        let path = format!(
            "/api/v4/projects?membership=false{archived}&simple=true&pagination=keyset&order_by=id&sort=asc&per_page={}",
            self.per_page
        );
        self.list_paginated(&path).await
    }

    /// Projects to collect.
    ///
    /// A non-empty `groups` list is recursive (`include_subgroups=true`).
    /// `explicit_paths` are fetched one by one and unioned with that list.
    /// The full visible project list is used only when both are empty.
    /// Results are deduped by project id. Explicit projects skip the
    /// include/exclude filters. Archived explicit projects are omitted and
    /// named in the returned warnings.
    ///
    /// # Errors
    ///
    /// Returns an error on HTTP failure.
    pub async fn select_scope(
        &self,
        groups: &[String],
        explicit_paths: &[String],
        filter: &ProjectFilter,
        now: chrono::DateTime<chrono::Utc>,
    ) -> Result<(Vec<Project>, Vec<String>), Error> {
        let discovered = if groups.is_empty() && explicit_paths.is_empty() {
            self.list_visible_projects(filter.exclude_archived).await?
        } else {
            let mut all = Vec::new();
            for group in groups {
                all.extend(
                    self.list_group_projects(group, filter.exclude_archived)
                        .await?,
                );
            }
            all
        };
        let chosen = select_projects(&discovered, filter, now);
        let mut explicit = Vec::new();
        for path in explicit_paths {
            explicit.push(self.project_by_path(path).await?);
        }
        Ok(union_explicit(chosen, explicit))
    }

    async fn list_paginated<T: serde::de::DeserializeOwned>(
        &self,
        first: &str,
    ) -> Result<Vec<T>, Error> {
        let mut next = Some(absolute_url(&self.base_url, first));
        let mut all = Vec::new();
        while let Some(url) = next {
            let (status, headers, bytes) =
                self.request_url(reqwest::Method::GET, &url, None).await?;
            if !status.is_success() {
                return Err(http_error("list projects", status, &bytes));
            }
            let batch: Vec<T> = serde_json::from_slice(&bytes)
                .map_err(|err| Error::GitLab(format!("gitlab project page was not JSON: {err}")))?;
            all.extend(batch);
            next = link_next(&headers)
                .map(|href| absolute_url(&self.base_url, &href))
                .or_else(|| next_page(&headers).map(|page| with_page(&url, page)));
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
        self.request_url(method, &absolute_url(&self.base_url, path), body)
            .await
    }

    async fn request_url(
        &self,
        method: reqwest::Method,
        url: &str,
        body: Option<Value>,
    ) -> Result<(reqwest::StatusCode, reqwest::header::HeaderMap, Vec<u8>), Error> {
        for _attempt in 0..5 {
            let mut builder = self
                .http
                .request(method.clone(), url)
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

fn absolute_url(base: &str, url: &str) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        url.to_owned()
    } else if let Some(path) = url.strip_prefix('/') {
        format!("{base}/{path}")
    } else {
        format!("{base}/{url}")
    }
}

fn link_next(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let value = headers.get("link")?.to_str().ok()?;
    for part in value.split(',') {
        let mut pieces = part.split(';');
        let url = pieces.next()?.trim();
        let url = url.trim_start_matches('<').trim_end_matches('>');
        let is_next = pieces.any(|piece| {
            let piece = piece.trim();
            piece.eq_ignore_ascii_case("rel=\"next\"") || piece.eq_ignore_ascii_case("rel=next")
        });
        if is_next {
            return Some(url.to_owned());
        }
    }
    None
}

fn with_page(url: &str, page: u32) -> String {
    if let Some(start) = url.find("page=") {
        let after = start + "page=".len();
        let end = url[after..]
            .find('&')
            .map(|index| after + index)
            .unwrap_or(url.len());
        format!("{}{page}{}", &url[..after], &url[end..])
    } else if url.contains('?') {
        format!("{url}&page={page}")
    } else {
        format!("{url}?page={page}")
    }
}

/// Union an explicit opt-in list onto an already filtered list.
///
/// Archived explicit projects are dropped with a warning. Other explicit
/// projects are kept even when a filter would have removed them. Duplicate
/// ids are kept once.
#[must_use]
pub fn union_explicit(
    mut chosen: Vec<Project>,
    explicit: Vec<Project>,
) -> (Vec<Project>, Vec<String>) {
    let mut seen: std::collections::BTreeSet<u64> =
        chosen.iter().map(|project| project.id).collect();
    let mut warnings = Vec::new();
    for project in explicit {
        if project.archived {
            warnings.push(format!(
                "skipping archived project {}",
                project.path_with_namespace
            ));
            continue;
        }
        if seen.insert(project.id) {
            chosen.push(project);
        }
    }
    chosen.sort_by(|left, right| left.path_with_namespace.cmp(&right.path_with_namespace));
    chosen.dedup_by(|left, right| left.id == right.id);
    (chosen, warnings)
}

/// Keep projects that match every configured filter.
///
/// `now` is injected so tests do not read the clock. A project with no
/// `last_activity_at` fails `min_last_activity`. A project with no
/// `forked_from_project` is not treated as a fork.
#[must_use]
pub fn select_projects(
    projects: &[Project],
    filter: &ProjectFilter,
    now: chrono::DateTime<chrono::Utc>,
) -> Vec<Project> {
    projects
        .iter()
        .filter(|project| project_matches(project, filter, now))
        .cloned()
        .collect()
}

fn project_matches(
    project: &Project,
    filter: &ProjectFilter,
    now: chrono::DateTime<chrono::Utc>,
) -> bool {
    if filter.exclude_archived && project.archived {
        return false;
    }
    if filter.exclude_forks && project.forked_from_project.is_some() {
        return false;
    }
    if let Some(visibility) = &filter.visibility {
        match &project.visibility {
            Some(value) if value.eq_ignore_ascii_case(visibility) => {}
            _ => return false,
        }
    }
    if !filter.topics.is_empty()
        && !filter
            .topics
            .iter()
            .any(|topic| project.topics.iter().any(|have| have == topic))
    {
        return false;
    }
    if !filter.include_paths.is_empty()
        && !filter
            .include_paths
            .iter()
            .any(|pattern| path_glob_matches(pattern, &project.path_with_namespace))
    {
        return false;
    }
    if filter
        .exclude_paths
        .iter()
        .any(|pattern| path_glob_matches(pattern, &project.path_with_namespace))
    {
        return false;
    }
    if let Some(window) = filter.min_last_activity {
        let Some(stamp) = project
            .last_activity_at
            .as_deref()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        else {
            return false;
        };
        if stamp.with_timezone(&chrono::Utc) < now - window {
            return false;
        }
    }
    true
}

/// Parse `180d`, `24h`, or `2w`.
///
/// # Errors
///
/// Returns an error when the text is not a positive integer plus `d`, `h`, or `w`.
pub fn parse_activity_window(value: &str) -> Result<chrono::Duration, Error> {
    let value = value.trim();
    let Some((count, unit)) = value.split_at_checked(value.len().saturating_sub(1)) else {
        return Err(Error::Config(format!(
            "min_last_activity `{value}` must look like 180d"
        )));
    };
    let Ok(count) = count.parse::<i64>() else {
        return Err(Error::Config(format!(
            "min_last_activity `{value}` must look like 180d"
        )));
    };
    if count <= 0 {
        return Err(Error::Config(
            "min_last_activity must be a positive window".to_owned(),
        ));
    }
    let window = match unit {
        "d" => chrono::Duration::days(count),
        "h" => chrono::Duration::hours(count),
        "w" => chrono::Duration::days(count.saturating_mul(7)),
        _ => {
            return Err(Error::Config(format!(
                "min_last_activity `{value}` must end in d, h, or w"
            )));
        }
    };
    Ok(window)
}

fn path_glob_matches(pattern: &str, path: &str) -> bool {
    let expression = glob_to_regex(pattern);
    regex::Regex::new(&expression)
        .map(|pattern| pattern.is_match(path))
        .unwrap_or(false)
}

fn glob_to_regex(pattern: &str) -> String {
    let mut regex = String::from("^");
    let chars: Vec<char> = pattern.chars().collect();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '*' && chars.get(index + 1) == Some(&'*') {
            regex.push_str(".*");
            index += 2;
            continue;
        }
        if chars[index] == '*' {
            regex.push_str("[^/]*");
            index += 1;
            continue;
        }
        if "\\.+()|[]{}^$?".contains(chars[index]) {
            regex.push('\\');
        }
        regex.push(chars[index]);
        index += 1;
    }
    regex.push('$');
    regex
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
            encode_uri_component("org/infra/platform"),
            "org%2Finfra%2Fplatform"
        );
        assert_eq!(
            encode_uri_component("org/other-team/ingress-gateway"),
            "org%2Fother-team%2Fingress-gateway"
        );
    }

    fn sample(id: u64, path: &str) -> Project {
        Project {
            id,
            path_with_namespace: path.to_owned(),
            default_branch: Some("main".to_owned()),
            last_activity_at: Some("2026-11-01T00:00:00Z".to_owned()),
            archived: false,
            visibility: Some("private".to_owned()),
            topics: vec!["platform".to_owned()],
            forked_from_project: None,
        }
    }

    #[test]
    fn filters_and_explicit_opt_in() {
        let mut fork = sample(2, "team/fork");
        fork.forked_from_project = Some(serde_json::json!({"id": 1}));
        let mut archived = sample(3, "team/old");
        archived.archived = true;
        let archived_again = archived.clone();
        let mut old = sample(4, "team/stale");
        old.last_activity_at = Some("2020-01-01T00:00:00Z".to_owned());
        let mut public = sample(5, "other/public");
        public.visibility = Some("public".to_owned());
        public.topics.clear();
        let projects = vec![
            sample(1, "org/infra/platform/api"),
            fork,
            archived,
            old,
            public,
        ];
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-16T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let filter = ProjectFilter {
            include_paths: vec!["org/infra/**".to_owned()],
            exclude_paths: vec!["**/sidecar".to_owned()],
            topics: vec!["platform".to_owned()],
            exclude_forks: true,
            exclude_archived: true,
            min_last_activity: Some(parse_activity_window("180d").unwrap()),
            visibility: Some("private".to_owned()),
        };
        let kept = select_projects(&projects, &filter, now);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].path_with_namespace, "org/infra/platform/api");

        let explicit = vec![sample(9, "org/other-team/ingress-gateway"), archived_again];
        let (unioned, warnings) = union_explicit(kept, explicit);
        assert_eq!(unioned.len(), 2);
        assert!(warnings[0].contains("skipping archived project team/old"));
    }
}
