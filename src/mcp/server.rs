//! Streamable HTTP MCP server.
//!
//! Tools call [`crate::analysis::algo`]. The access token is checked here and
//! is not sent to GitLab, Kubernetes, Prometheus, or Datadog.

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};

use crate::analysis::algo;
use crate::config::Aliases;
use crate::error::Error;
use crate::mcp::auth::{self, JwksCache};
use crate::models::{Env, GraphSnapshot};

/// Runtime data for one server.
pub struct McpState {
    /// Token issuer.
    pub issuer: String,
    /// Expected audience.
    pub audience: String,
    /// Signature, issuer, audience, and scope rules.
    pub policy: crate::mcp::auth::AccessPolicy,
    /// JWKS document URL.
    pub jwks_url: String,
    /// Skip bearer checks. Only valid on a loopback bind.
    pub insecure: bool,
    /// Graph the tools read.
    pub snapshot: GraphSnapshot,
    /// Alias table for `component_context` and name lookup.
    pub aliases: Aliases,
    /// Cached JWKS. Unused when `insecure` is set.
    pub jwks: tokio::sync::Mutex<JwksCache>,
    /// Client used only for the JWKS URL. It does not receive the access token.
    pub http: reqwest::Client,
}

/// Axum router for metadata and `/mcp`.
pub fn router(state: Arc<McpState>) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            get(protected_resource),
        )
        .route("/mcp", post(mcp))
        .with_state(state)
}

/// Bind and serve until the process is stopped.
///
/// # Errors
///
/// Returns an error when `--insecure-local` is combined with a public bind,
/// or when the socket cannot be opened.
pub async fn serve(
    bind: &str,
    insecure: bool,
    policy: crate::mcp::auth::AccessPolicy,
    jwks_url: &str,
    snapshot_path: &Path,
    aliases: Aliases,
) -> Result<(), Error> {
    if insecure && !auth::is_loopback(bind) {
        return Err(Error::Config(
            "--insecure-local refuses a non-loopback bind".to_owned(),
        ));
    }
    if insecure {
        eprintln!(
            "WARNING: depgraph serve-mcp is running with authentication DISABLED on localhost only. Do not expose this port."
        );
    }
    let snapshot = if snapshot_path.is_file() {
        let text = std::fs::read_to_string(snapshot_path)?;
        serde_json::from_str(&text).map_err(|err| Error::Validation(err.to_string()))?
    } else {
        empty_graph()
    };
    let state = Arc::new(McpState {
        issuer: policy.issuer.clone(),
        audience: policy
            .audiences
            .first()
            .cloned()
            .unwrap_or_else(|| "http://127.0.0.1:8080".to_owned()),
        policy,
        jwks_url: jwks_url.to_owned(),
        insecure,
        snapshot,
        aliases,
        jwks: tokio::sync::Mutex::new(JwksCache::default()),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|err| Error::Config(err.to_string()))?,
    });
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(Error::Io)?;
    axum::serve(listener, router(state))
        .await
        .map_err(Error::Io)
}

async fn protected_resource(State(state): State<Arc<McpState>>) -> Json<Value> {
    Json(auth::protected_resource_metadata(
        &state.audience,
        &state.issuer,
    ))
}

async fn mcp(
    State(state): State<Arc<McpState>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !state.insecure {
        let token = bearer(&headers);
        let Some(token) = token else {
            return unauthorized(&state.audience);
        };
        if auth::authorize(
            &token,
            &state.policy,
            &state.jwks_url,
            &state.jwks,
            &state.http,
        )
        .await
        .is_err()
        {
            return unauthorized(&state.audience);
        }
    }
    let id = body.get("id").cloned().unwrap_or(Value::Null);
    let method = body
        .get("method")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let result = match method {
        "initialize" => json!({
            "protocolVersion": "2025-03-26",
            "capabilities": { "tools": { "listChanged": false } },
            "serverInfo": { "name": "depgraph", "version": "0.1.0" }
        }),
        "notifications/initialized" | "ping" => json!({}),
        "tools/list" => json!({ "tools": tool_definitions() }),
        "tools/call" => match call_tool(&state, body.get("params").unwrap_or(&Value::Null)) {
            Ok(value) => value,
            Err(message) => {
                return Json(json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": { "code": -32000, "message": message }
                }))
                .into_response();
            }
        },
        _ => {
            return Json(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": -32601, "message": "method not found" }
            }))
            .into_response();
        }
    };
    Json(json!({"jsonrpc": "2.0", "id": id, "result": result})).into_response()
}

fn bearer(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    Some(rest.trim().to_owned())
}

fn unauthorized(audience: &str) -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(header::WWW_AUTHENTICATE, auth::www_authenticate(audience))],
        Json(json!({"error": "unauthorized"})),
    )
        .into_response()
}

fn call_tool(state: &McpState, params: &Value) -> Result<Value, String> {
    let name = params
        .get("name")
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let args = params
        .get("arguments")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let env = args
        .get("env")
        .and_then(|value| value.as_str())
        .and_then(parse_env);
    let component = args
        .get("component")
        .or_else(|| args.get("target"))
        .and_then(|value| value.as_str())
        .unwrap_or("");
    let depth = args
        .get("depth")
        .and_then(|value| value.as_u64())
        .map(|value| value as usize);
    let payload = match name {
        "show" => json!({"component": component}),
        "deps" => json!(
            algo::dependencies(&state.snapshot, component, env, depth)
                .map_err(|err| err.to_string())?
        ),
        "consumers" => json!(
            algo::consumers(&state.snapshot, component, env, depth)
                .map_err(|err| err.to_string())?
        ),
        "impact" => {
            json!(algo::impact(&state.snapshot, component, env).map_err(|err| err.to_string())?)
        }
        "deploy_order" => {
            let set = args
                .get("components")
                .and_then(|value| value.as_array())
                .map(|items| {
                    items
                        .iter()
                        .filter_map(|item| item.as_str().map(str::to_owned))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_else(|| vec![component.to_owned()]);
            json!(algo::deploy_order(&state.snapshot, &set, env))
        }
        "cycles" => json!(algo::cycles(&state.snapshot, env)),
        "library_impact" => json!({"message": algo::library_impact_disabled()}),
        "drift" => json!({"env": env.map(|value| value.as_str()).unwrap_or("prod")}),
        "critical_nodes" => json!(algo::critical_nodes(&state.snapshot, env)),
        "orphans" => json!(algo::orphans(&state.snapshot)),
        "diff" => {
            json!({"note": "pass two snapshots to the CLI; the server diffs against an empty addition set"})
        }
        "component_context" => component_context(state, component, env)?,
        _ => return Err(format!("unknown tool {name}")),
    };
    Ok(json!({
        "content": [{ "type": "text", "text": payload.to_string() }],
        "isError": false
    }))
}

fn component_context(state: &McpState, component: &str, env: Option<Env>) -> Result<Value, String> {
    let resolved = state
        .aliases
        .aliases
        .get(component)
        .map(String::as_str)
        .unwrap_or(component);
    let id = if resolved.starts_with("component:") {
        resolved.to_owned()
    } else {
        format!("component:{resolved}")
    };
    let node = state
        .snapshot
        .nodes
        .iter()
        .find(|node| node.id == id || node.id == resolved)
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
        "dependencies": deps,
        "consumers": consumers
    }))
}

fn tool_definitions() -> Vec<Value> {
    crate::mcp::TOOLS
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": format!("Read-only depgraph tool {}. Requires scope depgraph:read.", tool.name),
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "component": { "type": "string" },
                        "env": { "type": "string", "enum": ["dev", "acc", "prod"] },
                        "depth": { "type": "integer" },
                        "components": { "type": "array", "items": { "type": "string" } }
                    }
                }
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

fn empty_graph() -> GraphSnapshot {
    GraphSnapshot {
        schema_version: "0.1.0".to_owned(),
        run_id: "mcp".to_owned(),
        generated_at: crate::models::Timestamp::parse("1970-01-01T00:00:00Z").expect("epoch"),
        sources: Default::default(),
        nodes: Vec::new(),
        edges: Vec::new(),
    }
}
