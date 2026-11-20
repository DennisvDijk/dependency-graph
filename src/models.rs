//! Graph snapshot model. Field order is the canonical JSON key order.
//!
//! `schema_version` is semver. Nodes and edges are sorted by id before a
//! snapshot is written. Optional edge metrics stay absent until a collector
//! fills them; `status`, `criticality`, and `active` always have defaults.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// RFC 3339 timestamp stored as text so the schema stays a string.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// Parse an RFC 3339 timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error when `value` is not RFC 3339.
    pub fn parse(value: impl Into<String>) -> Result<Self, String> {
        let timestamp = Self(value.into());
        timestamp.to_datetime()?;
        Ok(timestamp)
    }

    /// Borrow the raw RFC 3339 text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Convert to UTC.
    ///
    /// # Errors
    ///
    /// Returns an error when the stored text is not RFC 3339.
    pub fn to_datetime(&self) -> Result<jiff::Timestamp, String> {
        self.0
            .parse::<jiff::Timestamp>()
            .map_err(|err| err.to_string())
    }
}

/// Canonical environment. Source names such as `staging` map onto these.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Env {
    /// Development.
    Dev,
    /// Acceptance.
    Acc,
    /// Production.
    Prod,
}

impl Env {
    /// Stable wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Dev => "dev",
            Self::Acc => "acc",
            Self::Prod => "prod",
        }
    }
}

impl std::fmt::Display for Env {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Node kind. The id prefix must agree with this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NodeType {
    /// A deployable or library-producing component.
    Component,
    /// One component running in one environment.
    Deployment,
    /// A database, topic, queue, or other shared resource.
    Resource,
    /// A versionless package URL.
    Library,
    /// A discovered Kubernetes cluster.
    Cluster,
    /// A discovered Kubernetes workload object.
    Workload,
}

impl NodeType {
    /// Stable wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Component => "component",
            Self::Deployment => "deployment",
            Self::Resource => "resource",
            Self::Library => "library",
            Self::Cluster => "cluster",
            Self::Workload => "workload",
        }
    }
}

/// How two nodes are related.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum EdgeType {
    /// Declared in catalog or contract data.
    DeclaredDependsOn,
    /// Seen in traces or mesh telemetry for one environment.
    ObservedCalls,
    /// Network packets observed between components; direction is packet direction, not call initiation.
    NetworkTraffic,
    /// Suggested by static analysis and not yet confirmed.
    CandidateCalls,
    /// Component runs as this deployment.
    DeployedAs,
    /// A workload runs in a cluster.
    RunsInCluster,
    /// A component is associated with a workload by an explicit label.
    HasWorkload,
    /// Component includes this library.
    UsesLibrary,
    /// Component publishes to a topic or queue.
    PublishesTo,
    /// Component consumes from a topic or queue.
    ConsumesFrom,
    /// A gateway, ingress or load balancer forwards traffic to a service.
    RoutesTo,
    /// A component provides an API or event contract (OpenAPI, AsyncAPI, proto).
    ProvidesContract,
    /// A component relies on a contract another component provides.
    UsesContract,
}

/// The family an edge type belongs to. One definition, used wherever the code
/// must decide how a kind of edge behaves (impact, scoring, display).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EdgeClass {
    /// One thing needs another to work: declared, observed, suspected or routed.
    Dependency,
    /// Messages flow through a queue or topic.
    Messaging,
    /// A component runs as a deployment.
    Deployment,
    /// Where something runs. Says nothing about what depends on what.
    Placement,
    /// A package is used.
    Library,
    /// Packets were seen. Direction is not call direction.
    Traffic,
    /// An API or event contract is provided or used.
    Contract,
}

impl EdgeType {
    /// Every edge type, in wire order. Tests use it to keep docs and schema in step.
    pub const ALL: [Self; 13] = [
        Self::DeclaredDependsOn,
        Self::ObservedCalls,
        Self::NetworkTraffic,
        Self::CandidateCalls,
        Self::DeployedAs,
        Self::RunsInCluster,
        Self::HasWorkload,
        Self::UsesLibrary,
        Self::PublishesTo,
        Self::ConsumesFrom,
        Self::RoutesTo,
        Self::ProvidesContract,
        Self::UsesContract,
    ];

    /// The family of this edge type. Adding a type forces a decision here.
    #[must_use]
    pub const fn class(self) -> EdgeClass {
        match self {
            Self::DeclaredDependsOn
            | Self::ObservedCalls
            | Self::CandidateCalls
            | Self::RoutesTo => EdgeClass::Dependency,
            Self::PublishesTo | Self::ConsumesFrom => EdgeClass::Messaging,
            Self::DeployedAs => EdgeClass::Deployment,
            Self::RunsInCluster | Self::HasWorkload => EdgeClass::Placement,
            Self::UsesLibrary => EdgeClass::Library,
            Self::NetworkTraffic => EdgeClass::Traffic,
            Self::ProvidesContract | Self::UsesContract => EdgeClass::Contract,
        }
    }

    /// Whether impact analysis follows this edge. Traffic and placement do not
    /// say what depends on what.
    #[must_use]
    pub const fn affects_impact(self) -> bool {
        !matches!(self.class(), EdgeClass::Traffic | EdgeClass::Placement)
    }

    /// Whether the edge is stored pointing away from the thing that depends on
    /// the other end. A publisher is not affected by the topic it writes to, so
    /// `publishes_to` is flipped when the graph is walked. So is
    /// `provides_contract`: a change in the provider reaches the contract and
    /// then the components that use it. `routes_to` is not
    /// flipped: a backend change must reach the gateway in front of it.
    #[must_use]
    pub const fn points_to_dependent(self) -> bool {
        matches!(self, Self::PublishesTo | Self::ProvidesContract)
    }

    /// Whether the edge links two services or resources in a way a person would
    /// list as "A depends on B". Used when scoring accuracy.
    #[must_use]
    pub const fn is_service_tie(self) -> bool {
        matches!(
            self.class(),
            EdgeClass::Dependency | EdgeClass::Messaging | EdgeClass::Traffic
        )
    }

    /// Stable wire value. This string is part of the edge-id hash.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DeclaredDependsOn => "declared_depends_on",
            Self::ObservedCalls => "observed_calls",
            Self::NetworkTraffic => "network_traffic",
            Self::CandidateCalls => "candidate_calls",
            Self::DeployedAs => "deployed_as",
            Self::RunsInCluster => "runs_in_cluster",
            Self::HasWorkload => "has_workload",
            Self::UsesLibrary => "uses_library",
            Self::PublishesTo => "publishes_to",
            Self::ConsumesFrom => "consumes_from",
            Self::RoutesTo => "routes_to",
            Self::ProvidesContract => "provides_contract",
            Self::UsesContract => "uses_contract",
        }
    }
}

/// Where an edge observation came from.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Backstage catalog or `catalog-info.yaml`.
    Catalog,
    /// OpenTelemetry service graph or traces.
    Otel,
    /// Istio telemetry.
    Istio,
    /// Cilium Hubble flows.
    Hubble,
    /// Hint derived from an SBOM, not a resolved library edge by itself.
    SbomHint,
    /// Static contract or source scan.
    Static,
    /// A cloud provider's own service map: AWS X-Ray or Azure Application Insights.
    Cloud,
    /// Kubernetes API workload metadata.
    Kubernetes,
}

/// Agreement between declared and observed edges.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
pub enum EdgeStatus {
    /// Declared and observed.
    #[serde(rename = "both")]
    Both,
    /// Observed, with no declaration.
    #[serde(rename = "observed_only")]
    ObservedOnly,
    /// Declared, with no observation.
    #[serde(rename = "declared_only")]
    DeclaredOnly,
    /// Only a candidate signal.
    #[serde(rename = "candidate")]
    Candidate,
    /// Not applicable, including structural edges such as `deployed_as`.
    #[default]
    #[serde(rename = "n/a")]
    NotApplicable,
}

/// Where a hard/soft label came from. A heuristic is evidence, not a cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum CriticalitySource {
    /// `depgraph/soft-dependencies` in catalog-info.yaml.
    Declared,
    /// A fault-injection result that was ingested, not executed here.
    Experiment,
    /// Fallback, circuit breaker, or async. Evidence only.
    Heuristic,
    /// Not classified.
    #[default]
    Unknown,
}

impl CriticalitySource {
    /// Wire value.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Declared => "declared",
            Self::Experiment => "experiment",
            Self::Heuristic => "heuristic",
            Self::Unknown => "unknown",
        }
    }
}

/// How hard a dependency is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum Criticality {
    /// Failure propagates.
    Hard,
    /// The caller tolerates failure.
    Soft,
    /// Not classified.
    #[default]
    Unknown,
}

/// Outcome of one collector inside a snapshot or `meta.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// The collector finished and its records were kept.
    Ok,
    /// The collector failed.
    Failed,
    /// This run failed; the previous records stay in place.
    Stale,
    /// The collector was not selected for this run.
    Skipped,
}

/// One observation window on an edge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeSource {
    /// Which system produced the observation.
    pub kind: SourceKind,
    /// First time this source supported the edge.
    pub first_seen: Timestamp,
    /// Last time this source supported the edge.
    pub last_seen: Timestamp,
    /// Optional pointer to a fixture, job, or query. Never a credentialed URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_ref: Option<String>,
}

/// Numeric and protocol attributes. Absent fields are unknown, not zero.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct EdgeAttrs {
    /// Call count over seven days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls_7d: Option<u64>,
    /// Error rate over seven days, from 0 to 1 when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate_7d: Option<f64>,
    /// 95th percentile latency in milliseconds over seven days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub p95_ms_7d: Option<f64>,
    /// Protocol name, such as `http` or `grpc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// Call counter when the query window is one day because retention is under 7 days.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub calls_total: Option<u64>,
    /// `true` when the call is synchronous.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync: Option<bool>,
}

/// A component, deployment, resource, or library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Node {
    /// Id following the convention in `docs/naming.md`.
    pub id: String,
    /// Kind. Must agree with the id prefix.
    #[serde(rename = "type")]
    pub node_type: NodeType,
    /// Free-form attributes. Keys are sorted on write. Do not put secrets here.
    #[serde(default)]
    pub attrs: BTreeMap<String, Value>,
}

/// A directed relationship. `env` is null when the edge is not environment-specific.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    /// Stable hash of `(type, from_id, to_id, env)`.
    pub id: String,
    /// Relationship kind.
    #[serde(rename = "type")]
    pub edge_type: EdgeType,
    /// Source node id.
    pub from_id: String,
    /// Target node id.
    pub to_id: String,
    /// Canonical environment, or null.
    #[serde(default)]
    pub env: Option<Env>,
    /// At least one source is required.
    #[serde(default)]
    pub sources: Vec<EdgeSource>,
    /// Declared versus observed. Defaults to `n/a`.
    #[serde(default)]
    pub status: EdgeStatus,
    /// Hard, soft, or unknown. Defaults to `unknown`.
    #[serde(default)]
    pub criticality: Criticality,
    /// Which source produced `criticality`. Defaults to `unknown` and is omitted then.
    #[serde(default, skip_serializing_if = "criticality_source_unknown")]
    pub criticality_source: CriticalitySource,
    /// Inactive edges stay in the snapshot until expiry. Defaults to `true`.
    #[serde(default = "default_active")]
    pub active: bool,
    /// Optional measurements.
    #[serde(default)]
    pub attrs: EdgeAttrs,
}

fn default_active() -> bool {
    true
}

fn criticality_source_unknown(source: &CriticalitySource) -> bool {
    *source == CriticalitySource::Unknown
}

/// Per-collector summary stored on the snapshot. Messages stay in `meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SourceSummary {
    /// Last known outcome.
    pub status: RunStatus,
    /// When the collector finished, if it did.
    #[serde(default)]
    pub finished_at: Option<Timestamp>,
    /// Records kept for this collector.
    pub count: u64,
}

/// One published graph.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GraphSnapshot {
    /// Semver of this document shape.
    pub schema_version: String,
    /// Id of the run that produced the snapshot.
    pub run_id: String,
    /// When the snapshot was built.
    pub generated_at: Timestamp,
    /// Collector name to summary. Keys are sorted.
    pub sources: BTreeMap<String, SourceSummary>,
    /// Nodes, sorted by id in canonical output.
    pub nodes: Vec<Node>,
    /// Edges, sorted by id in canonical output.
    pub edges: Vec<Edge>,
}

impl GraphSnapshot {
    /// Sort nodes, edges, and edge sources so two builds emit the same bytes.
    pub fn canonicalize(&mut self) {
        self.nodes.sort_by(|left, right| left.id.cmp(&right.id));
        self.edges.sort_by(|left, right| left.id.cmp(&right.id));
        for edge in &mut self.edges {
            edge.sources.sort_by(|left, right| {
                left.kind
                    .cmp(&right.kind)
                    .then_with(|| left.first_seen.cmp(&right.first_seen))
                    .then_with(|| left.last_seen.cmp(&right.last_seen))
                    .then_with(|| left.evidence_ref.cmp(&right.evidence_ref))
            });
        }
    }
}

/// JSON Schema generated from [`GraphSnapshot`].
///
/// # Panics
///
/// Panics if the schema value cannot be represented as JSON, which is a bug.
#[must_use]
pub fn graph_schema_value() -> Value {
    let schema = schemars::schema_for!(GraphSnapshot);
    serde_json::to_value(&schema).expect("graph schema serializes to JSON")
}

/// Pretty canonical JSON, including a trailing newline.
///
/// # Errors
///
/// Returns an error if the snapshot cannot be serialized.
pub fn to_canonical_json(graph: &GraphSnapshot) -> Result<String, serde_json::Error> {
    let mut copy = graph.clone();
    copy.canonicalize();
    let mut text = serde_json::to_string_pretty(&copy)?;
    text.push('\n');
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_status_criticality_and_active() {
        let json = r#"{
            "id": "edge:abc",
            "type": "declared_depends_on",
            "from_id": "component:a",
            "to_id": "component:b"
        }"#;
        let edge: Edge = serde_json::from_str(json).unwrap();
        assert_eq!(edge.status, EdgeStatus::NotApplicable);
        assert_eq!(edge.criticality, Criticality::Unknown);
        assert!(edge.active);
        assert!(edge.env.is_none());
        assert!(edge.sources.is_empty());
    }

    #[test]
    fn edge_status_wire_values() {
        assert_eq!(
            serde_json::to_string(&EdgeStatus::NotApplicable).unwrap(),
            "\"n/a\""
        );
        assert_eq!(
            serde_json::to_string(&EdgeStatus::ObservedOnly).unwrap(),
            "\"observed_only\""
        );
    }

    #[test]
    fn edge_type_as_str_matches_serde() {
        let types = [
            EdgeType::DeclaredDependsOn,
            EdgeType::ObservedCalls,
            EdgeType::CandidateCalls,
            EdgeType::DeployedAs,
            EdgeType::RunsInCluster,
            EdgeType::HasWorkload,
            EdgeType::UsesLibrary,
            EdgeType::PublishesTo,
            EdgeType::ConsumesFrom,
            EdgeType::RoutesTo,
        ];
        for edge_type in types {
            let json = serde_json::to_string(&edge_type).unwrap();
            assert_eq!(json, format!("\"{}\"", edge_type.as_str()));
        }
    }

    #[test]
    fn unknown_env_is_rejected() {
        let error = serde_json::from_str::<Env>("\"staging\"").unwrap_err();
        assert!(error.to_string().contains("staging") || error.to_string().contains("unknown"));
    }

    #[test]
    fn timestamp_accepts_offset_and_zulu() {
        let zulu = Timestamp::parse("2026-01-01T00:00:00Z").unwrap();
        let offset = Timestamp::parse("2026-01-01T01:00:00+01:00").unwrap();
        assert_eq!(zulu.to_datetime().unwrap(), offset.to_datetime().unwrap());
        assert!(Timestamp::parse("yesterday").is_err());
    }
}

#[cfg(test)]
mod taxonomy_tests {
    use super::*;
    use crate::ids::{ResourceFamily, ResourceKind};

    #[test]
    fn all_lists_are_complete_and_unique() {
        let wire: std::collections::BTreeSet<_> =
            EdgeType::ALL.iter().map(|t| t.as_str()).collect();
        assert_eq!(wire.len(), EdgeType::ALL.len());
        let kinds: std::collections::BTreeSet<_> =
            ResourceKind::ALL.iter().map(|k| k.as_str()).collect();
        assert_eq!(kinds.len(), ResourceKind::ALL.len());
    }

    #[test]
    fn edge_classes_keep_the_behaviour_the_code_relied_on() {
        use EdgeType::*;
        for kind in [NetworkTraffic, RunsInCluster, HasWorkload] {
            assert!(!kind.affects_impact(), "{kind:?}");
        }
        for kind in [
            DeclaredDependsOn,
            ObservedCalls,
            CandidateCalls,
            RoutesTo,
            PublishesTo,
            ConsumesFrom,
            DeployedAs,
            UsesLibrary,
        ] {
            assert!(kind.affects_impact(), "{kind:?}");
        }
        assert!(PublishesTo.points_to_dependent());
        assert!(!RoutesTo.points_to_dependent());
        assert!(NetworkTraffic.is_service_tie());
        assert!(!DeployedAs.is_service_tie());
        assert_eq!(RoutesTo.class(), EdgeClass::Dependency);
    }

    #[test]
    fn resource_families_group_kinds() {
        assert_eq!(ResourceKind::Queue.family(), ResourceFamily::Messaging);
        assert_eq!(ResourceKind::Gateway.family(), ResourceFamily::Network);
        assert_eq!(ResourceKind::Database.family(), ResourceFamily::DataStore);
    }

    /// The naming doc must name every edge type and resource kind, so a new
    /// one cannot ship without its documentation.
    #[test]
    fn naming_doc_lists_every_edge_type_and_resource_kind() {
        let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/docs/naming.md"))
            .expect("naming.md");
        for kind in EdgeType::ALL {
            assert!(
                doc.contains(kind.as_str()),
                "naming.md misses edge type {}",
                kind.as_str()
            );
        }
        for kind in ResourceKind::ALL {
            assert!(
                doc.contains(kind.as_str()),
                "naming.md misses resource kind {}",
                kind.as_str()
            );
        }
    }
}
