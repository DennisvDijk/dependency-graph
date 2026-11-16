mod common;

#[test]
fn canonical_sample_is_stable() {
    let value: serde_json::Value = serde_json::from_str(
        &depgraph::models::to_canonical_json(&common::sample_graph()).unwrap(),
    )
    .unwrap();
    insta::assert_json_snapshot!(value);
}

#[test]
fn empty_fixture_matches_the_model() {
    let generated = depgraph::models::to_canonical_json(&common::empty_graph()).unwrap();
    let committed = include_str!("fixtures/empty-graph.json");
    assert_eq!(generated, committed);
}
