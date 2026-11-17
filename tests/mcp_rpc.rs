//! MCP protocol behaviour that agents rely on: version negotiation, discovery,
//! tool metadata, and tool failures reported as tool results.

use depgraph::config::Aliases;
use depgraph::mcp::{AccessPolicy, JwksCache, McpState, handle_message};
use depgraph::models::{GraphSnapshot, Node, NodeType, Timestamp};
use serde_json::{Value, json};

fn state() -> McpState {
    let mut aliases = Aliases::default();
    aliases
        .aliases
        .insert("payments".into(), "payments-api".into());
    let mut attrs = std::collections::BTreeMap::new();
    attrs.insert("owner".into(), json!("platform-team"));
    McpState {
        issuer: "https://idp.example.com".into(),
        audience: "http://127.0.0.1:8080".into(),
        policy: AccessPolicy::generic("https://idp.example.com", "http://127.0.0.1:8080"),
        jwks_url: String::new(),
        insecure: true,
        snapshot: GraphSnapshot {
            schema_version: "0.1.0".into(),
            run_id: "test".into(),
            generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
            sources: Default::default(),
            nodes: vec![Node {
                id: "component:payments-api".into(),
                node_type: NodeType::Component,
                attrs,
            }],
            edges: Vec::new(),
        },
        aliases,
        jwks: tokio::sync::Mutex::new(JwksCache::default()),
        http: reqwest::Client::new(),
    }
}

fn call(message: Value) -> Value {
    handle_message(&state(), &message).expect("a reply")
}

#[test]
fn initialize_answers_with_a_version_the_client_asked_for() {
    for version in ["2025-03-26", "2025-06-18", "2025-11-25"] {
        let reply = call(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":version,"capabilities":{},"clientInfo":{"name":"t","version":"1"}}}));
        assert_eq!(reply["result"]["protocolVersion"], version);
        assert!(
            reply["result"]["instructions"]
                .as_str()
                .unwrap()
                .contains("impact")
        );
    }
    let unknown = call(json!({"jsonrpc":"2.0","id":1,"method":"initialize",
        "params":{"protocolVersion":"1999-01-01"}}));
    assert_eq!(unknown["result"]["protocolVersion"], "2025-11-25");
}

#[test]
fn stateless_clients_can_discover_and_are_refused_an_unknown_version() {
    let discover = call(json!({"jsonrpc":"2.0","id":1,"method":"server/discover"}));
    assert_eq!(discover["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(discover["result"]["resultType"], "complete");

    let refused = call(json!({"jsonrpc":"2.0","id":2,"method":"tools/list",
        "params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2030-01-01"}}}));
    assert_eq!(refused["error"]["code"], -32022);
}

#[test]
fn notifications_get_no_reply() {
    assert!(
        handle_message(
            &state(),
            &json!({"jsonrpc":"2.0","method":"notifications/initialized"})
        )
        .is_none()
    );
}

#[test]
fn every_tool_is_described_read_only_and_requires_what_it_needs() {
    let reply = call(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}));
    let tools = reply["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 13);
    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["description"].as_str().unwrap().len() > 30,
            "{name} needs a real description"
        );
        assert_eq!(tool["annotations"]["readOnlyHint"], true, "{name}");
        assert_eq!(tool["annotations"]["destructiveHint"], false, "{name}");
        assert_eq!(tool["inputSchema"]["type"], "object", "{name}");
    }
    let impact = tools.iter().find(|t| t["name"] == "impact").unwrap();
    assert_eq!(impact["inputSchema"]["required"][0], "component");
    assert_eq!(reply["result"]["ttlMs"], 300_000);
}

#[test]
fn a_bad_name_is_a_tool_error_the_model_can_read_not_a_protocol_error() {
    let reply = call(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"impact","arguments":{"component":"nope"}}}));
    assert!(reply.get("error").is_none());
    assert_eq!(reply["result"]["isError"], true);
    assert!(
        reply["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("unknown node")
    );

    let missing = call(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"no_such_tool","arguments":{}}}));
    assert_eq!(missing["error"]["code"], -32602);
}

#[test]
fn show_reads_the_node_and_resolves_aliases() {
    let reply = call(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"show","arguments":{"component":"payments"}}}));
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(
        reply["result"]["structuredContent"]["node"]["id"],
        "component:payments-api"
    );
}

#[test]
fn a_bad_env_is_reported_instead_of_ignored() {
    let reply = call(json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"impact","arguments":{"component":"payments","env":"production"}}}));
    assert_eq!(reply["result"]["isError"], true);
}
