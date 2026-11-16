use std::collections::BTreeMap;

use depgraph::ids::{self, ResourceKind};
use depgraph::models::{
    Criticality, Edge, EdgeAttrs, EdgeSource, EdgeStatus, EdgeType, Env, GraphSnapshot, Node,
    NodeType, SourceKind, SourceSummary, Timestamp,
};

pub fn stamp(value: &str) -> Timestamp {
    Timestamp::parse(value).unwrap()
}

pub fn empty_graph() -> GraphSnapshot {
    GraphSnapshot {
        schema_version: "0.1.0".to_owned(),
        run_id: "empty".to_owned(),
        generated_at: stamp("2026-01-01T00:00:00Z"),
        sources: BTreeMap::new(),
        nodes: Vec::new(),
        edges: Vec::new(),
    }
}

fn node(id: String, node_type: NodeType) -> Node {
    Node {
        id,
        node_type,
        attrs: BTreeMap::new(),
    }
}

fn edge(
    edge_type: EdgeType,
    from_id: &str,
    to_id: &str,
    env: Option<Env>,
    kind: SourceKind,
    status: EdgeStatus,
) -> Edge {
    Edge {
        id: ids::edge_id(edge_type, from_id, to_id, env),
        edge_type,
        from_id: from_id.to_owned(),
        to_id: to_id.to_owned(),
        env,
        sources: vec![EdgeSource {
            kind,
            first_seen: stamp("2026-01-01T00:00:00Z"),
            last_seen: stamp("2026-01-15T00:00:00Z"),
            evidence_ref: None,
        }],
        status,
        criticality: Criticality::Hard,
        active: true,
        attrs: EdgeAttrs::default(),
    }
}

pub fn sample_graph() -> GraphSnapshot {
    let mut sources = BTreeMap::new();
    sources.insert(
        "gitlab_inventory".to_owned(),
        SourceSummary {
            status: depgraph::models::RunStatus::Ok,
            finished_at: Some(stamp("2026-01-15T12:00:00Z")),
            count: 2,
        },
    );
    let mut observed = edge(
        EdgeType::ObservedCalls,
        "component:payments-api",
        "component:billing-api",
        Some(Env::Prod),
        SourceKind::Otel,
        EdgeStatus::ObservedOnly,
    );
    observed.attrs.calls_7d = Some(12);
    observed.attrs.protocol = Some("http".to_owned());
    observed.attrs.sync = Some(true);
    GraphSnapshot {
        schema_version: "0.1.0".to_owned(),
        run_id: "sample".to_owned(),
        generated_at: stamp("2026-01-15T12:00:00Z"),
        sources,
        nodes: vec![
            node(
                ids::component_id("billing-api").unwrap(),
                NodeType::Component,
            ),
            node(
                ids::component_id("payments-api").unwrap(),
                NodeType::Component,
            ),
            node(
                ids::deployment_id("payments-api", Env::Prod).unwrap(),
                NodeType::Deployment,
            ),
            node(
                ids::library_id("pkg:cargo/serde").unwrap(),
                NodeType::Library,
            ),
            node(
                ids::resource_id(ResourceKind::Database, "payments-db").unwrap(),
                NodeType::Resource,
            ),
        ],
        edges: vec![
            edge(
                EdgeType::DeclaredDependsOn,
                "component:payments-api",
                "component:billing-api",
                None,
                SourceKind::Catalog,
                EdgeStatus::DeclaredOnly,
            ),
            observed,
            edge(
                EdgeType::DeployedAs,
                "component:payments-api",
                "deployment:payments-api@prod",
                Some(Env::Prod),
                SourceKind::Catalog,
                EdgeStatus::NotApplicable,
            ),
            edge(
                EdgeType::UsesLibrary,
                "component:payments-api",
                "library:pkg:cargo/serde",
                None,
                SourceKind::SbomHint,
                EdgeStatus::NotApplicable,
            ),
        ],
    }
}
