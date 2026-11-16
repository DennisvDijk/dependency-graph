use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine;
use depgraph::config::Aliases;
use depgraph::mcp::{McpState, router};
use depgraph::models::{GraphSnapshot, Node, NodeType, Timestamp};
use jsonwebtoken::{EncodingKey, Header};
use serde::Serialize;
use serde_json::{Value, json};
use tower::ServiceExt;

fn snapshot() -> GraphSnapshot {
    let mut attrs = std::collections::BTreeMap::new();
    attrs.insert("project_path".into(), json!("org/payments-api"));
    attrs.insert(
        "web_url".into(),
        json!("https://gitlab.example.com/org/payments-api"),
    );
    attrs.insert("default_branch".into(), json!("main"));
    attrs.insert("owner".into(), json!("platform-team"));
    attrs.insert(
        "manifest_paths".into(),
        json!(["deploy/deployment.yaml", "istio/virtual-service.yaml"]),
    );
    GraphSnapshot {
        schema_version: "0.1.0".into(),
        run_id: "test".into(),
        generated_at: Timestamp::parse("2026-01-15T00:00:00Z").unwrap(),
        sources: Default::default(),
        nodes: vec![
            Node {
                id: "component:payments-api".into(),
                node_type: NodeType::Component,
                attrs,
            },
            Node {
                id: "component:billing-api".into(),
                node_type: NodeType::Component,
                attrs: Default::default(),
            },
        ],
        edges: Vec::new(),
    }
}

struct Issued {
    token: String,
    jwks: String,
    saw_authorization: Arc<Mutex<bool>>,
}

fn issue() -> Issued {
    let secret = b"phase3-local-test-key-32b-long!!";
    let k = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
    let jwks = format!(r#"{{"keys":[{{"kty":"oct","k":"{k}","kid":"test","alg":"HS256"}}]}}"#);
    #[derive(Serialize)]
    struct Claims {
        iss: &'static str,
        aud: &'static str,
        exp: usize,
        scope: &'static str,
    }
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + 600;
    let mut header = Header::new(jsonwebtoken::Algorithm::HS256);
    header.kid = Some("test".into());
    let token = jsonwebtoken::encode(
        &header,
        &Claims {
            iss: "https://idp.example.com",
            aud: "http://127.0.0.1:8080",
            exp,
            scope: "depgraph:read",
        },
        &EncodingKey::from_secret(secret),
    )
    .unwrap();
    Issued {
        token,
        jwks,
        saw_authorization: Arc::new(Mutex::new(false)),
    }
}

fn state(issued: &Issued) -> Arc<McpState> {
    let jwks = issued.jwks.clone();
    let flag = issued.saw_authorization.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut buf = [0_u8; 2048];
            let n = std::io::Read::read(&mut stream, &mut buf).unwrap_or(0);
            let text = String::from_utf8_lossy(&buf[..n]);
            if text.to_ascii_lowercase().contains("authorization:") {
                *flag.lock().unwrap() = true;
            }
            let body = jwks.as_bytes();
            let header = format!(
                "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                body.len()
            );
            let _ = std::io::Write::write_all(&mut stream, header.as_bytes());
            let _ = std::io::Write::write_all(&mut stream, body);
        }
    });
    let mut aliases = Aliases::default();
    aliases
        .aliases
        .insert("payments".into(), "payments-api".into());
    Arc::new(McpState {
        issuer: "https://idp.example.com".into(),
        audience: "http://127.0.0.1:8080".into(),
        jwks_url: format!("http://{address}/jwks"),
        insecure: false,
        snapshot: snapshot(),
        aliases,
        jwks: tokio::sync::Mutex::new(depgraph::mcp::JwksCache::default()),
        http: reqwest::Client::new(),
    })
}

async fn body_json(response: axum::response::Response) -> Value {
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    serde_json::from_slice(&bytes)
        .unwrap_or(json!({ "raw": String::from_utf8_lossy(&bytes).into_owned() }))
}

#[tokio::test]
async fn mcp_requires_a_scoped_token_and_does_not_forward_it() {
    let issued = issue();
    let app = router(state(&issued));
    let metadata = app
        .clone()
        .oneshot(
            Request::get("/.well-known/oauth-protected-resource")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(metadata.status(), StatusCode::OK);
    let doc = body_json(metadata).await;
    assert_eq!(doc["resource"], "http://127.0.0.1:8080");
    assert_eq!(doc["scopes_supported"][0], "depgraph:read");

    let anonymous = router(state(&issued))
        .oneshot(
            Request::post("/mcp")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#.to_owned(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);
    let header = anonymous
        .headers()
        .get("www-authenticate")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    assert!(header.contains("resource_metadata="));
    assert!(header.contains("/.well-known/oauth-protected-resource"));

    let authed = router(state(&issued))
        .oneshot(
            Request::post("/mcp")
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {}", issued.token))
                .body(Body::from(
                    r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"component_context","arguments":{"component":"payments"}}}"#.to_owned(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(authed.status(), StatusCode::OK);
    let payload = body_json(authed).await;
    let text = payload["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("org/payments-api"));
    assert!(text.contains("platform-team"));
    assert!(text.contains("istio/virtual-service.yaml"));
    assert!(!*issued.saw_authorization.lock().unwrap());
}

#[test]
fn dry_run_prints_the_mr_note_and_the_drift_issue() {
    let graph = snapshot();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("snapshot.json");
    std::fs::write(&path, depgraph::models::to_canonical_json(&graph).unwrap()).unwrap();
    let note = assert_cmd::Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "mr-check",
            "--dry-run",
            "--snapshot",
            path.to_str().unwrap(),
            "--project",
            "org/payments-api",
            "--changed-file",
            "openapi/api.yaml",
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let note = String::from_utf8(note).unwrap();
    assert!(note.contains("<!-- depgraph-mr-check -->"));
    assert!(note.contains("contract_change: yes"));
    assert!(note.contains("component: payments-api"));
    let drift = assert_cmd::Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "publish",
            "drift-issue",
            "--dry-run",
            "--snapshot",
            path.to_str().unwrap(),
        ])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let drift = String::from_utf8(drift).unwrap();
    assert!(drift.contains("# Drift prod"));
    assert!(!drift.contains("glpat-"));
}
