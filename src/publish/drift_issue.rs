//! One open GitLab issue labeled `depgraph-drift`.
//!
//! A weekly run updates that issue instead of opening a second one. The bot
//! token stays in the environment variable named by config.

use crate::analysis::drift_markdown;
use crate::error::Error;
use crate::models::{Env, GraphSnapshot};
use crate::redact;

const LABEL: &str = "depgraph-drift";

/// Markdown body for the weekly issue. Production drift only.
#[must_use]
pub fn drift_issue_body(graph: &GraphSnapshot) -> String {
    format!(
        "Weekly drift for production. This issue is updated in place.\n\n{}",
        drift_markdown(graph, Env::Prod)
    )
}

/// Create or update the single open `depgraph-drift` issue.
///
/// # Errors
///
/// Returns an error on HTTP failure. The token is redacted.
pub async fn upsert_drift_issue(
    base_url: &str,
    project: &str,
    token: &str,
    body: &str,
) -> Result<(), Error> {
    if base_url.contains('@') {
        return Err(Error::Config(
            "gitlab base url must not contain credentials".to_owned(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| Error::GitLab(redact::redact(&err.to_string())))?;
    let project_id = crate::gitlab::encode_uri_component(project);
    let list_url = format!(
        "{}/api/v4/projects/{project_id}/issues?labels={LABEL}&state=opened&per_page=20",
        base_url.trim_end_matches('/')
    );
    let response = client
        .get(&list_url)
        .header("PRIVATE-TOKEN", token)
        .send()
        .await
        .map_err(|err| Error::GitLab(redact::redact(&err.to_string())))?;
    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|err| Error::GitLab(redact::redact(&err.to_string())))?;
    if !status.is_success() {
        return Err(Error::GitLab(format!(
            "gitlab issues returned {status}: {}",
            redact::redact(&text)
        )));
    }
    let issues: Vec<serde_json::Value> = serde_json::from_str(&text).unwrap_or_default();
    let payload = serde_json::json!({
        "title": "depgraph drift",
        "description": body,
        "labels": LABEL,
    });
    let url = if let Some(iid) = issues
        .first()
        .and_then(|issue| issue.get("iid"))
        .and_then(|value| value.as_u64())
    {
        format!(
            "{}/api/v4/projects/{project_id}/issues/{iid}",
            base_url.trim_end_matches('/')
        )
    } else {
        format!(
            "{}/api/v4/projects/{project_id}/issues",
            base_url.trim_end_matches('/')
        )
    };
    let method = if issues.is_empty() {
        reqwest::Method::POST
    } else {
        reqwest::Method::PUT
    };
    let response = client
        .request(method, &url)
        .header("PRIVATE-TOKEN", token)
        .json(&payload)
        .send()
        .await
        .map_err(|err| Error::GitLab(redact::redact(&err.to_string())))?;
    if !response.status().is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::GitLab(format!(
            "gitlab issue write failed: {}",
            redact::redact(&text)
        )));
    }
    Ok(())
}
