//! Merge raw collector records into one snapshot.
//!
//! Pure: the caller passes `generated_at`. No network and no clock.
//! One deployment is kept per component per environment. A higher version
//! rank wins (digest, then semver, then git sha, then any other tag).
//! Losers are recorded as deployment mismatches. Names that do not resolve
//! are returned for `unresolved.csv`.

use std::collections::{BTreeMap, BTreeSet};

use semver::Version;

use crate::config::{Aliases, EnvIndex};
use crate::error::Error;
use crate::ids::{self, ResourceKind};
use crate::models::{
    Edge, EdgeAttrs, EdgeSource, EdgeStatus, EdgeType, Env, GraphSnapshot, Node, NodeType,
    SourceKind, SourceSummary, Timestamp,
};
use crate::raw::RawRecord;
use crate::store::DeploymentMismatch;

/// A name the normalizer could not attach to a node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    /// Record kind, such as `depends_on` or `environment`.
    pub kind: String,
    /// The unresolved text.
    pub name: String,
    /// Why it was skipped.
    pub reason: String,
}

/// Graph plus the side files a build writes.
#[derive(Debug, Clone, PartialEq)]
pub struct BuildOutput {
    /// Canonical graph, not yet sorted. Callers sort via `to_canonical_json`.
    pub graph: GraphSnapshot,
    /// Rows for `unresolved.csv`.
    pub unresolved: Vec<Unresolved>,
    /// Versions that lost the precedence check.
    pub mismatches: Vec<DeploymentMismatch>,
}

/// Merge records. `libraries_enabled` false drops library nodes and edges.
///
/// # Errors
///
/// Returns an error when a record payload is not the expected shape or the
/// resulting graph breaks an invariant.
pub fn merge_records(
    records: &[RawRecord],
    aliases: &Aliases,
    environments: &EnvIndex,
    generated_at: &Timestamp,
    run_id: &str,
    libraries_enabled: bool,
) -> Result<BuildOutput, Error> {
    let mut components: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut resources: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut resource_links: Vec<(String, String, String)> = Vec::new();
    let mut deployments: Vec<DeploymentCand> = Vec::new();
    let mut unresolved = Vec::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();

    for record in records {
        *counts.entry(record.collector.clone()).or_insert(0) += 1;
        match record.kind.as_str() {
            "component" => {
                let name = canonical_name(required_str(&record.payload, "name")?, aliases);
                if !ids::is_kebab_case(&name) {
                    unresolved.push(unresolved_row("component", &name, "name is not kebab-case"));
                    continue;
                }
                let deps = record
                    .payload
                    .get("depends_on")
                    .and_then(|value| value.as_array())
                    .cloned()
                    .unwrap_or_default();
                let entry = components.entry(name).or_default();
                for dep in deps {
                    if let Some(dep) = dep.as_str() {
                        entry.insert(dep.to_owned());
                    }
                }
            }
            "resource" => {
                let kind = required_str(&record.payload, "resource_kind")?;
                let name = required_str(&record.payload, "name")?;
                if ResourceKind::parse(kind).is_none() || !ids::is_kebab_case(name) {
                    unresolved.push(unresolved_row(
                        "resource",
                        name,
                        "kind or name is not canonical",
                    ));
                    continue;
                }
                resources.insert((kind.to_owned(), name.to_owned()), record.collector.clone());
                if let Some(component) = record
                    .payload
                    .get("component")
                    .and_then(|value| value.as_str())
                {
                    resource_links.push((
                        canonical_name(component, aliases),
                        kind.to_owned(),
                        name.to_owned(),
                    ));
                }
            }
            "gitlab_deployment" => {
                let component =
                    canonical_name(required_str(&record.payload, "component")?, aliases);
                let environment = required_str(&record.payload, "environment")?;
                let Some(env) = environments.resolve(environment) else {
                    unresolved.push(unresolved_row(
                        "environment",
                        environment,
                        "no canonical environment",
                    ));
                    continue;
                };
                let version = required_str(&record.payload, "version")?;
                let (rank, normalized) = version_rank(version);
                deployments.push(DeploymentCand {
                    component,
                    env,
                    version: normalized,
                    rank,
                    created_at: record
                        .payload
                        .get("created_at")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_owned(),
                });
            }
            "library" if !libraries_enabled => {
                unresolved.push(unresolved_row(
                    "library",
                    &record.payload.to_string(),
                    "libraries are disabled",
                ));
            }
            other => unresolved.push(unresolved_row("record", other, "ignored record kind")),
        }
    }

    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for name in components.keys() {
        nodes.push(Node {
            id: ids::component_id(name).map_err(Error::Validation)?,
            node_type: NodeType::Component,
            attrs: BTreeMap::new(),
        });
    }
    for (kind, name) in resources.keys() {
        let parsed = ResourceKind::parse(kind).expect("kind checked");
        nodes.push(Node {
            id: ids::resource_id(parsed, name).map_err(Error::Validation)?,
            node_type: NodeType::Resource,
            attrs: BTreeMap::new(),
        });
    }

    let component_ids: BTreeSet<String> = components
        .keys()
        .map(|name| format!("component:{name}"))
        .collect();
    let resource_ids: BTreeMap<String, String> = resources
        .keys()
        .map(|(kind, name)| (name.clone(), format!("resource:{kind}:{name}")))
        .collect();

    for (from, deps) in &components {
        let from_id = format!("component:{from}");
        for dep in deps {
            match resolve_ref(dep, &component_ids, &resource_ids) {
                Some(to_id) => edges.push(depends_edge(&from_id, &to_id, generated_at)),
                None => unresolved.push(unresolved_row("depends_on", dep, &format!("from {from}"))),
            }
        }
    }
    for (component, kind, name) in &resource_links {
        let from_id = format!("component:{component}");
        let to_id = format!("resource:{kind}:{name}");
        if component_ids.contains(&from_id) && resources.contains_key(&(kind.clone(), name.clone()))
        {
            edges.push(depends_edge(&from_id, &to_id, generated_at));
        }
    }

    let mut grouped: BTreeMap<(String, Env), Vec<DeploymentCand>> = BTreeMap::new();
    for deployment in deployments {
        if !components.contains_key(&deployment.component) {
            unresolved.push(unresolved_row(
                "deployment",
                &deployment.component,
                "component was not declared",
            ));
            continue;
        }
        grouped
            .entry((deployment.component.clone(), deployment.env))
            .or_default()
            .push(deployment);
    }
    let mut mismatches = Vec::new();
    for ((component, env), mut cands) in grouped {
        cands.sort_by(|left, right| left.version.cmp(&right.version));
        let winner_index = cands
            .iter()
            .enumerate()
            .max_by(|(_, left), (_, right)| cmp_deployment(left, right))
            .map(|(index, _)| index)
            .unwrap_or(0);
        let winner = cands[winner_index].clone();
        let discarded: Vec<String> = cands
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != winner_index)
            .map(|(_, cand)| cand.version.clone())
            .collect();
        if !discarded.is_empty() {
            mismatches.push(DeploymentMismatch {
                component: component.clone(),
                env: env.as_str().to_owned(),
                chosen: winner.version.clone(),
                discarded,
            });
        }
        let deployment_id = ids::deployment_id(&component, env).map_err(Error::Validation)?;
        nodes.push(Node {
            id: deployment_id.clone(),
            node_type: NodeType::Deployment,
            attrs: BTreeMap::from([(
                "version".to_owned(),
                serde_json::Value::String(winner.version.clone()),
            )]),
        });
        let from_id = format!("component:{component}");
        let stamp = Timestamp::parse(&winner.created_at).unwrap_or_else(|_| generated_at.clone());
        edges.push(deployed_edge(&from_id, &deployment_id, env, &stamp));
    }

    let mut sources = BTreeMap::new();
    for (name, count) in counts {
        sources.insert(
            name,
            SourceSummary {
                status: crate::models::RunStatus::Ok,
                finished_at: Some(generated_at.clone()),
                count,
            },
        );
    }
    edges.sort_by(|left, right| left.id.cmp(&right.id));
    edges.dedup_by(|left, right| left.id == right.id);
    let mut graph = GraphSnapshot {
        schema_version: "0.1.0".to_owned(),
        run_id: run_id.to_owned(),
        generated_at: generated_at.clone(),
        sources,
        nodes,
        edges,
    };
    graph.canonicalize();
    crate::normalize::validate(&graph, Some(aliases)).into_result()?;
    unresolved.sort_by(|left, right| {
        (&left.kind, &left.name, &left.reason).cmp(&(&right.kind, &right.name, &right.reason))
    });
    unresolved.dedup();
    mismatches
        .sort_by(|left, right| (&left.component, &left.env).cmp(&(&right.component, &right.env)));
    Ok(BuildOutput {
        graph,
        unresolved,
        mismatches,
    })
}

/// CSV body for unresolved names, including the header and a trailing newline.
#[must_use]
pub fn unresolved_csv(rows: &[Unresolved]) -> String {
    let mut lines = vec!["kind,name,reason".to_owned()];
    for row in rows {
        lines.push(format!(
            "{},{},{}",
            csv(&row.kind),
            csv(&row.name),
            csv(&row.reason)
        ));
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// Rank a version string. Higher is better.
#[must_use]
pub fn version_rank(reference: &str) -> (u8, String) {
    if reference.contains("sha256:") {
        return (4, reference.to_owned());
    }
    let trimmed = reference.trim().trim_start_matches('v');
    if Version::parse(trimmed).is_ok() {
        return (3, trimmed.to_owned());
    }
    if reference.len() == 40 && reference.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return (2, reference.to_owned());
    }
    if !reference.is_empty() {
        return (1, reference.to_owned());
    }
    (0, String::new())
}

fn canonical_name(name: &str, aliases: &Aliases) -> String {
    aliases
        .aliases
        .get(name)
        .cloned()
        .unwrap_or_else(|| name.to_owned())
}

fn required_str<'a>(payload: &'a serde_json::Value, field: &str) -> Result<&'a str, Error> {
    payload
        .get(field)
        .and_then(|value| value.as_str())
        .ok_or_else(|| Error::Validation(format!("record is missing `{field}`")))
}

fn resolve_ref(
    reference: &str,
    components: &BTreeSet<String>,
    resources: &BTreeMap<String, String>,
) -> Option<String> {
    let (kind, rest) = reference.split_once(':')?;
    let name = rest.split_once('/')?.1;
    match kind {
        "component" => {
            let id = format!("component:{name}");
            components.contains(&id).then_some(id)
        }
        "resource" => resources.get(name).cloned(),
        _ => None,
    }
}

fn depends_edge(from_id: &str, to_id: &str, generated_at: &Timestamp) -> Edge {
    let edge_type = EdgeType::DeclaredDependsOn;
    Edge {
        id: ids::edge_id(edge_type, from_id, to_id, None),
        edge_type,
        from_id: from_id.to_owned(),
        to_id: to_id.to_owned(),
        env: None,
        sources: vec![EdgeSource {
            kind: SourceKind::Catalog,
            first_seen: generated_at.clone(),
            last_seen: generated_at.clone(),
            evidence_ref: None,
        }],
        status: EdgeStatus::DeclaredOnly,
        criticality: Default::default(),
        active: true,
        attrs: EdgeAttrs::default(),
    }
}

fn deployed_edge(from_id: &str, to_id: &str, env: Env, stamp: &Timestamp) -> Edge {
    let edge_type = EdgeType::DeployedAs;
    Edge {
        id: ids::edge_id(edge_type, from_id, to_id, Some(env)),
        edge_type,
        from_id: from_id.to_owned(),
        to_id: to_id.to_owned(),
        env: Some(env),
        sources: vec![EdgeSource {
            kind: SourceKind::Static,
            first_seen: stamp.clone(),
            last_seen: stamp.clone(),
            evidence_ref: None,
        }],
        status: EdgeStatus::NotApplicable,
        criticality: Default::default(),
        active: true,
        attrs: EdgeAttrs::default(),
    }
}

fn cmp_deployment(left: &DeploymentCand, right: &DeploymentCand) -> std::cmp::Ordering {
    left.rank
        .cmp(&right.rank)
        .then_with(|| cmp_same_rank(left, right))
        .then_with(|| left.created_at.cmp(&right.created_at))
        .then_with(|| left.version.cmp(&right.version))
}

fn cmp_same_rank(left: &DeploymentCand, right: &DeploymentCand) -> std::cmp::Ordering {
    if left.rank == 3
        && let (Ok(left_version), Ok(right_version)) = (
            Version::parse(&left.version),
            Version::parse(&right.version),
        )
    {
        return left_version.cmp(&right_version);
    }
    std::cmp::Ordering::Equal
}

fn unresolved_row(kind: &str, name: &str, reason: &str) -> Unresolved {
    Unresolved {
        kind: kind.to_owned(),
        name: name.to_owned(),
        reason: reason.to_owned(),
    }
}

fn csv(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[derive(Clone)]
struct DeploymentCand {
    component: String,
    env: Env,
    version: String,
    rank: u8,
    created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_outranks_an_older_semver() {
        assert!(version_rank("v1.4.0").0 > version_rank("latest").0);
        assert_eq!(version_rank("v1.4.0").1, "1.4.0");
        assert_eq!(version_rank("sha256:abcd").0, 4);
    }
}
