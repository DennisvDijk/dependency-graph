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
    Criticality, CriticalitySource, Edge, EdgeAttrs, EdgeSource, EdgeStatus, EdgeType, Env,
    GraphSnapshot, Node, NodeType, SourceKind, SourceSummary, Timestamp,
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
    // (component from label, project path, resource kind, resource name)
    let mut resource_links: Vec<(Option<String>, String, String, String)> = Vec::new();
    // Indexes into `resource_links` whose component name is matched loosely.
    let mut loose_links: Vec<usize> = Vec::new();
    let mut deployments: Vec<DeploymentCand> = Vec::new();
    let mut unresolved = Vec::new();
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let mut observed: BTreeMap<ObservedKey, ObservedAgg> = BTreeMap::new();
    let mut candidates: Vec<CandidateCall> = Vec::new();
    let mut soft_deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut heuristics: BTreeMap<String, bool> = BTreeMap::new();
    let mut experiments: BTreeMap<(String, String), bool> = BTreeMap::new();
    let mut owners: BTreeMap<String, (String, String)> = BTreeMap::new();
    // GitLab project path -> components defined in it. A manifest resource
    // without its own workload label belongs to the component of its repo.
    let mut project_components: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    // GitLab project path -> (languages, has a Dockerfile or compose file).
    let mut stacks: BTreeMap<String, (BTreeSet<String>, bool)> = BTreeMap::new();

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
                let project_path = text_field(&record.payload, "project_path");
                if !project_path.is_empty() {
                    project_components
                        .entry(project_path)
                        .or_default()
                        .insert(name.clone());
                }
                let entry = components.entry(name.clone()).or_default();
                for dep in deps {
                    if let Some(dep) = dep.as_str() {
                        entry.insert(dep.to_owned());
                    }
                }
                let soft: Vec<String> = string_list(&record.payload, "soft_dependencies")
                    .iter()
                    .map(|entry| soft_name(entry, aliases))
                    .filter(|entry| !entry.is_empty())
                    .collect();
                if !soft.is_empty() {
                    soft_deps.entry(name.clone()).or_default().extend(soft);
                }
                if string_list(&record.payload, "heuristics")
                    .iter()
                    .any(|token| crate::classify::is_soft_heuristic(token))
                {
                    heuristics.insert(name.clone(), true);
                }
                let owner = text_field(&record.payload, "owner");
                let system = text_field(&record.payload, "system");
                if !owner.is_empty() || !system.is_empty() {
                    owners.entry(name).or_insert((owner, system));
                }
            }
            "chaos_experiment" => {
                let client = canonical_name(required_str(&record.payload, "client")?, aliases);
                let dependency =
                    canonical_name(required_str(&record.payload, "dependency")?, aliases);
                let survived = record
                    .payload
                    .get("survived")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                experiments.insert((client, dependency), survived);
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
                // Catalog resources are standalone; manifest resources belong
                // to a component, named by label or else found by repo below.
                if record.collector != "gitlab_inventory" {
                    let component = record
                        .payload
                        .get("component")
                        .and_then(|value| value.as_str())
                        .map(|component| canonical_name(component, aliases));
                    if record
                        .payload
                        .get("loose")
                        .and_then(|value| value.as_bool())
                        == Some(true)
                    {
                        loose_links.push(resource_links.len());
                    }
                    resource_links.push((
                        component,
                        text_field(&record.payload, "project_path"),
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
                    external: record
                        .payload
                        .get("external")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                    destination_kind: record
                        .payload
                        .get("destination_kind")
                        .and_then(|value| value.as_str())
                        .map(str::to_owned),
                    loose: record
                        .payload
                        .get("loose")
                        .and_then(|value| value.as_bool())
                        .unwrap_or(false),
                });
            }
            "project_facts" => {
                let project_path = text_field(&record.payload, "project_path");
                let entry = stacks.entry(project_path).or_default();
                entry.0.extend(string_list(&record.payload, "languages"));
                entry.1 |= record
                    .payload
                    .get("container")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
            }
            other => unresolved.push(unresolved_row("record", other, "ignored record kind", 0)),
        }
    }

    // Security groups and other cloud resources name their service loosely
    // (`orders-py-prod-sg`). Match to a known component or drop the resource.
    for index in loose_links {
        let (label, _, kind, name) = &resource_links[index];
        let matched = label.as_deref().and_then(|label| {
            loose_match(
                label,
                |name| components.contains_key(name),
                aliases,
                environments,
            )
        });
        if matched.is_none() {
            unresolved.push(unresolved_row(
                "resource",
                label.as_deref().unwrap_or(name),
                "no component matches this cloud resource",
                0,
            ));
            resources.remove(&(kind.clone(), name.clone()));
        }
        resource_links[index].0 = matched;
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

    let mut component_stack: BTreeMap<&str, (BTreeSet<String>, bool)> = BTreeMap::new();
    for (project_path, names) in &project_components {
        let Some((languages, container)) = stacks.get(project_path) else {
            continue;
        };
        for name in names {
            let entry = component_stack.entry(name.as_str()).or_default();
            entry.0.extend(languages.iter().cloned());
            entry.1 |= *container;
        }
    }
    let mut nodes = Vec::new();
    let mut edges = Vec::new();
    for name in components.keys() {
        let mut attrs = BTreeMap::new();
        if let Some((languages, container)) = component_stack.get(name.as_str()) {
            if !languages.is_empty() {
                attrs.insert(
                    "languages".into(),
                    serde_json::Value::from(languages.iter().cloned().collect::<Vec<_>>()),
                );
            }
            if *container {
                attrs.insert("container".into(), serde_json::Value::Bool(true));
            }
        }
        if let Some((owner, system)) = owners.get(name) {
            if !owner.is_empty() {
                attrs.insert("owner".into(), serde_json::Value::String(owner.clone()));
            }
            if !system.is_empty() {
                attrs.insert("system".into(), serde_json::Value::String(system.clone()));
            }
        }
        nodes.push(Node {
            id: ids::component_id(name).map_err(Error::Validation)?,
            node_type: NodeType::Component,
            attrs,
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

    // A dependency named only in `depgraph/soft-dependencies` is still a
    // dependency: give it an edge, marked soft below.
    let mut declared: BTreeMap<String, BTreeSet<String>> = components.clone();
    for (from, names) in &soft_deps {
        let Some(deps) = declared.get_mut(from) else {
            continue;
        };
        for name in names {
            let known = deps
                .iter()
                .any(|dep| dep.rsplit('/').next().unwrap_or(dep) == name);
            if known {
                continue;
            }
            if component_ids.contains(&format!("component:{name}")) {
                deps.insert(format!("component:default/{name}"));
            } else if resource_ids.contains_key(name) {
                deps.insert(format!("resource:default/{name}"));
            } else {
                unresolved.push(unresolved_row(
                    "soft_dependency",
                    name,
                    &format!("from {from}"),
                    0,
                ));
            }
        }
    }
    for (from, deps) in &declared {
        let from_id = format!("component:{from}");
        for dep in deps {
            match resolve_ref(dep, &component_ids, &resource_ids) {
                Some(to_id) => {
                    let dep_name = dep.rsplit('/').next().unwrap_or(dep);
                    let evidence = crate::classify::Evidence {
                        declared_soft: soft_deps
                            .get(from)
                            .is_some_and(|names| names.iter().any(|name| name == dep_name)),
                        experiment_survived: experiments
                            .get(&(from.clone(), dep_name.to_owned()))
                            .copied(),
                        heuristic: heuristics.get(from).copied().unwrap_or(false),
                    };
                    let (criticality, criticality_source) = crate::classify::classify(evidence);
                    edges.push(depends_edge(
                        &from_id,
                        &to_id,
                        generated_at,
                        criticality,
                        criticality_source,
                        SourceKind::Catalog,
                    ));
                }
                None => unresolved.push(unresolved_row(
                    "depends_on",
                    dep,
                    &format!("from {from}"),
                    0,
                )),
            }
        }
    }
    let mut linked = BTreeSet::new();
    for (label, project_path, kind, name) in &resource_links {
        let to_id = format!("resource:{kind}:{name}");
        if !resources.contains_key(&(kind.clone(), name.clone())) {
            continue;
        }
        let by_label = label
            .as_ref()
            .map(|component| format!("component:{component}"))
            .filter(|id| component_ids.contains(id));
        // Fall back to the one component defined in the same repository.
        let by_repo = || {
            let owners = project_components.get(project_path)?;
            if owners.len() != 1 {
                return None;
            }
            let id = format!("component:{}", owners.iter().next()?);
            component_ids.contains(&id).then_some(id)
        };
        let Some(from_id) = by_label.or_else(by_repo) else {
            continue;
        };
        if from_id == to_id || !linked.insert((from_id.clone(), to_id.clone())) {
            continue;
        }
        edges.push(depends_edge(
            &from_id,
            &to_id,
            generated_at,
            Default::default(),
            Default::default(),
            SourceKind::Static,
        ));
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
    inherit_criticality(&mut edges);
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

/// `depgraph/soft-dependencies` entries may be a bare name, a GitLab project
/// path (`group/notifications`) or an entity ref (`component:default/x`).
fn soft_name(entry: &str, aliases: &Aliases) -> String {
    let entry = entry.trim();
    let last = entry.rsplit('/').next().unwrap_or(entry);
    let last = last.rsplit(':').next().unwrap_or(last);
    canonical_name(last, aliases)
}

/// Words that cloud resource names add around a service name.
const NAME_NOISE: &[&str] = &[
    "sg",
    "secgroup",
    "security",
    "group",
    "svc",
    "service",
    "app",
    "ecs",
    "eks",
    "task",
    "lambda",
    "fn",
    "function",
    "alb",
    "nlb",
    "elb",
    "lb",
    "allow",
    "rule",
    "rules",
    "internal",
    "private",
    "public",
    "dev",
    "development",
    "acc",
    "acceptance",
    "test",
    "tst",
    "qa",
    "staging",
    "stage",
    "stg",
    "prod",
    "production",
    "prd",
];

/// Find a known name inside a cloud resource name: try the name as is (and
/// its alias), then keep dropping a noise word or environment name from
/// either end. Returns `None` when nothing known is found; nothing is guessed.
fn loose_match(
    name: &str,
    known: impl Fn(&str) -> bool,
    aliases: &Aliases,
    environments: &EnvIndex,
) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    let mut parts: Vec<&str> = lower
        .split(['-', '_', '.'])
        .filter(|part| !part.is_empty())
        .collect();
    let noise = |part: &str| NAME_NOISE.contains(&part) || environments.resolve(part).is_some();
    while !parts.is_empty() {
        let candidate = canonical_name(&parts.join("-"), aliases);
        if known(&candidate) {
            return Some(candidate);
        }
        if noise(parts[0]) {
            parts.remove(0);
        } else if noise(parts[parts.len() - 1]) {
            parts.pop();
        } else {
            return None;
        }
    }
    None
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

fn depends_edge(
    from_id: &str,
    to_id: &str,
    generated_at: &Timestamp,
    criticality: crate::models::Criticality,
    criticality_source: crate::models::CriticalitySource,
    source: SourceKind,
) -> Edge {
    let edge_type = EdgeType::DeclaredDependsOn;
    Edge {
        id: ids::edge_id(edge_type, from_id, to_id, None),
        edge_type,
        from_id: from_id.to_owned(),
        to_id: to_id.to_owned(),
        env: None,
        sources: vec![EdgeSource {
            kind: source,
            first_seen: generated_at.clone(),
            last_seen: generated_at.clone(),
            evidence_ref: None,
        }],
        status: EdgeStatus::DeclaredOnly,
        criticality,
        criticality_source,
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
        criticality_source: Default::default(),
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
    external: bool,
    /// A resource kind (`database`) when the destination is not a component.
    destination_kind: Option<String>,
    /// Names come from cloud resources and are matched with `loose_match`.
    loose: bool,
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

fn string_list(payload: &serde_json::Value, field: &str) -> Vec<String> {
    payload
        .get(field)
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
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
        let is_component = |name: &str| components.contains_key(name);
        let resolve = |name: &str| {
            if candidate.loose {
                loose_match(name, is_component, aliases, environments)
                    .unwrap_or_else(|| canonical_name(name, aliases))
            } else {
                canonical_name(name, aliases)
            }
        };
        let source = resolve(&candidate.source);
        if let Some(kind) = candidate.destination_kind.as_deref() {
            if !components.contains_key(&source) {
                unresolved.push(unresolved_row(
                    "candidate_call",
                    &candidate.source,
                    "name is not a known component",
                    0,
                ));
                continue;
            }
            if ResourceKind::parse(kind).is_none() || !ids::is_kebab_case(&candidate.destination) {
                continue;
            }
            let name = loose_match(
                &candidate.destination,
                |name| resources.contains_key(&(kind.to_owned(), name.to_owned())),
                aliases,
                environments,
            )
            .unwrap_or_else(|| candidate.destination.clone());
            let id = format!("resource:{kind}:{name}");
            if !resources.contains_key(&(kind.to_owned(), name.clone())) {
                resources.insert((kind.to_owned(), name), "k8s_workloads".to_owned());
                inferred.insert(id.clone());
            }
            edges.push(candidate_edge(
                &format!("component:{source}"),
                &id,
                None,
                generated_at,
            ));
            continue;
        }
        let destination = resolve(&candidate.destination);
        let destination_known = candidate.external || components.contains_key(&destination);
        if !components.contains_key(&source) || !destination_known {
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
        let to_id = if candidate.external {
            let id = format!("resource:external-api:{destination}");
            resources
                .entry(("external-api".to_owned(), destination.clone()))
                .or_insert_with(|| "k8s_workloads".to_owned());
            inferred.insert(id.clone());
            id
        } else {
            format!("component:{destination}")
        };
        edges.push(candidate_edge(
            &format!("component:{source}"),
            &to_id,
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
        (None, None) if key.source_namespace != key.destination_namespace => Err(format!(
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
        criticality_source: Default::default(),
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
        criticality_source: Default::default(),
        active: true,
        attrs: EdgeAttrs::default(),
    }
}

/// Runtime edges carry the call volume that path scores use, so they take the
/// hard/soft classification of the declared edge between the same pair.
fn inherit_criticality(edges: &mut [Edge]) {
    let declared: BTreeMap<(String, String), (Criticality, CriticalitySource)> = edges
        .iter()
        .filter(|edge| {
            edge.edge_type == EdgeType::DeclaredDependsOn
                && edge.criticality != Criticality::Unknown
        })
        .map(|edge| {
            (
                (edge.from_id.clone(), edge.to_id.clone()),
                (edge.criticality, edge.criticality_source),
            )
        })
        .collect();
    for edge in edges.iter_mut().filter(|edge| {
        matches!(
            edge.edge_type,
            EdgeType::ObservedCalls | EdgeType::CandidateCalls
        ) && edge.criticality == Criticality::Unknown
    }) {
        if let Some((criticality, source)) =
            declared.get(&(edge.from_id.clone(), edge.to_id.clone()))
        {
            edge.criticality = *criticality;
            edge.criticality_source = *source;
        }
    }
}

/// Network policies, firewall rules and IaC never show up in mesh traffic, so
/// comparing them with observed calls would only produce noise.
pub(crate) fn is_structural(to_id: &str) -> bool {
    [
        "resource:infra:",
        "resource:network-policy:",
        "resource:firewall-rule:",
    ]
    .iter()
    .any(|prefix| to_id.starts_with(prefix))
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
            EdgeType::DeclaredDependsOn if is_structural(&edge.to_id) => EdgeStatus::NotApplicable,
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
