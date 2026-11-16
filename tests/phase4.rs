use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{SystemTime, UNIX_EPOCH};

use assert_cmd::Command;
use base64::Engine;
use depgraph::config::parse_config;
use depgraph::mcp::{
    AuthKind, authorize_with_jwks, entra_endpoints, jwks_from_discovery, policy_from_config,
};
use jsonwebtoken::{EncodingKey, Header};
use predicates::prelude::*;
use serde::Serialize;

fn depgraph() -> Command {
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

fn token(
    secret: &[u8],
    iss: &str,
    aud: &str,
    scp: &str,
    roles: &[&str],
    groups: &[&str],
    nbf: usize,
) -> String {
    #[derive(Serialize)]
    struct Claims<'a> {
        iss: &'a str,
        aud: &'a str,
        exp: usize,
        nbf: usize,
        scp: &'a str,
        roles: &'a [&'a str],
        groups: &'a [&'a str],
    }
    let exp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        + 600;
    let mut header = Header::new(jsonwebtoken::Algorithm::HS256);
    header.kid = Some("test".into());
    jsonwebtoken::encode(
        &header,
        &Claims {
            iss,
            aud,
            exp,
            nbf,
            scp,
            roles,
            groups,
        },
        &EncodingKey::from_secret(secret),
    )
    .unwrap()
}

fn jwks(secret: &[u8]) -> String {
    let k = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(secret);
    format!(r#"{{"keys":[{{"kty":"oct","k":"{k}","kid":"test","alg":"HS256"}}]}}"#)
}

#[test]
fn entra_accepts_delegated_scope_or_app_role_and_rejects_the_wrong_group() {
    let secret = b"phase4-entra-test-key-32-bytes!";
    let body = jwks(secret);
    let yaml =
        include_str!("../config/config.yaml").replace("provider: generic", "provider: entra");
    let mut config = parse_config(&yaml).unwrap();
    config.auth.allowed_groups = vec!["group-placeholder".into()];
    let (policy, jwks_url) = policy_from_config(&config).unwrap();
    assert_eq!(policy.kind, AuthKind::Entra);
    assert!(jwks_url.contains("00000000-0000-0000-0000-000000000000/discovery/v2.0/keys"));
    assert!(policy.validate_nbf);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as usize
        - 30;
    let scoped = token(
        secret,
        &policy.issuer,
        &policy.audiences[0],
        "depgraph.read",
        &[],
        &["group-placeholder"],
        now,
    );
    authorize_with_jwks(&scoped, &policy, &body).unwrap();
    let role = token(
        secret,
        &policy.issuer,
        &policy.audiences[0],
        "",
        &["Depgraph.Read"],
        &["group-placeholder"],
        now,
    );
    authorize_with_jwks(&role, &policy, &body).unwrap();
    let outsider = token(
        secret,
        &policy.issuer,
        &policy.audiences[0],
        "depgraph.read",
        &[],
        &["other-group"],
        now,
    );
    assert!(authorize_with_jwks(&outsider, &policy, &body).is_err());
    let future = token(
        secret,
        &policy.issuer,
        &policy.audiences[0],
        "depgraph.read",
        &[],
        &["group-placeholder"],
        now + 10_000,
    );
    assert!(authorize_with_jwks(&future, &policy, &body).is_err());
    let discovery = r#"{"issuer":"https://login.microsoftonline.com/00000000-0000-0000-0000-000000000000/v2.0","jwks_uri":"https://login.microsoftonline.com/00000000-0000-0000-0000-000000000000/discovery/v2.0/keys"}"#;
    let endpoints = entra_endpoints("00000000-0000-0000-0000-000000000000").unwrap();
    assert_eq!(jwks_from_discovery(discovery).unwrap(), endpoints.jwks);
    assert!(entra_endpoints("not a tenant").is_err());
}

#[test]
fn init_doctor_and_mcp_config_are_stable() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    fs::create_dir(&bin).unwrap();
    let glab = bin.join("glab");
    fs::write(
        &glab,
        "#!/bin/sh\nif [ \"$1\" = config ]; then printf '%s\\n' 'gitlab.example.com'; exit 0; fi\nif [ \"$1\" = auth ]; then exit 0; fi\nprintf '%s\\n' 'fixture-token-should-not-appear'\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&glab).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&glab, perms).unwrap();
    let path = format!("{}:/usr/bin:/bin", bin.display());

    let init = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(dir.path())
        .env("PATH", &path)
        .env_remove("GITLAB_TOKEN")
        .args(["init"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let init = String::from_utf8(init).unwrap();
    assert!(init.contains("wrote config/config.yaml"));
    assert!(init.contains("next: depgraph doctor"));
    let config = fs::read_to_string(dir.path().join("config/config.yaml")).unwrap();
    assert!(config.contains("https://gitlab.example.com"));
    assert!(config.contains("provider: generic"));
    assert!(!config.contains("fixture-token"));
    assert!(!init.contains("fixture-token"));

    fs::create_dir(dir.path().join("data")).unwrap();
    fs::write(
        dir.path().join("data/snapshot.json"),
        r#"{"schema_version":"0.1.0","run_id":"local","generated_at":"2026-01-15T00:00:00Z","sources":{},"nodes":[],"edges":[]}"#,
    )
    .unwrap();
    let doctor = Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(dir.path())
        .env("PATH", &path)
        .env_remove("GITLAB_TOKEN")
        .args(["doctor", "--now", "2026-01-16T00:00:00Z"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let doctor = String::from_utf8(doctor).unwrap();
    assert!(doctor.contains("ok config"));
    assert!(doctor.contains("ok snapshot: fresh"));
    assert!(doctor.contains("ok glab"));
    assert!(doctor.contains("ok token: a GitLab token is available"));
    assert!(!doctor.contains("fixture-token"));

    let cursor = depgraph()
        .args(["mcp-config", "cursor"])
        .assert()
        .success()
        .stdout(predicate::str::contains("http://127.0.0.1:8080/mcp"))
        .get_output()
        .stdout
        .clone();
    assert_eq!(
        String::from_utf8(cursor).unwrap(),
        fs::read_to_string("tests/fixtures/phase4/mcp-cursor.json").unwrap()
    );
    for client in ["claude", "vscode", "opencode"] {
        let output = depgraph()
            .args(["mcp-config", client])
            .assert()
            .success()
            .get_output()
            .stdout
            .clone();
        assert_eq!(
            String::from_utf8(output).unwrap(),
            fs::read_to_string(format!("tests/fixtures/phase4/mcp-{client}.json")).unwrap()
        );
    }
    depgraph()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("depgraph"));
}

#[test]
fn build_archives_a_run_that_at_can_reload() {
    let dir = tempfile::tempdir().unwrap();
    let raw = dir.path().join("raw");
    fs::create_dir(&raw).unwrap();
    fs::write(
        raw.join("gitlab_inventory.jsonl"),
        "{\"collector\":\"gitlab_inventory\",\"kind\":\"component\",\"payload\":{\"name\":\"payments-api\",\"depends_on\":[]}}\n",
    )
    .unwrap();
    let snapshot = dir.path().join("snapshot.json");
    Command::cargo_bin("depgraph")
        .unwrap()
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "build",
            "--raw",
            raw.to_str().unwrap(),
            "--output",
            snapshot.to_str().unwrap(),
            "--aliases",
            "config/aliases.yaml",
        ])
        .assert()
        .success();
    let runs = snapshot.parent().unwrap().join("runs");
    let listed = depgraph::store::list_runs(&runs).unwrap();
    assert_eq!(listed.len(), 1);
    let reloaded = depgraph::store::load_run(&runs, &listed[0]).unwrap();
    assert!(
        reloaded
            .nodes
            .iter()
            .any(|node| node.id == "component:payments-api")
    );
}
