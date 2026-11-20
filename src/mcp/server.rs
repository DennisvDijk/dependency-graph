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

use crate::config::Aliases;
use crate::error::Error;
use crate::mcp::auth::{self, JwksCache};
use crate::models::GraphSnapshot;

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
    /// Archived runs, the folder `runs` next to the snapshot. `diff` reads it.
    pub runs: std::path::PathBuf,
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
        .route("/healthz", get(healthz))
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
    let state = Arc::new(build_state(
        insecure,
        policy,
        jwks_url,
        snapshot_path,
        aliases,
    )?);
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(Error::Io)?;
    axum::serve(listener, router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(Error::Io)
}

/// Load the snapshot and build the shared state.
pub(crate) fn build_state(
    insecure: bool,
    policy: crate::mcp::auth::AccessPolicy,
    jwks_url: &str,
    snapshot_path: &Path,
    aliases: Aliases,
) -> Result<McpState, Error> {
    let snapshot = if snapshot_path.is_file() {
        let text = std::fs::read_to_string(snapshot_path)?;
        serde_json::from_str(&text).map_err(|err| Error::Validation(err.to_string()))?
    } else {
        empty_graph()
    };
    Ok(McpState {
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
        runs: crate::store::runs_dir(snapshot_path),
        jwks: tokio::sync::Mutex::new(JwksCache::default()),
        http: reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|err| Error::Config(err.to_string()))?,
    })
}

/// Resolves on SIGTERM (a pod being stopped) or Ctrl-C, so open requests can
/// finish before the process exits.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            term.recv().await;
        } else {
            std::future::pending::<()>().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

/// Liveness for a container or load balancer. Reveals nothing about the graph.
async fn healthz() -> &'static str {
    "ok"
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
    if state.insecure && !local_request(&headers) {
        // A web page can point its own host name at 127.0.0.1 (DNS rebinding)
        // and then talk to this port. Only requests that name the loopback
        // host, and carry no foreign Origin, may pass without a token.
        return (StatusCode::FORBIDDEN, Json(json!({"error": "forbidden"}))).into_response();
    }
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
    // A JSON-RPC notification gets no body, only 202 Accepted.
    match crate::mcp::rpc::handle(&state, &body) {
        Some(reply) => Json(reply).into_response(),
        None => StatusCode::ACCEPTED.into_response(),
    }
}

/// Whether `Host` and `Origin` both name the loopback interface.
pub fn local_request(headers: &HeaderMap) -> bool {
    let loopback = |value: &str| {
        let host = value
            .trim_start_matches("http://")
            .trim_start_matches("https://");
        let host = host.split('/').next().unwrap_or(host);
        auth::is_loopback(host)
    };
    let host_ok = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(loopback);
    let origin_ok = headers
        .get(header::ORIGIN)
        .is_none_or(|value| value.to_str().ok().is_some_and(loopback));
    host_ok && origin_ok
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

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(host: &str, origin: Option<&str>) -> HeaderMap {
        let mut map = HeaderMap::new();
        map.insert(header::HOST, host.parse().unwrap());
        if let Some(origin) = origin {
            map.insert(header::ORIGIN, origin.parse().unwrap());
        }
        map
    }

    #[test]
    fn local_mode_accepts_only_loopback_host_and_origin() {
        assert!(local_request(&headers("127.0.0.1:8080", None)));
        assert!(local_request(&headers(
            "localhost:8080",
            Some("http://localhost:3000")
        )));
        assert!(local_request(&headers("[::1]:8080", None)));
        // DNS rebinding: the page's own name resolves to 127.0.0.1.
        assert!(!local_request(&headers("evil.example.com:8080", None)));
        assert!(!local_request(&headers(
            "127.0.0.1:8080",
            Some("https://evil.example.com")
        )));
        assert!(!local_request(&HeaderMap::new()));
    }
}
