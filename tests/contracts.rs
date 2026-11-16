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
fn adr_001_records_every_default() {
    let text = include_str!("../docs/decisions/ADR-001-aannames.md");
    assert_eq!(text.matches("- Status: standaardaanname").count(), 17);
    assert!(text.contains("## Vragen die een mens nog moet beantwoorden"));
    assert!(text.contains("## Verifieer vóór gebruik"));
    assert!(text.contains("traces_service_graph_"));
    assert!(text.contains("Cartography"));
    assert!(text.contains("Dependency-Track"));
}
