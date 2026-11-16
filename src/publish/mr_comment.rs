//! One merge-request note for consumers and contract changes.
//!
//! The note is marked with `<!-- depgraph-mr-check -->`. A later run updates
//! that note instead of posting another one. The bot token is read from an
//! environment variable and is not written into the note or into errors.

use crate::analysis::algo;
use crate::config::Aliases;
use crate::error::Error;
use crate::models::{Env, GraphSnapshot};
use crate::redact;

const MARKER: &str = "<!-- depgraph-mr-check -->";

/// Inputs for one MR check. The snapshot is already loaded.
pub struct MrCheck<'a> {
    /// GitLab project path, such as `org/payments-api`.
    pub project_path: &'a str,
    /// Graph to read.
    pub snapshot: &'a GraphSnapshot,
    /// Alias table. Project paths and short names both resolve here.
    pub aliases: &'a Aliases,
    /// Globs from `contract_paths`.
    pub contract_paths: &'a [String],
    /// Files changed in the merge request.
    pub changed_files: &'a [String],
}

/// The note body and whether a contract file changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MrNote {
    /// Markdown body, including the stable marker.
    pub body: String,
    /// Resolved component, when the alias table knew the project.
    pub component: Option<String>,
    /// Whether a changed file matched `contract_paths`.
    pub contract_change: bool,
}

/// Build the note. Does not call GitLab.
#[must_use]
pub fn build_note(check: &MrCheck<'_>) -> MrNote {
    let component = resolve_component(check.project_path, check.aliases, check.snapshot);
    let consumers = component
        .as_ref()
        .and_then(|name| algo::consumers(check.snapshot, name, Some(Env::Prod), None).ok());
    let contract_hits: Vec<&str> = check
        .changed_files
        .iter()
        .filter(|path| {
            check
                .contract_paths
                .iter()
                .any(|pattern| glob_matches(pattern, path))
        })
        .map(String::as_str)
        .collect();
    let mut lines = vec![
        MARKER.to_owned(),
        String::new(),
        format!("project: {}", check.project_path),
    ];
    match &component {
        Some(name) => lines.push(format!("component: {name}")),
        None => lines.push(
            "component: unresolved (add the project path or its last segment to aliases.yaml)"
                .to_owned(),
        ),
    }
    lines.push(format!(
        "contract_change: {}",
        if contract_hits.is_empty() {
            "no"
        } else {
            "yes"
        }
    ));
    for path in &contract_hits {
        lines.push(format!("- {path}"));
    }
    lines.push("prod consumers:".to_owned());
    match consumers {
        Some(visits) if !visits.is_empty() => {
            for visit in visits {
                lines.push(format!("- {} (depth {})", visit.id, visit.depth));
            }
        }
        _ => lines.push("- none".to_owned()),
    }
    MrNote {
        body: lines.join("\n") + "\n",
        component,
        contract_change: !contract_hits.is_empty(),
    }
}

fn resolve_component(
    project_path: &str,
    aliases: &Aliases,
    snapshot: &GraphSnapshot,
) -> Option<String> {
    let last = project_path.rsplit('/').next().unwrap_or(project_path);
    for candidate in [project_path, last] {
        if let Some(name) = aliases.aliases.get(candidate) {
            return Some(name.clone());
        }
    }
    let component = format!("component:{last}");
    snapshot
        .nodes
        .iter()
        .any(|node| node.id == component)
        .then(|| last.to_owned())
}

/// Create or update the single marked note on a merge request.
///
/// # Errors
///
/// Returns an error on HTTP failure. The token is redacted.
pub async fn upsert_note(
    base_url: &str,
    project: &str,
    mr_iid: u64,
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
    let project = crate::gitlab::encode_uri_component(project);
    let list_url = format!(
        "{}/api/v4/projects/{project}/merge_requests/{mr_iid}/notes?per_page=100",
        base_url.trim_end_matches('/')
    );
    let existing = gitlab_get(&client, &list_url, token).await?;
    let notes: Vec<serde_json::Value> = serde_json::from_str(&existing).unwrap_or_default();
    let note_id = notes.iter().find_map(|note| {
        let text = note
            .get("body")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        text.contains(MARKER)
            .then(|| note.get("id").and_then(|value| value.as_u64()))
            .flatten()
    });
    let payload = serde_json::json!({ "body": body });
    if let Some(id) = note_id {
        let url = format!(
            "{}/api/v4/projects/{project}/merge_requests/{mr_iid}/notes/{id}",
            base_url.trim_end_matches('/')
        );
        gitlab_send(&client, reqwest::Method::PUT, &url, token, &payload).await?;
    } else {
        let url = format!(
            "{}/api/v4/projects/{project}/merge_requests/{mr_iid}/notes",
            base_url.trim_end_matches('/')
        );
        gitlab_send(&client, reqwest::Method::POST, &url, token, &payload).await?;
    }
    Ok(())
}

async fn gitlab_get(client: &reqwest::Client, url: &str, token: &str) -> Result<String, Error> {
    let response = client
        .get(url)
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
            "gitlab notes returned {status}: {}",
            redact::redact(&text)
        )));
    }
    Ok(text)
}

async fn gitlab_send(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: &str,
    token: &str,
    payload: &serde_json::Value,
) -> Result<(), Error> {
    let response = client
        .request(method, url)
        .header("PRIVATE-TOKEN", token)
        .json(payload)
        .send()
        .await
        .map_err(|err| Error::GitLab(redact::redact(&err.to_string())))?;
    let status = response.status();
    if !status.is_success() {
        let text = response.text().await.unwrap_or_default();
        return Err(Error::GitLab(format!(
            "gitlab note write returned {status}: {}",
            redact::redact(&text)
        )));
    }
    Ok(())
}

fn glob_matches(pattern: &str, path: &str) -> bool {
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
    regex::Regex::new(&regex)
        .map(|pattern| pattern.is_match(path))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        Edge, EdgeSource, EdgeStatus, EdgeType, Node, NodeType, SourceKind, Timestamp,
    };
    use std::collections::BTreeMap;

    #[test]
    fn note_lists_prod_consumers_and_contract_files_once() {
        let snapshot = GraphSnapshot {
            schema_version: "0.1.0".into(),
            run_id: "test".into(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: vec![
                Node {
                    id: "component:payments-api".into(),
                    node_type: NodeType::Component,
                    attrs: BTreeMap::new(),
                },
                Node {
                    id: "component:billing-api".into(),
                    node_type: NodeType::Component,
                    attrs: BTreeMap::new(),
                },
            ],
            edges: vec![Edge {
                id: "edge:test".into(),
                edge_type: EdgeType::DeclaredDependsOn,
                from_id: "component:billing-api".into(),
                to_id: "component:payments-api".into(),
                env: Some(Env::Prod),
                sources: vec![EdgeSource {
                    kind: SourceKind::Catalog,
                    first_seen: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
                    last_seen: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
                    evidence_ref: None,
                }],
                status: EdgeStatus::DeclaredOnly,
                criticality: Default::default(),
                active: true,
                attrs: Default::default(),
            }],
        };
        let mut aliases = Aliases::default();
        aliases
            .aliases
            .insert("org/payments".into(), "payments-api".into());
        let note = build_note(&MrCheck {
            project_path: "org/payments",
            snapshot: &snapshot,
            aliases: &aliases,
            contract_paths: &["openapi/**".into()],
            changed_files: &["openapi/api.yaml".into(), "README.md".into()],
        });
        assert_eq!(note.component.as_deref(), Some("payments-api"));
        assert!(note.contract_change);
        assert!(note.body.contains(MARKER));
        assert_eq!(note.body.matches(MARKER).count(), 1);
        assert!(note.body.contains("component:billing-api"));
        assert!(note.body.contains("openapi/api.yaml"));
        assert!(!note.body.contains("README.md"));
    }
}
