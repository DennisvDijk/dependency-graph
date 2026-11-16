//! OAuth 2.1 resource-server checks.
//!
//! depgraph does not implement PKCE and does not forward the access token.
//! JWKS documents are fetched with a client that has no Authorization header.

use std::time::{Duration, Instant};

use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::Error;
use crate::redact;

/// How long a JWKS document is reused before the next `kid` forces a refresh.
const JWKS_TTL: Duration = Duration::from_secs(600);

/// Cached JWKS body. The access token is never stored here.
#[derive(Debug, Default)]
pub struct JwksCache {
    body: Option<String>,
    fetched: Option<Instant>,
}

/// Which issuer rules apply. Generic is the default resource server.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthKind {
    /// Any issuer. Scope `depgraph:read` in `scope` or `scp`.
    Generic,
    /// Microsoft Entra ID. Read access is `scp` and/or `roles`.
    Entra,
}

/// What a token must satisfy after the signature check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessPolicy {
    /// Expected `iss`.
    pub issuer: String,
    /// Accepted `aud` values. The resource indicator is first.
    pub audiences: Vec<String>,
    /// Generic or Entra.
    pub kind: AuthKind,
    /// When non-empty, the token's `groups` must overlap.
    pub allowed_groups: Vec<String>,
    /// When non-empty, the token's `roles` must overlap.
    pub allowed_roles: Vec<String>,
    /// Reject tokens that are not yet valid.
    pub validate_nbf: bool,
}

impl AccessPolicy {
    /// Default resource-server policy: one issuer, one audience, scope `depgraph:read`.
    #[must_use]
    pub fn generic(issuer: impl Into<String>, audience: impl Into<String>) -> Self {
        Self {
            issuer: issuer.into(),
            audiences: vec![audience.into()],
            kind: AuthKind::Generic,
            allowed_groups: Vec::new(),
            allowed_roles: Vec::new(),
            validate_nbf: false,
        }
    }
}

/// Claims we enforce besides the signature.
#[derive(Debug, Deserialize)]
struct AccessClaims {
    #[serde(default)]
    scope: String,
    #[serde(default, deserialize_with = "string_or_list")]
    scp: Vec<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    roles: Vec<String>,
    #[serde(default, deserialize_with = "string_or_list")]
    groups: Vec<String>,
}

fn string_or_list<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    match value {
        serde_json::Value::String(text) => Ok(text.split_whitespace().map(str::to_owned).collect()),
        serde_json::Value::Array(items) => Ok(items
            .into_iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect()),
        _ => Ok(Vec::new()),
    }
}

/// Entra v2 endpoints for one tenant. No network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntraEndpoints {
    /// `https://login.microsoftonline.com/<tenant>/v2.0`
    pub issuer: String,
    /// OpenID discovery document.
    pub discovery: String,
    /// JWKS URL published by that document for the v2 endpoint.
    pub jwks: String,
}

/// Build the v2 issuer, discovery URL, and conventional JWKS URL.
///
/// # Errors
///
/// Returns an error when `tenant` is empty or not a GUID, `common`, or `organizations`.
pub fn entra_endpoints(tenant: &str) -> Result<EntraEndpoints, Error> {
    if !valid_tenant(tenant) {
        return Err(Error::Config(
            "auth.tenant_id must be a GUID, common, or organizations. next: set auth.tenant_id or DEPGRAPH_ENTRA_TENANT_ID"
                .into(),
        ));
    }
    let issuer = format!("https://login.microsoftonline.com/{tenant}/v2.0");
    Ok(EntraEndpoints {
        discovery: format!("{issuer}/.well-known/openid-configuration"),
        jwks: format!("https://login.microsoftonline.com/{tenant}/discovery/v2.0/keys"),
        issuer,
    })
}

/// Read `jwks_uri` from an OpenID discovery document.
///
/// # Errors
///
/// Returns an error when the document has no `jwks_uri`.
pub fn jwks_from_discovery(body: &str) -> Result<String, Error> {
    let value: serde_json::Value = serde_json::from_str(body)
        .map_err(|_| Error::Config("openid discovery was not JSON".into()))?;
    value
        .get("jwks_uri")
        .and_then(|item| item.as_str())
        .map(str::to_owned)
        .ok_or_else(|| Error::Config("openid discovery has no jwks_uri".into()))
}

fn valid_tenant(tenant: &str) -> bool {
    if matches!(tenant, "common" | "organizations") {
        return true;
    }
    let parts: Vec<&str> = tenant.split('-').collect();
    parts.len() == 5
        && parts.iter().map(|part| part.len()).eq([8, 4, 4, 4, 12])
        && tenant
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-')
}

/// Protected Resource Metadata (RFC 9728).
#[must_use]
pub fn protected_resource_metadata(audience: &str, issuer: &str) -> Value {
    json!({
        "resource": audience,
        "authorization_servers": [issuer],
        "scopes_supported": ["depgraph:read"],
        "bearer_methods_supported": ["header"]
    })
}

/// `WWW-Authenticate` value for a 401, pointing at the metadata URL.
#[must_use]
pub fn www_authenticate(audience: &str) -> String {
    let metadata = metadata_url(audience);
    format!("Bearer realm=\"depgraph\", error=\"invalid_token\", resource_metadata=\"{metadata}\"")
}

/// Validate a bearer token against the issuer JWKS.
///
/// The token string is not included in the error.
///
/// # Errors
///
/// Returns an error when the signature, issuer, audience, expiry, or scope fails.
pub async fn authorize(
    token: &str,
    policy: &AccessPolicy,
    jwks_url: &str,
    cache: &tokio::sync::Mutex<JwksCache>,
    http: &reqwest::Client,
) -> Result<(), Error> {
    let header =
        decode_header(token).map_err(|_| Error::Validation("token was rejected".into()))?;
    let kid = header.kid.unwrap_or_default();
    let jwk = load_jwk(jwks_url, &kid, cache, http, false).await?;
    match authorize_with_jwk(token, policy, &jwk) {
        Ok(()) => Ok(()),
        Err(_) => {
            let jwk = load_jwk(jwks_url, &kid, cache, http, true).await?;
            authorize_with_jwk(token, policy, &jwk)
        }
    }
}

/// Validate a token against a JWKS document. Does not open a socket.
///
/// # Errors
///
/// Returns an error when the token is rejected. The token text is not included.
pub fn authorize_with_jwks(
    token: &str,
    policy: &AccessPolicy,
    jwks_body: &str,
) -> Result<(), Error> {
    let header =
        decode_header(token).map_err(|_| Error::Validation("token was rejected".into()))?;
    let kid = header.kid.unwrap_or_default();
    let jwk =
        find_jwk(jwks_body, &kid).ok_or_else(|| Error::Validation("token was rejected".into()))?;
    authorize_with_jwk(token, policy, &jwk)
}

fn authorize_with_jwk(
    token: &str,
    policy: &AccessPolicy,
    jwk: &jsonwebtoken::jwk::Jwk,
) -> Result<(), Error> {
    let header =
        decode_header(token).map_err(|_| Error::Validation("token was rejected".into()))?;
    let key =
        DecodingKey::from_jwk(jwk).map_err(|_| Error::Validation("token was rejected".into()))?;
    let mut validation = Validation::new(header.alg);
    let audiences: Vec<&str> = policy.audiences.iter().map(String::as_str).collect();
    validation.set_audience(&audiences);
    validation.set_issuer(&[&policy.issuer]);
    validation.validate_exp = true;
    validation.validate_nbf = policy.validate_nbf;
    let data = decode::<AccessClaims>(token, &key, &validation)
        .map_err(|_| Error::Validation("token was rejected".into()))?;
    if !has_access(policy, &data.claims) {
        return Err(Error::Validation("token was rejected".into()));
    }
    Ok(())
}

fn has_access(policy: &AccessPolicy, claims: &AccessClaims) -> bool {
    let read = match policy.kind {
        AuthKind::Generic => {
            claims
                .scope
                .split_whitespace()
                .any(|scope| scope == "depgraph:read")
                || claims.scp.iter().any(|scope| scope == "depgraph:read")
        }
        AuthKind::Entra => {
            let mut scopes = claims
                .scope
                .split_whitespace()
                .chain(claims.scp.iter().map(String::as_str));
            let scope_ok = scopes
                .any(|scope| matches!(scope, "depgraph:read" | "depgraph.read" | "Depgraph.Read"));
            let role_ok = claims.roles.iter().any(|role| {
                matches!(
                    role.as_str(),
                    "depgraph:read" | "depgraph.read" | "Depgraph.Read"
                )
            });
            scope_ok || role_ok
        }
    };
    if !read {
        return false;
    }
    if !policy.allowed_groups.is_empty()
        && !claims
            .groups
            .iter()
            .any(|group| policy.allowed_groups.iter().any(|allowed| allowed == group))
    {
        return false;
    }
    if !policy.allowed_roles.is_empty()
        && !claims
            .roles
            .iter()
            .any(|role| policy.allowed_roles.iter().any(|allowed| allowed == role))
    {
        return false;
    }
    true
}

async fn load_jwk(
    jwks_url: &str,
    kid: &str,
    cache: &tokio::sync::Mutex<JwksCache>,
    http: &reqwest::Client,
    force: bool,
) -> Result<jsonwebtoken::jwk::Jwk, Error> {
    {
        let guard = cache.lock().await;
        let fresh = guard
            .fetched
            .is_some_and(|fetched| fetched.elapsed() < JWKS_TTL);
        if !force
            && fresh
            && let Some(body) = &guard.body
            && let Some(jwk) = find_jwk(body, kid)
        {
            return Ok(jwk);
        }
    }
    let response = http
        .get(jwks_url)
        .send()
        .await
        .map_err(|err| Error::Config(redact::redact(&format!("jwks fetch failed: {err}"))))?;
    if !response.status().is_success() {
        return Err(Error::Config(format!(
            "jwks fetch returned {}",
            response.status()
        )));
    }
    let body = response
        .text()
        .await
        .map_err(|err| Error::Config(redact::redact(&err.to_string())))?;
    let jwk = find_jwk(&body, kid).ok_or_else(|| Error::Validation("token was rejected".into()))?;
    let mut guard = cache.lock().await;
    guard.body = Some(body);
    guard.fetched = Some(Instant::now());
    Ok(jwk)
}

fn find_jwk(body: &str, kid: &str) -> Option<jsonwebtoken::jwk::Jwk> {
    let set: JwkSet = serde_json::from_str(body).ok()?;
    if kid.is_empty() {
        return set.keys.into_iter().next();
    }
    set.find(kid).cloned()
}

fn metadata_url(audience: &str) -> String {
    format!(
        "{}/.well-known/oauth-protected-resource",
        audience.trim_end_matches('/')
    )
}

/// Resolve the MCP access policy and the JWKS URL to fetch.
///
/// Entra tenant and client ids prefer `DEPGRAPH_ENTRA_TENANT_ID` and
/// `DEPGRAPH_ENTRA_CLIENT_ID` when those variables are non-empty. The values
/// are not included in errors.
///
/// # Errors
///
/// Returns an error when an Entra tenant id is not usable.
pub fn policy_from_config(config: &crate::config::Config) -> Result<(AccessPolicy, String), Error> {
    match config.auth.provider {
        crate::config::AuthProvider::Generic => Ok((
            AccessPolicy::generic(&config.mcp.issuer, &config.mcp.audience),
            config.mcp.jwks_url.clone(),
        )),
        crate::config::AuthProvider::Entra => {
            let tenant = env_or(&config.auth.tenant_id, "DEPGRAPH_ENTRA_TENANT_ID");
            let client = env_or(&config.auth.client_id, "DEPGRAPH_ENTRA_CLIENT_ID");
            let endpoints = entra_endpoints(&tenant)?;
            let audience = if config.auth.audience.is_empty() {
                format!("api://{client}")
            } else {
                config.auth.audience.clone()
            };
            let mut audiences = vec![audience];
            if !client.is_empty() && !audiences.contains(&client) {
                audiences.push(client);
            }
            Ok((
                AccessPolicy {
                    issuer: endpoints.issuer,
                    audiences,
                    kind: AuthKind::Entra,
                    allowed_groups: config.auth.allowed_groups.clone(),
                    allowed_roles: config.auth.allowed_roles.clone(),
                    validate_nbf: true,
                },
                endpoints.jwks,
            ))
        }
    }
}

fn env_or(configured: &str, key: &str) -> String {
    match std::env::var(key) {
        Ok(value) if !value.trim().is_empty() => value.trim().to_owned(),
        _ => configured.to_owned(),
    }
}

/// Whether `bind` is a loopback socket.
#[must_use]
pub fn is_loopback(bind: &str) -> bool {
    let host = bind.rsplit_once(':').map(|(host, _)| host).unwrap_or(bind);
    let host = host.trim_matches(['[', ']']);
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}
