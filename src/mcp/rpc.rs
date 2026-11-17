//! MCP messages, independent of transport.
//!
//! The HTTP server (`server.rs`) and the stdio server (`stdio.rs`) both call
//! [`handle`]. Authentication happens before, in the transport.
//!
//! The server keeps no session state, so it speaks both the handshake
//! revisions (`initialize`, 2025-03-26 up to 2025-11-25) and the stateless
//! revision 2026-07-28 (`server/discover`, per-request version in `_meta`).
//! Every tool is read-only.

use serde_json::{Value, json};

use crate::analysis::algo;
use crate::mcp::server::McpState;
use crate::models::Env;

/// Protocol revisions this server answers, newest first.
pub const SUPPORTED_VERSIONS: [&str; 4] = ["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"];

/// Newest revision that uses the `initialize` handshake.
const HANDSHAKE_VERSION: &str = "2025-11-25";

const VERSION_KEY: &str = "io.modelcontextprotocol/protocolVersion";

/// Tools never change between calls: a client may keep the list for five minutes.
const LIST_TTL_MS: u64 = 300_000;

const INSTRUCTIONS: &str = "Read-only answers about what depends on what. \
Start with `impact` (what breaks if a component changes, with owners to tell) or \
`component_context` (one component's owner, repository, dependencies and consumers). \
Names are kebab-case component names such as payments-api. \
Every tie carries a status: `candidate` means inferred from source code, not confirmed. \
An empty result can mean the graph does not see it, not that nothing depends on it.";

/// What went wrong in a tool call.
enum ToolError {
    /// No such tool. A protocol error.
    Unknown(String),
    /// The call was understood but failed. Reported to the model as a tool result.
    Failed(String),
}

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        Self::Failed(message)
    }
}

/// Answer one JSON-RPC message. `None` means no reply: a notification.
pub fn handle(state: &McpState, message: &Value) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error(Value::Null, -32600, "invalid request", None));
    };
    let id = object.get("id").cloned();
    let method = object
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let params = object.get("params").unwrap_or(&Value::Null);
    let Some(id) = id else {
        // A notification, such as notifications/initialized: no reply.
        return None;
    };
    if let Some(version) = params
        .pointer("/_meta")
        .and_then(|meta| meta.get(VERSION_KEY))
        .and_then(Value::as_str)
        && !SUPPORTED_VERSIONS.contains(&version)
    {
        return Some(error(
            id,
            -32022,
            "unsupported protocol version",
            Some(json!({ "supported": SUPPORTED_VERSIONS })),
        ));
    }
    let result = match method {
        "initialize" => {
            let asked = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let version = if SUPPORTED_VERSIONS.contains(&asked) {
                asked
            } else {
                HANDSHAKE_VERSION
            };
            json!({
                "protocolVersion": version,
                "capabilities": capabilities(),
                "serverInfo": server_info(),
                "instructions": INSTRUCTIONS,
            })
        }
        "server/discover" => json!({
            "supportedVersions": SUPPORTED_VERSIONS,
            "capabilities": capabilities(),
            "serverInfo": server_info(),
            "instructions": INSTRUCTIONS,
        }),
        "ping" => json!({}),
        "tools/list" => json!({
            "tools": tool_definitions(),
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "private",
        }),
        "tools/call" => match call_tool(state, params) {
            Ok(payload) => tool_result(&payload, false),
            Err(ToolError::Failed(text)) => tool_result(&json!({ "error": text }), true),
            Err(ToolError::Unknown(name)) => {
                return Some(error(id, -32602, &format!("unknown tool `{name}`"), None));
            }
        },
        _ => return Some(error(id, -32601, "method not found", None)),
    };
    Some(with_result_type(id, result))
}

fn with_result_type(id: Value, mut result: Value) -> Value {
    if let Some(object) = result.as_object_mut() {
        object.insert("resultType".to_owned(), json!("complete"));
    }
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({ "code": code, "message": message });
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({ "jsonrpc": "2.0", "id": id, "error": error })
}

fn capabilities() -> Value {
    json!({ "tools": { "listChanged": false } })
}

fn server_info() -> Value {
    json!({
        "name": "depgraph",
        "title": "depgraph: service dependency graph",
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// A tool result. Objects also go in `structuredContent`, for clients that read it.
fn tool_result(payload: &Value, is_error: bool) -> Value {
    let mut result = json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "isError": is_error,
    });
    if payload.is_object() {
        result["structuredContent"] = payload.clone();
    }
    result
}

/// Name, title, description, required arguments.
fn tool_meta(name: &str) -> (&'static str, &'static str, &'static [&'static str]) {
    match name {
        "show" => (
            "Show a component",
            "One node with its attributes (owner, department, lifecycle, system) and every tie touching it, each with status and confidence.",
            &["component"],
        ),
        "deps" => (
            "What a component depends on",
            "Everything the component needs, transitively (databases, queues, other services). Use for 'what does X call?'.",
            &["component"],
        ),
        "consumers" => (
            "What depends on a component",
            "Everything that calls or needs the component, transitively. Use for 'who uses X?'.",
            &["component"],
        ),
        "impact" => (
            "Impact of changing a component",
            "What breaks if this component changes, ranked closest and most critical first, with the owners to tell. Ties marked candidate are inferred from source code, not confirmed. Pass env prod for production.",
            &["component"],
        ),
        "deploy_order" => (
            "Deploy order",
            "Order in which to deploy the given components, dependencies first. A cycle is reported instead of an order. Pass `components` (several) or `component` (one).",
            &[],
        ),
        "cycles" => (
            "Circular dependencies",
            "Groups of components that depend on each other in a loop.",
            &[],
        ),
        "library_impact" => (
            "Library impact",
            "Not available: library tracking is off by default. Returns a message saying so.",
            &[],
        ),
        "drift" => (
            "Declared versus observed",
            "Ties that are declared but never seen in traffic, seen but not declared, or only inferred, for one environment. Markdown.",
            &[],
        ),
        "critical_nodes" => (
            "Most critical nodes",
            "Nodes whose failure reaches the most others: highest fan-in, bridges and articulation points.",
            &[],
        ),
        "orphans" => (
            "Nodes with no ties",
            "Nodes nothing links to. Often a source the tool could not read, not a safe delete.",
            &[],
        ),
        "diff" => (
            "Diff two runs",
            "Not available over MCP: it needs two snapshots. Use `depgraph diff` on the command line.",
            &[],
        ),
        "component_context" => (
            "Component context",
            "Everything an assistant needs to start work on a component: repository path and URL, branch, catalog and manifest paths, owner, languages, direct dependencies and direct consumers.",
            &["component"],
        ),
        "scorecard" => (
            "Data quality per owner",
            "How complete the graph is for each owner: components, with system, with telemetry. An aid, not a ranking for blame.",
            &[],
        ),
        _ => ("", "", &[]),
    }
}

/// Tool list, in a stable order.
pub fn tool_definitions() -> Vec<Value> {
    crate::mcp::TOOLS
        .iter()
        .map(|tool| {
            let (title, description, required) = tool_meta(tool.name);
            json!({
                "name": tool.name,
                "title": title,
                "description": description,
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "component": {
                            "type": "string",
                            "description": "Component name such as payments-api, or a node id such as component:payments-api. Aliases are resolved."
                        },
                        "components": {
                            "type": "array",
                            "items": { "type": "string" },
                            "description": "Several component names. Used by deploy_order."
                        },
                        "env": {
                            "type": "string",
                            "enum": ["dev", "acc", "prod"],
                            "description": "Only ties in this environment. Omit for all."
                        },
                        "depth": {
                            "type": "integer",
                            "minimum": 1,
                            "description": "Stop after this many hops. Omit for all."
                        }
                    },
                    "required": required,
                },
                "annotations": {
                    "title": title,
                    "readOnlyHint": tool.read_only,
                    "destructiveHint": false,
                    "idempotentHint": true,
                    "openWorldHint": false,
                },
            })
        })
        .collect()
}

fn parse_env(value: &str) -> Option<Env> {
    match value {
        "dev" => Some(Env::Dev),
        "acc" => Some(Env::Acc),
        "prod" => Some(Env::Prod),
        _ => None,
    }
}

/// Node id for a name: aliases first, then the id as given, then `component:<name>`.
fn resolve(state: &McpState, component: &str) -> Result<String, String> {
    let name = component.trim();
    if name.is_empty() {
        return Err("`component` is required".to_owned());
    }
    let aliased = state
        .aliases
        .aliases
        .get(name)
        .map(String::as_str)
        .unwrap_or(name);
    for candidate in [aliased.to_owned(), format!("component:{aliased}")] {
        if state.snapshot.nodes.iter().any(|node| node.id == candidate) {
            return Ok(candidate);
        }
    }
    Err(format!(
        "unknown node `{name}`. Names are kebab-case component names; try `show` with another spelling"
    ))
}

fn call_tool(state: &McpState, params: &Value) -> Result<Value, ToolError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !crate::mcp::TOOLS.iter().any(|tool| tool.name == name) {
        return Err(ToolError::Unknown(name.to_owned()));
    }
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let env = match args.get("env").and_then(Value::as_str) {
        None => None,
        Some(text) => Some(
            parse_env(text).ok_or_else(|| format!("env must be dev, acc or prod, not `{text}`"))?,
        ),
    };
    let component = args
        .get("component")
        .or_else(|| args.get("target"))
        .and_then(Value::as_str)
        .unwrap_or_default();
    let depth = args
        .get("depth")
        .and_then(Value::as_u64)
        .map(|value| value as usize);
    let graph = &state.snapshot;
    let failed = |err: crate::error::Error| err.to_string();
    let payload = match name {
        "show" => {
            let id = resolve(state, component)?;
            let node = graph
                .nodes
                .iter()
                .find(|node| node.id == id)
                .ok_or_else(|| format!("unknown node `{component}`"))?;
            let ties: Vec<&crate::models::Edge> = graph
                .edges
                .iter()
                .filter(|edge| edge.from_id == id || edge.to_id == id)
                .filter(|edge| env.is_none() || edge.env.is_none() || edge.env == env)
                .collect();
            json!({ "node": node, "ties": ties })
        }
        "deps" => {
            let id = resolve(state, component)?;
            let found = algo::dependencies(graph, &id, env, depth).map_err(failed)?;
            json!({ "component": id, "dependencies": found })
        }
        "consumers" => {
            let id = resolve(state, component)?;
            let found = algo::consumers(graph, &id, env, depth).map_err(failed)?;
            json!({ "component": id, "consumers": found })
        }
        "impact" => {
            let id = resolve(state, component)?;
            let ranked =
                crate::score::ranked_impact(graph, &id, env, crate::score::Hardness::default())
                    .map_err(failed)?;
            let owners = crate::analysis::affected_owners(graph, &ranked);
            json!({ "component": id, "impact": ranked, "owners_to_tell": owners })
        }
        "deploy_order" => {
            let named: Vec<String> = match args.get("components").and_then(Value::as_array) {
                Some(items) => items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect(),
                None => vec![component.to_owned()],
            };
            let ids = named
                .iter()
                .map(|item| resolve(state, item))
                .collect::<Result<Vec<_>, _>>()?;
            json!(algo::deploy_order(graph, &ids, env))
        }
        "cycles" => json!({ "cycles": algo::cycles(graph, env) }),
        "library_impact" => json!({ "message": algo::library_impact_disabled() }),
        "drift" => {
            let env = env.unwrap_or(Env::Prod);
            json!({
                "env": env.as_str(),
                "markdown": crate::analysis::drift_markdown(graph, env),
            })
        }
        "critical_nodes" => {
            let hardness = crate::score::Hardness::default();
            let nodes: Vec<Value> = algo::critical_nodes(graph, env)
                .iter()
                .map(|node| {
                    json!({
                        "id": node.id,
                        "in_degree": node.in_degree,
                        "betweenness": node.betweenness,
                        "articulation": node.articulation,
                        "weight_score": crate::score::weight_score(graph, &node.id, env, hardness),
                    })
                })
                .collect();
            json!({ "nodes": nodes })
        }
        "orphans" => json!({ "orphans": algo::orphans(graph) }),
        "diff" => {
            return Err(ToolError::Failed(
                "diff needs two snapshots and is not available over MCP; run `depgraph diff --from <run> --to <run>`"
                    .to_owned(),
            ));
        }
        "component_context" => component_context(state, component, env)?,
        "scorecard" => {
            let rows: Vec<Value> = crate::score::scorecard(graph)
                .into_iter()
                .map(|row| {
                    json!({
                        "owner": row.owner,
                        "components": row.components,
                        "with_system": row.with_system,
                        "with_telemetry": row.with_telemetry,
                    })
                })
                .collect();
            json!({ "owners": rows })
        }
        _ => return Err(ToolError::Unknown(name.to_owned())),
    };
    Ok(payload)
}

fn component_context(state: &McpState, component: &str, env: Option<Env>) -> Result<Value, String> {
    let id = resolve(state, component)?;
    let node = state
        .snapshot
        .nodes
        .iter()
        .find(|node| node.id == id)
        .ok_or_else(|| format!("unknown node `{component}`"))?;
    let deps = algo::dependencies(&state.snapshot, &node.id, env, Some(1))
        .map_err(|err| err.to_string())?;
    let consumers =
        algo::consumers(&state.snapshot, &node.id, env, Some(1)).map_err(|err| err.to_string())?;
    let attr = |key: &str| node.attrs.get(key).cloned().unwrap_or(Value::Null);
    Ok(json!({
        "component": node.id,
        "project_path": attr("project_path"),
        "web_url": attr("web_url"),
        "default_branch": attr("default_branch"),
        "catalog_path": node.attrs.get("catalog_path").cloned().unwrap_or(json!("catalog-info.yaml")),
        "manifest_paths": node.attrs.get("manifest_paths").cloned().unwrap_or(json!([])),
        "owner": attr("owner"),
        "department": attr("department"),
        "lifecycle": attr("lifecycle"),
        "languages": node.attrs.get("languages").cloned().unwrap_or(json!([])),
        "container": node.attrs.get("container").cloned().unwrap_or(json!(false)),
        "dependencies": deps,
        "consumers": consumers
    }))
}
