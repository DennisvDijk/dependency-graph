#[test]
fn gitlab_pipeline_keeps_the_planned_shape() {
    let text = include_str!("../.gitlab-ci.yml");
    for needle in [
        "stages:",
        "- test",
        "- collect",
        "- build",
        "- publish",
        "\ncollect:",
        "publish-snapshot:",
        "\npages:",
        "drift-issue:",
        "0 */6 * * *",
        "30 3 * * *",
        "0 7 * * 1",
        "COLLECT_SET=fast",
        "COLLECT_SET=full",
        "SCHEDULE_KIND=weekly",
        "depgraph",
    ] {
        assert!(text.contains(needle), "missing {needle}");
    }
}

#[test]
fn adr_001_records_answers_and_remaining_defaults() {
    let text = include_str!("../docs/decisions/ADR-001-aannames.md");
    assert_eq!(text.matches("- Status: beantwoord").count(), 8);
    assert_eq!(text.matches("- Status: standaardaanname").count(), 9);
    assert!(text.contains("## Vragen die een mens nog moet beantwoorden"));
    assert!(text.contains("## Verifieer vóór gebruik"));
    assert!(text.contains("traces_service_graph_"));
    assert!(text.contains("Cartography"));
    assert!(text.contains("Dependency-Track"));
    assert!(text.contains("GET /api/v4/version"));
}

#[test]
fn later_adrs_record_the_runtime_pilot_and_mcp_decisions() {
    let runtime = include_str!("../docs/decisions/ADR-003-runtime-bron.md");
    assert!(runtime.contains("istio_requests_total"));
    assert!(runtime.contains("reporter=\"destination\""));
    assert!(runtime.contains("Langfuse"));
    assert!(runtime.contains("network-policy"));
    assert!(runtime.contains("firewall-rule"));
    assert!(runtime.contains("`infra`"));
    let pilot = include_str!("../docs/decisions/ADR-004-pilot-en-datadog.md");
    assert!(pilot.contains("DEPGRAPH_DATADOG_API_KEY"));
    assert!(pilot.contains("DEPGRAPH_DATADOG_APP_KEY"));
    assert!(pilot.contains("network-policy"));
    let mcp = include_str!("../docs/decisions/ADR-005-mcp-server.md");
    for tool in [
        "show",
        "deps",
        "consumers",
        "impact",
        "deploy_order",
        "cycles",
        "library_impact",
        "drift",
        "critical_nodes",
        "orphans",
        "diff",
    ] {
        assert!(mcp.contains(format!("`{tool}`").as_str()), "missing {tool}");
    }
    assert!(mcp.contains("RFC 9728"));
    assert!(mcp.contains("RFC 8707"));
    assert!(mcp.contains("Streamable HTTP"));
}
