//! Graph invariants. Fail hard: a snapshot is valid or it is not stored.
//!
//! The checks are pure. They do not consult the clock, the network, or a
//! random source.

use std::collections::BTreeSet;

use semver::Version;

use crate::config::Aliases;
use crate::error::Error;
use crate::ids::{self, ParsedNodeId};
use crate::models::{EdgeType, GraphSnapshot, NodeType};
use crate::redact::{self, SecretHit};

/// One broken invariant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// Stable code for tests and CI output.
    pub code: &'static str,
    /// What failed. Secret text is redacted.
    pub detail: String,
}

/// Every invariant that failed. Empty means the graph is valid.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub struct ValidationReport {
    /// Failures, sorted by code then detail.
    pub errors: Vec<Violation>,
}

impl ValidationReport {
    /// Whether every invariant holds.
    #[must_use]
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
    }

    /// Codes in report order.
    #[must_use]
    pub fn codes(&self) -> Vec<&'static str> {
        self.errors.iter().map(|error| error.code).collect()
    }

    /// Convert a failed report into [`Error::Validation`].
    ///
    /// # Errors
    ///
    /// Returns an error when the report is not empty.
    pub fn into_result(self) -> Result<(), Error> {
        if self.is_ok() {
            Ok(())
        } else {
            Err(Error::Validation(self.to_string()))
        }
    }
}

impl std::fmt::Display for ValidationReport {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(formatter, "validation failed")?;
        for error in &self.errors {
            writeln!(formatter, "- {}: {}", error.code, error.detail)?;
        }
        Ok(())
    }
}

/// Check the snapshot and, when provided, the alias table.
///
/// Alias checks are skipped when `aliases` is `None`. An empty alias table
/// still requires every listed target to be a component in the graph.
pub fn validate(graph: &GraphSnapshot, aliases: Option<&Aliases>) -> ValidationReport {
    let mut errors = Vec::new();
    check_header(graph, &mut errors);
    let component_ids = check_nodes(graph, &mut errors);
    check_edges(graph, &mut errors);
    check_secrets(graph, &mut errors);
    if let Some(aliases) = aliases {
        check_aliases(aliases, &component_ids, &mut errors);
    }
    errors.sort_by(|left, right| {
        left.code
            .cmp(right.code)
            .then_with(|| left.detail.cmp(&right.detail))
    });
    errors.dedup();
    ValidationReport { errors }
}

fn check_header(graph: &GraphSnapshot, errors: &mut Vec<Violation>) {
    if Version::parse(&graph.schema_version).is_err() {
        errors.push(violation(
            "schema_version",
            format!("schema_version `{}` is not semver", graph.schema_version),
        ));
    }
    if !valid_run_id(&graph.run_id) {
        errors.push(violation(
            "run_id",
            format!("run_id `{}` must be a short token", graph.run_id),
        ));
    }
    if graph.generated_at.to_datetime().is_err() {
        errors.push(violation("bad_timestamp", "generated_at is not RFC 3339"));
    }
    for (name, summary) in &graph.sources {
        if let Some(finished_at) = &summary.finished_at
            && finished_at.to_datetime().is_err()
        {
            errors.push(violation(
                "bad_timestamp",
                format!("sources.{name}.finished_at is not RFC 3339"),
            ));
        }
    }
}

fn check_nodes(graph: &GraphSnapshot, errors: &mut Vec<Violation>) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut component_ids = BTreeSet::new();
    for node in &graph.nodes {
        if !seen.insert(node.id.clone()) {
            errors.push(violation(
                "duplicate_node_id",
                format!("node id `{}` is duplicated", node.id),
            ));
        }
        match (node.node_type, ids::parse_node_id(&node.id)) {
            (NodeType::Component, Ok(ParsedNodeId::Component { .. })) => {
                component_ids.insert(node.id.clone());
            }
            (NodeType::Deployment, Ok(ParsedNodeId::Deployment { name, .. })) => {
                let component = format!("component:{name}");
                if !graph.nodes.iter().any(|candidate| {
                    candidate.node_type == NodeType::Component && candidate.id == component
                }) {
                    errors.push(violation(
                        "deployment_missing_component",
                        format!("deployment `{}` references missing `{component}`", node.id),
                    ));
                }
            }
            (NodeType::Resource, Ok(ParsedNodeId::Resource { .. }))
            | (NodeType::Library, Ok(ParsedNodeId::Library { .. })) => {}
            (_, Err(detail)) => errors.push(violation("node_id_convention", detail)),
            (_, Ok(_)) => errors.push(violation(
                "node_type_mismatch",
                format!(
                    "node `{}` has type `{}` that does not match its id",
                    node.id,
                    node.node_type.as_str()
                ),
            )),
        }
    }
    component_ids
}

fn check_edges(graph: &GraphSnapshot, errors: &mut Vec<Violation>) {
    let node_ids: BTreeSet<&str> = graph.nodes.iter().map(|node| node.id.as_str()).collect();
    let mut seen = BTreeSet::new();
    for edge in &graph.edges {
        if !seen.insert(edge.id.clone()) {
            errors.push(violation(
                "duplicate_edge_id",
                format!("edge id `{}` is duplicated", edge.id),
            ));
        }
        let expected = ids::edge_id(edge.edge_type, &edge.from_id, &edge.to_id, edge.env);
        if edge.id != expected {
            errors.push(violation(
                "edge_id_mismatch",
                format!("edge `{}` does not match stable id `{expected}`", edge.id),
            ));
        }
        if !node_ids.contains(edge.from_id.as_str()) {
            errors.push(violation(
                "dangling_edge",
                format!(
                    "edge `{}` from_id `{}` is not a node",
                    edge.id, edge.from_id
                ),
            ));
        }
        if !node_ids.contains(edge.to_id.as_str()) {
            errors.push(violation(
                "dangling_edge",
                format!("edge `{}` to_id `{}` is not a node", edge.id, edge.to_id),
            ));
        }
        if edge.edge_type == EdgeType::ObservedCalls && edge.env.is_none() {
            errors.push(violation(
                "env_required",
                format!("observed_calls edge `{}` needs an environment", edge.id),
            ));
        }
        if edge.sources.is_empty() {
            errors.push(violation(
                "missing_source",
                format!("edge `{}` needs at least one source", edge.id),
            ));
        }
        for (index, source) in edge.sources.iter().enumerate() {
            match (
                source.first_seen.to_datetime(),
                source.last_seen.to_datetime(),
            ) {
                (Ok(first), Ok(last)) if first <= last => {}
                (Ok(_), Ok(_)) => errors.push(violation(
                    "source_time_order",
                    format!(
                        "edge `{}` source {index} has first_seen after last_seen",
                        edge.id
                    ),
                )),
                (Err(_), other) => {
                    errors.push(violation(
                        "bad_timestamp",
                        format!(
                            "edge `{}` source {index} first_seen is not RFC 3339",
                            edge.id
                        ),
                    ));
                    if other.is_err() {
                        errors.push(violation(
                            "bad_timestamp",
                            format!(
                                "edge `{}` source {index} last_seen is not RFC 3339",
                                edge.id
                            ),
                        ));
                    }
                }
                (Ok(_), Err(_)) => errors.push(violation(
                    "bad_timestamp",
                    format!(
                        "edge `{}` source {index} last_seen is not RFC 3339",
                        edge.id
                    ),
                )),
            }
        }
    }
}

fn check_secrets(graph: &GraphSnapshot, errors: &mut Vec<Violation>) {
    let value = match serde_json::to_value(graph) {
        Ok(value) => value,
        Err(err) => {
            errors.push(violation(
                "secret_material",
                format!("graph could not be scanned: {err}"),
            ));
            return;
        }
    };
    for hit in redact::find_secrets(&value) {
        errors.push(secret_violation(hit));
    }
}

fn check_aliases(aliases: &Aliases, component_ids: &BTreeSet<String>, errors: &mut Vec<Violation>) {
    for (alias, target) in &aliases.aliases {
        if !ids::is_kebab_case(target) {
            errors.push(violation(
                "alias_target_invalid",
                format!("alias `{alias}` target `{target}` is not a component name"),
            ));
            continue;
        }
        let component_id = format!("component:{target}");
        if !component_ids.contains(&component_id) {
            errors.push(violation(
                "alias_unknown_component",
                format!("alias `{alias}` maps to missing `{component_id}`"),
            ));
        }
        if ids::is_kebab_case(alias) {
            let alias_as_component = format!("component:{alias}");
            if component_ids.contains(&alias_as_component) && alias_as_component != component_id {
                errors.push(violation(
                    "alias_ambiguous",
                    format!(
                        "alias `{alias}` maps to `{component_id}` and also matches `{alias_as_component}`"
                    ),
                ));
            }
        }
        if let Some(kind) = redact::secret_kind(alias).or_else(|| redact::secret_kind(target)) {
            errors.push(violation(
                "secret_material",
                format!("alias `{alias}` contains {kind}"),
            ));
        }
    }
}

fn valid_run_id(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && value.len() <= 201
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | ':' | '-')
        })
}

fn violation(code: &'static str, detail: impl Into<String>) -> Violation {
    Violation {
        code,
        detail: redact::redact(&detail.into()),
    }
}

fn secret_violation(hit: SecretHit) -> Violation {
    violation(
        "secret_material",
        format!("{} contains {}", hit.path, hit.kind),
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::config::Aliases;
    use crate::ids::{self, ResourceKind};
    use crate::models::{
        Criticality, Edge, EdgeAttrs, EdgeSource, EdgeStatus, EdgeType, Env, GraphSnapshot, Node,
        NodeType, SourceKind, Timestamp,
    };

    fn stamp(value: &str) -> Timestamp {
        Timestamp::parse(value).unwrap()
    }

    fn source(first: &str, last: &str) -> EdgeSource {
        EdgeSource {
            kind: SourceKind::Catalog,
            first_seen: stamp(first),
            last_seen: stamp(last),
            evidence_ref: None,
        }
    }

    fn empty() -> GraphSnapshot {
        GraphSnapshot {
            schema_version: "0.1.0".to_owned(),
            run_id: "empty".to_owned(),
            generated_at: stamp("2026-01-01T00:00:00Z"),
            sources: BTreeMap::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    fn component(name: &str) -> Node {
        Node {
            id: ids::component_id(name).unwrap(),
            node_type: NodeType::Component,
            attrs: BTreeMap::new(),
        }
    }

    fn edge(
        edge_type: EdgeType,
        from_id: &str,
        to_id: &str,
        env: Option<Env>,
        source: EdgeSource,
    ) -> Edge {
        Edge {
            id: ids::edge_id(edge_type, from_id, to_id, env),
            edge_type,
            from_id: from_id.to_owned(),
            to_id: to_id.to_owned(),
            env,
            sources: vec![source],
            status: EdgeStatus::NotApplicable,
            criticality: Criticality::Unknown,
            active: true,
            attrs: EdgeAttrs::default(),
        }
    }

    #[test]
    fn empty_graph_is_valid() {
        let report = validate(&empty(), Some(&Aliases::default()));
        assert!(report.is_ok(), "{report}");
    }

    #[test]
    fn deployment_must_reference_a_component() {
        let mut graph = empty();
        graph.nodes.push(Node {
            id: ids::deployment_id("payments-api", Env::Prod).unwrap(),
            node_type: NodeType::Deployment,
            attrs: BTreeMap::new(),
        });
        let report = validate(&graph, None);
        assert!(report.codes().contains(&"deployment_missing_component"));

        graph.nodes.push(component("payments-api"));
        assert!(validate(&graph, None).is_ok(), "{report}");
    }

    #[test]
    fn dangling_observed_time_and_secret_edges_fail() {
        let mut graph = empty();
        graph.nodes.push(component("payments-api"));
        graph.nodes.push(component("billing-api"));
        let mut observed = edge(
            EdgeType::ObservedCalls,
            "component:payments-api",
            "component:billing-api",
            None,
            source("2026-02-01T00:00:00Z", "2026-01-01T00:00:00Z"),
        );
        observed.sources.clear();
        graph.edges.push(observed);
        graph.edges.push(edge(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:missing",
            None,
            source("2026-01-01T00:00:00Z", "2026-01-02T00:00:00Z"),
        ));
        graph.nodes[0].attrs.insert(
            "url".to_owned(),
            serde_json::Value::String("https://user:password@example.com".to_owned()),
        );
        let report = validate(&graph, None);
        let codes = report.codes();
        assert!(codes.contains(&"env_required"));
        assert!(codes.contains(&"missing_source"));
        assert!(codes.contains(&"dangling_edge"));
        assert!(codes.contains(&"secret_material"));
        assert!(!report.to_string().contains("password"));
    }

    #[test]
    fn source_time_order_and_bad_ids_fail() {
        let mut graph = empty();
        graph.nodes.push(component("payments-api"));
        graph.nodes.push(Node {
            id: "component:Not-Kebab".to_owned(),
            node_type: NodeType::Component,
            attrs: BTreeMap::new(),
        });
        graph.edges.push(edge(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:payments-api",
            None,
            source("2026-02-01T00:00:00Z", "2026-01-01T00:00:00Z"),
        ));
        let report = validate(&graph, None);
        assert!(report.codes().contains(&"node_id_convention"));
        assert!(report.codes().contains(&"source_time_order"));
    }

    #[test]
    fn edge_id_must_be_the_stable_hash() {
        let mut graph = empty();
        graph.nodes.push(component("payments-api"));
        graph.nodes.push(component("billing-api"));
        let mut declared = edge(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:billing-api",
            None,
            source("2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"),
        );
        declared.id = "edge:not-the-hash".to_owned();
        graph.edges.push(declared);
        assert!(validate(&graph, None).codes().contains(&"edge_id_mismatch"));
    }

    #[test]
    fn aliases_must_name_one_existing_component() {
        let mut graph = empty();
        graph.nodes.push(component("payments-api"));
        graph.nodes.push(component("billing-api"));
        let mut aliases = Aliases::default();
        aliases
            .aliases
            .insert("payments".to_owned(), "payments-api".to_owned());
        assert!(validate(&graph, Some(&aliases)).is_ok());

        aliases
            .aliases
            .insert("billing-api".to_owned(), "payments-api".to_owned());
        let report = validate(&graph, Some(&aliases));
        assert!(report.codes().contains(&"alias_ambiguous"));

        aliases.aliases.clear();
        aliases
            .aliases
            .insert("payments".to_owned(), "missing".to_owned());
        assert!(
            validate(&graph, Some(&aliases))
                .codes()
                .contains(&"alias_unknown_component")
        );
    }

    #[test]
    fn a_full_fixture_shape_is_valid() {
        let mut graph = empty();
        graph.run_id = "sample".to_owned();
        graph.nodes.push(component("payments-api"));
        graph.nodes.push(component("billing-api"));
        graph.nodes.push(Node {
            id: ids::deployment_id("payments-api", Env::Prod).unwrap(),
            node_type: NodeType::Deployment,
            attrs: BTreeMap::new(),
        });
        graph.nodes.push(Node {
            id: ids::resource_id(ResourceKind::Database, "payments-db").unwrap(),
            node_type: NodeType::Resource,
            attrs: BTreeMap::new(),
        });
        graph.nodes.push(Node {
            id: ids::library_id("pkg:cargo/serde").unwrap(),
            node_type: NodeType::Library,
            attrs: BTreeMap::new(),
        });
        graph.edges.push(edge(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:billing-api",
            None,
            source("2026-01-01T00:00:00Z", "2026-01-15T00:00:00Z"),
        ));
        let mut observed = edge(
            EdgeType::ObservedCalls,
            "component:payments-api",
            "component:billing-api",
            Some(Env::Prod),
            EdgeSource {
                kind: SourceKind::Otel,
                first_seen: stamp("2026-01-01T00:00:00Z"),
                last_seen: stamp("2026-01-15T00:00:00Z"),
                evidence_ref: Some("tempo:sample".to_owned()),
            },
        );
        observed.status = EdgeStatus::ObservedOnly;
        observed.attrs.calls_7d = Some(10);
        graph.edges.push(observed);
        graph.edges.push(edge(
            EdgeType::DeployedAs,
            "component:payments-api",
            "deployment:payments-api@prod",
            Some(Env::Prod),
            source("2026-01-15T00:00:00Z", "2026-01-15T00:00:00Z"),
        ));
        graph.edges.push(edge(
            EdgeType::UsesLibrary,
            "component:payments-api",
            "library:pkg:cargo/serde",
            None,
            EdgeSource {
                kind: SourceKind::SbomHint,
                first_seen: stamp("2026-01-15T00:00:00Z"),
                last_seen: stamp("2026-01-15T00:00:00Z"),
                evidence_ref: None,
            },
        ));
        assert!(validate(&graph, None).is_ok());
        let canonical = crate::models::to_canonical_json(&graph).unwrap();
        let again = crate::models::to_canonical_json(&graph).unwrap();
        graph.nodes.reverse();
        graph.edges.reverse();
        assert_eq!(canonical, again);
        assert_eq!(canonical, crate::models::to_canonical_json(&graph).unwrap());
        assert!(
            canonical.find("\"id\": \"component:billing-api\"").unwrap()
                < canonical
                    .find("\"id\": \"component:payments-api\"")
                    .unwrap()
        );
    }

    #[test]
    fn duplicate_node_ids_fail() {
        let mut graph = empty();
        graph.nodes.push(component("payments-api"));
        graph.nodes.push(component("payments-api"));
        assert!(
            validate(&graph, None)
                .codes()
                .contains(&"duplicate_node_id")
        );
    }
}
