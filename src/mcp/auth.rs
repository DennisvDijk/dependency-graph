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

/// Claims we enforce besides the signature.
#[derive(Debug, Deserialize)]
struct AccessClaims {
    #[serde(default)]
    scope: String,
    #[serde(default)]
    scp: Vec<String>,
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
    issuer: &str,
    audience: &str,
    jwks_url: &str,
    cache: &tokio::sync::Mutex<JwksCache>,
    http: &reqwest::Client,
) -> Result<(), Error> {
    let header =
        decode_header(token).map_err(|_| Error::Validation("token was rejected".into()))?;
    let algorithm = header.alg;
    let kid = header.kid.unwrap_or_default();
    let jwk = load_jwk(jwks_url, &kid, cache, http, false).await?;
    let key =
        DecodingKey::from_jwk(&jwk).map_err(|_| Error::Validation("token was rejected".into()))?;
    let mut validation = Validation::new(algorithm);
    validation.set_audience(&[audience]);
    validation.set_issuer(&[issuer]);
    validation.validate_exp = true;
    let data = decode::<AccessClaims>(token, &key, &validation);
    let data = match data {
        Ok(data) => data,
        Err(_) => {
            let jwk = load_jwk(jwks_url, &kid, cache, http, true).await?;
            let key = DecodingKey::from_jwk(&jwk)
                .map_err(|_| Error::Validation("token was rejected".into()))?;
            decode::<AccessClaims>(token, &key, &validation)
                .map_err(|_| Error::Validation("token was rejected".into()))?
        }
    };
    if !has_scope(&data.claims) {
        return Err(Error::Validation("token was rejected".into()));
    }
    Ok(())
}

fn has_scope(claims: &AccessClaims) -> bool {
    claims
        .scope
        .split_whitespace()
        .any(|scope| scope == "depgraph:read")
        || claims.scp.iter().any(|scope| scope == "depgraph:read")
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

/// Whether `bind` is a loopback socket.
#[must_use]
pub fn is_loopback(bind: &str) -> bool {
    let host = bind.rsplit_once(':').map(|(host, _)| host).unwrap_or(bind);
    let host = host.trim_matches(['[', ']']);
    matches!(host, "127.0.0.1" | "localhost" | "::1")
}
