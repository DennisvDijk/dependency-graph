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
    /// Call volume, when the row came from telemetry. Zero otherwise.
    pub volume: u64,
}

/// Inputs for one pure build. `generated_at` is the reference time.
pub struct MergeOptions<'a> {
    /// Alias table. Unknown names are not guessed past this table.
    pub aliases: &'a Aliases,
    /// Namespace and source-env map.
    pub environments: &'a EnvIndex,
    /// Reference time for expiry. Not read from the clock.
    pub generated_at: &'a Timestamp,
    /// Run id copied onto the snapshot.
    pub run_id: &'a str,
    /// When false, library records are unresolved instead of nodes.
    pub libraries_enabled: bool,
    /// Days after `last_seen` before an edge is inactive.
    pub edge_expiry_days: u32,
    /// Days after `last_seen` before an edge is dropped.
    pub retention_days: u32,
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
    /// Counts for `meta.json`.
    pub coverage: crate::store::Coverage,
}

/// Merge records. `libraries_enabled` false drops library nodes and edges.
///
/// # Errors
///
/// Returns an error when a record payload is not the expected shape or the
/// resulting graph breaks an invariant.
pub fn merge_records(
    records: &[RawRecord],
    options: &MergeOptions<'_>,
) -> Result<BuildOutput, Error> {
    let aliases = options.aliases;
    let environments = options.environments;
    let generated_at = options.generated_at;
    let run_id = options.run_id;
    let libraries_enabled = options.libraries_enabled;
    let mut components: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut resources: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut resource_links: Vec<(String, String, String)> = Vec::new();
    let mut deployments: Vec<DeploymentCand> = Vec::new();
    let mut unresolved = Vec::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut observed: BTreeMap<ObservedKey, ObservedAgg> = BTreeMap::new();
    let mut candidates: Vec<CandidateCall> = Vec::new();

    for record in records {
        *counts.entry(record.collector.clone()).or_insert(0) += 1;
        match record.kind.as_str() {
            "component" => {
                let name = canonical_name(required_str(&record.payload, "name")?, aliases);
                if !ids::is_kebab_case(&name) {
                    unresolved.push(unresolved_row(
                        "component",
                        &name,
                        "name is not kebab-case",
                        0,
                    ));
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
                        0,
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
                        0,
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
                    0,
                ));
            }
            "observed_call" => absorb_observed(record, &mut observed)?,
            "candidate_call" => {
                candidates.push(CandidateCall {
                    source: required_str(&record.payload, "source")?.to_owned(),
                    destination: required_str(&record.payload, "destination")?.to_owned(),
                    namespace: record
                        .payload
                        .get("namespace")
                        .and_then(|value| value.as_str())
                        .unwrap_or("")
                        .to_owned(),
                });
            }
            other => unresolved.push(unresolved_row("record", other, "ignored record kind", 0)),
        }
    }

    let mut inferred = BTreeSet::new();
    let mut runtime_edges = runtime_edges(&mut RuntimeBuild {
        observed: &observed,
        candidates: &candidates,
        components: &components,
        resources: &mut resources,
        inferred: &mut inferred,
        unresolved: &mut unresolved,
        environments,
        aliases,
        generated_at,
    });

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
        let id = ids::resource_id(parsed, name).map_err(Error::Validation)?;
        let mut attrs = BTreeMap::new();
        if inferred.contains(&id) {
            attrs.insert("inferred".to_owned(), serde_json::Value::Bool(true));
        }
        nodes.push(Node {
            id,
            node_type: NodeType::Resource,
            attrs,
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
                None => unresolved.push(unresolved_row(
                    "depends_on",
                    dep,
                    &format!("from {from}"),
                    0,
                )),
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
                0,
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
    edges.append(&mut runtime_edges);
    assign_status(&mut edges);
    expire_edges(
        &mut edges,
        generated_at,
        options.edge_expiry_days,
        options.retention_days,
    );

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
    collapse_unresolved(&mut unresolved);
    mismatches
        .sort_by(|left, right| (&left.component, &left.env).cmp(&(&right.component, &right.env)));
    let coverage = coverage_of(&graph, &unresolved);
    Ok(BuildOutput {
        graph,
        unresolved,
        mismatches,
        coverage,
    })
}

/// CSV body for unresolved names, including the header and a trailing newline.
#[must_use]
pub fn unresolved_csv(rows: &[Unresolved]) -> String {
    let mut lines = vec!["kind,name,reason,volume".to_owned()];
    for row in rows {
        lines.push(format!(
            "{},{},{},{}",
            csv(&row.kind),
            csv(&row.name),
            csv(&row.reason),
            row.volume
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

fn unresolved_row(kind: &str, name: &str, reason: &str, volume: u64) -> Unresolved {
    Unresolved {
        kind: kind.to_owned(),
        name: name.to_owned(),
        reason: reason.to_owned(),
        volume,
    }
}

fn collapse_unresolved(rows: &mut Vec<Unresolved>) {
    rows.sort_by(|left, right| {
        (&left.kind, &left.name, &left.reason).cmp(&(&right.kind, &right.name, &right.reason))
    });
    let mut collapsed: Vec<Unresolved> = Vec::new();
    for row in rows.drain(..) {
        if let Some(last) = collapsed.last_mut()
            && last.kind == row.kind
            && last.name == row.name
            && last.reason == row.reason
        {
            last.volume += row.volume;
            continue;
        }
        collapsed.push(row);
    }
    *rows = collapsed;
}

fn csv(value: &str) -> String {
    if value.contains([',', '"', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ObservedKey {
    source: String,
    destination: String,
    source_namespace: String,
    destination_namespace: String,
    external: bool,
    external_host: String,
    protocol: String,
    window: String,
}

struct ObservedAgg {
    calls: u64,
    errors: u64,
    p95_ms: Option<f64>,
    last_seen: String,
}

struct CandidateCall {
    source: String,
    destination: String,
    namespace: String,
}

fn absorb_observed(
    record: &RawRecord,
    observed: &mut BTreeMap<ObservedKey, ObservedAgg>,
) -> Result<(), Error> {
    let source = required_str(&record.payload, "source")?;
    let destination = required_str(&record.payload, "destination")?;
    let key = ObservedKey {
        source: source.to_owned(),
        destination: destination.to_owned(),
        source_namespace: text_field(&record.payload, "source_namespace"),
        destination_namespace: text_field(&record.payload, "destination_namespace"),
        external: record
            .payload
            .get("external")
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        external_host: text_field(&record.payload, "external_host"),
        protocol: text_field(&record.payload, "protocol"),
        window: text_field(&record.payload, "window"),
    };
    let calls = json_u64(&record.payload, "calls");
    let errors = json_u64(&record.payload, "errors");
    let last_seen = text_field(&record.payload, "last_seen");
    let p95_ms = record
        .payload
        .get("p95_ms")
        .and_then(|value| value.as_f64())
        .filter(|value| value.is_finite());
    let entry = observed.entry(key).or_insert_with(|| ObservedAgg {
        calls: 0,
        errors: 0,
        p95_ms: None,
        last_seen: last_seen.clone(),
    });
    entry.calls += calls;
    entry.errors += errors;
    if last_seen >= entry.last_seen {
        entry.last_seen = last_seen;
        if p95_ms.is_some() {
            entry.p95_ms = p95_ms;
        }
    }
    Ok(())
}

fn text_field(payload: &serde_json::Value, field: &str) -> String {
    payload
        .get(field)
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_owned()
}

fn json_u64(payload: &serde_json::Value, field: &str) -> u64 {
    payload
        .get(field)
        .and_then(|value| value.as_u64())
        .unwrap_or(0)
}

struct RuntimeBuild<'a> {
    observed: &'a BTreeMap<ObservedKey, ObservedAgg>,
    candidates: &'a [CandidateCall],
    components: &'a BTreeMap<String, BTreeSet<String>>,
    resources: &'a mut BTreeMap<(String, String), String>,
    inferred: &'a mut BTreeSet<String>,
    unresolved: &'a mut Vec<Unresolved>,
    environments: &'a EnvIndex,
    aliases: &'a Aliases,
    generated_at: &'a Timestamp,
}

fn runtime_edges(build: &mut RuntimeBuild<'_>) -> Vec<Edge> {
    let RuntimeBuild {
        observed,
        candidates,
        components,
        resources,
        inferred,
        unresolved,
        environments,
        aliases,
        generated_at,
    } = build;
    let mut edges = Vec::new();
    for (key, agg) in observed.iter() {
        let source = canonical_name(&key.source, aliases);
        let source_id = format!("component:{source}");
        if !components.contains_key(&source) {
            unresolved.push(unresolved_row(
                "observed_call",
                &key.source,
                "source is not a known component",
                agg.calls,
            ));
            continue;
        }
        let env = match observed_env(key, environments) {
            Ok(env) => env,
            Err(message) => {
                unresolved.push(unresolved_row(
                    "environment",
                    &message,
                    "namespace is not mapped",
                    agg.calls,
                ));
                continue;
            }
        };
        let to_id = if key.external {
            let name = if key.external_host.is_empty() {
                key.destination.clone()
            } else {
                key.external_host.clone()
            };
            if !ids::is_kebab_case(&name) {
                unresolved.push(unresolved_row(
                    "observed_call",
                    &name,
                    "external host is not kebab-case",
                    agg.calls,
                ));
                continue;
            }
            let id = format!("resource:external-api:{name}");
            resources.insert(
                ("external-api".to_owned(), name),
                "runtime_edges".to_owned(),
            );
            inferred.insert(id.clone());
            id
        } else {
            let destination = canonical_name(&key.destination, aliases);
            if !components.contains_key(&destination) {
                unresolved.push(unresolved_row(
                    "observed_call",
                    &key.destination,
                    "destination is not a known component",
                    agg.calls,
                ));
                continue;
            }
            format!("component:{destination}")
        };
        edges.push(observed_edge(
            &source_id,
            &to_id,
            env,
            key,
            agg,
            generated_at,
        ));
    }
    for candidate in candidates.iter() {
        let source = canonical_name(&candidate.source, aliases);
        let destination = canonical_name(&candidate.destination, aliases);
        if !components.contains_key(&source) || !components.contains_key(&destination) {
            let missing = if components.contains_key(&source) {
                &candidate.destination
            } else {
                &candidate.source
            };
            unresolved.push(unresolved_row(
                "candidate_call",
                missing,
                "name is not a known component",
                0,
            ));
            continue;
        }
        let env = if candidate.namespace.is_empty() {
            None
        } else {
            match environments.resolve_namespace(&candidate.namespace) {
                Some(env) => Some(env),
                None => {
                    unresolved.push(unresolved_row(
                        "environment",
                        &candidate.namespace,
                        "namespace is not mapped",
                        0,
                    ));
                    continue;
                }
            }
        };
        edges.push(candidate_edge(
            &format!("component:{source}"),
            &format!("component:{destination}"),
            env,
            generated_at,
        ));
    }
    fold_same_id(edges)
}

fn fold_same_id(edges: Vec<Edge>) -> Vec<Edge> {
    let mut folded: BTreeMap<String, Edge> = BTreeMap::new();
    for edge in edges {
        if let Some(existing) = folded.get_mut(&edge.id) {
            merge_measurement(existing, &edge);
        } else {
            folded.insert(edge.id.clone(), edge);
        }
    }
    folded.into_values().collect()
}

fn merge_measurement(existing: &mut Edge, extra: &Edge) {
    let left_calls = existing.attrs.calls_7d.unwrap_or(0);
    let right_calls = extra.attrs.calls_7d.unwrap_or(0);
    let left_errors = existing.attrs.error_rate_7d.unwrap_or(0.0) * left_calls as f64;
    let right_errors = extra.attrs.error_rate_7d.unwrap_or(0.0) * right_calls as f64;
    if existing.attrs.calls_7d.is_some() || extra.attrs.calls_7d.is_some() {
        let calls = left_calls + right_calls;
        existing.attrs.calls_7d = Some(calls);
        if calls > 0 {
            existing.attrs.error_rate_7d = Some((left_errors + right_errors) / calls as f64);
        }
    }
    if existing.attrs.calls_total.is_some() || extra.attrs.calls_total.is_some() {
        existing.attrs.calls_total =
            Some(existing.attrs.calls_total.unwrap_or(0) + extra.attrs.calls_total.unwrap_or(0));
    }
    if existing.attrs.protocol.is_none() {
        existing.attrs.protocol.clone_from(&extra.attrs.protocol);
    }
    if let (Some(current), Some(added)) = (existing.sources.first_mut(), extra.sources.first()) {
        if added.last_seen > current.last_seen {
            current.last_seen = added.last_seen.clone();
            existing.attrs.p95_ms_7d = extra.attrs.p95_ms_7d.or(existing.attrs.p95_ms_7d);
        }
        if added.first_seen < current.first_seen {
            current.first_seen = added.first_seen.clone();
        }
    }
}

fn observed_env(key: &ObservedKey, environments: &EnvIndex) -> Result<Env, String> {
    let source = environments.resolve_namespace(&key.source_namespace);
    if key.external {
        return source.ok_or_else(|| key.source_namespace.clone());
    }
    let destination = environments.resolve_namespace(&key.destination_namespace);
    match (source, destination) {
        (Some(left), Some(right)) if left == right => Ok(left),
        (Some(_), Some(_)) => Err(format!(
            "{} and {}",
            key.source_namespace, key.destination_namespace
        )),
        (None, _) => Err(key.source_namespace.clone()),
        (_, None) => Err(key.destination_namespace.clone()),
    }
}

fn observed_edge(
    from_id: &str,
    to_id: &str,
    env: Env,
    key: &ObservedKey,
    agg: &ObservedAgg,
    generated_at: &Timestamp,
) -> Edge {
    let edge_type = EdgeType::ObservedCalls;
    let last_seen = Timestamp::parse(&agg.last_seen).unwrap_or_else(|_| generated_at.clone());
    let mut attrs = EdgeAttrs::default();
    if key.window == "7d" {
        attrs.calls_7d = Some(agg.calls);
        if agg.calls > 0 {
            let errors = agg.errors.min(agg.calls);
            attrs.error_rate_7d = Some(errors as f64 / agg.calls as f64);
        }
        attrs.p95_ms_7d = agg.p95_ms;
    } else {
        attrs.calls_total = Some(agg.calls);
    }
    if !key.protocol.is_empty() {
        attrs.protocol = Some(key.protocol.clone());
    }
    Edge {
        id: ids::edge_id(edge_type, from_id, to_id, Some(env)),
        edge_type,
        from_id: from_id.to_owned(),
        to_id: to_id.to_owned(),
        env: Some(env),
        sources: vec![EdgeSource {
            kind: SourceKind::Istio,
            first_seen: last_seen.clone(),
            last_seen,
            evidence_ref: Some(format!("prometheus:istio_requests_total[{}]", key.window)),
        }],
        status: EdgeStatus::ObservedOnly,
        criticality: Default::default(),
        active: true,
        attrs,
    }
}

fn candidate_edge(from_id: &str, to_id: &str, env: Option<Env>, generated_at: &Timestamp) -> Edge {
    let edge_type = EdgeType::CandidateCalls;
    Edge {
        id: ids::edge_id(edge_type, from_id, to_id, env),
        edge_type,
        from_id: from_id.to_owned(),
        to_id: to_id.to_owned(),
        env,
        sources: vec![EdgeSource {
            kind: SourceKind::Static,
            first_seen: generated_at.clone(),
            last_seen: generated_at.clone(),
            evidence_ref: Some("manifest".to_owned()),
        }],
        status: EdgeStatus::Candidate,
        criticality: Default::default(),
        active: true,
        attrs: EdgeAttrs::default(),
    }
}

fn assign_status(edges: &mut [Edge]) {
    let observed: BTreeSet<(String, String)> = edges
        .iter()
        .filter(|edge| edge.edge_type == EdgeType::ObservedCalls)
        .map(|edge| (edge.from_id.clone(), edge.to_id.clone()))
        .collect();
    let declared: BTreeSet<(String, String)> = edges
        .iter()
        .filter(|edge| edge.edge_type == EdgeType::DeclaredDependsOn)
        .map(|edge| (edge.from_id.clone(), edge.to_id.clone()))
        .collect();
    for edge in edges {
        let pair = (edge.from_id.clone(), edge.to_id.clone());
        edge.status = match edge.edge_type {
            EdgeType::DeclaredDependsOn => {
                if observed.contains(&pair) {
                    EdgeStatus::Both
                } else {
                    EdgeStatus::DeclaredOnly
                }
            }
            EdgeType::ObservedCalls => {
                if declared.contains(&pair) {
                    EdgeStatus::Both
                } else {
                    EdgeStatus::ObservedOnly
                }
            }
            EdgeType::CandidateCalls => EdgeStatus::Candidate,
            _ => EdgeStatus::NotApplicable,
        };
    }
}

fn expire_edges(
    edges: &mut Vec<Edge>,
    generated_at: &Timestamp,
    edge_expiry_days: u32,
    retention_days: u32,
) {
    let Ok(generated) = generated_at.to_datetime() else {
        return;
    };
    edges.retain(|edge| {
        let last = edge
            .sources
            .iter()
            .filter_map(|source| source.last_seen.to_datetime().ok())
            .max();
        let Some(last) = last else {
            return true;
        };
        let age = (generated - last).num_days().max(0);
        age < i64::from(retention_days)
    });
    for edge in edges {
        let last = edge
            .sources
            .iter()
            .filter_map(|source| source.last_seen.to_datetime().ok())
            .max();
        if let Some(last) = last {
            let age = (generated - last).num_days().max(0);
            if age >= i64::from(edge_expiry_days) {
                edge.active = false;
            }
        }
    }
}

fn coverage_of(graph: &GraphSnapshot, unresolved: &[Unresolved]) -> crate::store::Coverage {
    let mut coverage = crate::store::Coverage {
        components: graph
            .nodes
            .iter()
            .filter(|node| node.node_type == NodeType::Component)
            .count() as u64,
        unresolved: unresolved.len() as u64,
        inferred_resources: graph
            .nodes
            .iter()
            .filter(|node| {
                node.attrs
                    .get("inferred")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false)
            })
            .count() as u64,
        ..crate::store::Coverage::default()
    };
    for edge in &graph.edges {
        match edge.edge_type {
            EdgeType::DeclaredDependsOn => coverage.declared_edges += 1,
            EdgeType::ObservedCalls => coverage.observed_edges += 1,
            _ => {}
        }
        match edge.status {
            EdgeStatus::Both => coverage.both += 1,
            EdgeStatus::ObservedOnly => coverage.observed_only += 1,
            EdgeStatus::DeclaredOnly => coverage.declared_only += 1,
            EdgeStatus::Candidate => coverage.candidates += 1,
            EdgeStatus::NotApplicable => {}
        }
    }
    coverage
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
