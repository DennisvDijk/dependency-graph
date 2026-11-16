mod common;

use std::fs;
use std::path::PathBuf;

use assert_cmd::Command;
use predicates::prelude::*;

fn depgraph() -> Command {
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command.current_dir(env!("CARGO_MANIFEST_DIR"));
    command
}

#[test]
fn inventory_uses_recorded_gitlab_pages_and_hides_the_token() {
    let mock = common::gitlab_mock::spawn();
    let output = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("phase0-inventory.csv");
    let token = "fixture-token";
    depgraph()
        .env("GITLAB_TOKEN", token)
        .args([
            "inventory",
            "--output",
            output.to_str().unwrap(),
            "--base-url",
            &mock.base_url,
            "--group",
            "example-group/pilot",
            "--per-page",
            "1",
            "--max-concurrency",
            "2",
        ])
        .assert()
        .success();
    let csv = fs::read_to_string(&output).unwrap();
    assert!(
        !csv.contains(token),
        "inventory output must not contain the token"
    );
    assert!(csv.contains("example-group/pilot/billing-api,main,2026-01-10T08:00:00Z,false,false,false,false,false,false,false,false"));
    assert!(csv.contains("example-group/pilot/payments-api,main,2026-01-15T12:00:00Z,true,true,true,true,true,true,true,true"));
    let requests = mock.requests.lock().unwrap();
    assert!(requests.iter().all(|request| request.token == token));
    let project_gets = requests
        .iter()
        .filter(|request| {
            request.path.contains("/projects?") || request.path.contains("/projects&")
        })
        .count();
    // One 429 plus two pages.
    assert!(
        project_gets >= 3,
        "expected pagination and a retry, got {project_gets}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.path.contains("page=2"))
    );
}

#[test]
fn glab_supplies_the_token_when_the_env_var_is_absent() {
    let mock = common::gitlab_mock::spawn();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("glab");
    fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$GLAB_ARGS\"\nprintf '%s\\n' 'fixture-token-from-glab'\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&script).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&script, permissions).unwrap();
    }
    let args_file = dir.path().join("args.txt");
    let output = dir.path().join("inventory.csv");
    let path = format!(
        "{}:{}",
        dir.path().display(),
        std::env::var("PATH").unwrap_or_default()
    );
    depgraph()
        .env_remove("GITLAB_TOKEN")
        .env("PATH", path)
        .env("GLAB_ARGS", &args_file)
        .args([
            "inventory",
            "--output",
            output.to_str().unwrap(),
            "--base-url",
            &mock.base_url,
            "--group",
            "example-group/pilot",
            "--per-page",
            "100",
        ])
        .assert()
        .success();
    let args = fs::read_to_string(args_file).unwrap();
    assert!(args.contains("config"));
    assert!(args.contains("--host"));
    let requests = mock.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.token == "fixture-token-from-glab")
    );
    let csv = fs::read_to_string(output).unwrap();
    assert!(!csv.contains("fixture-token-from-glab"));
}

#[test]
fn validate_catalog_resolves_references() {
    depgraph()
        .args([
            "validate-catalog",
            "tests/fixtures/catalog/payments-api.yaml",
            "tests/fixtures/catalog/billing-api.yaml",
            "tests/fixtures/catalog/payments-db.yaml",
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("valid"));
    depgraph()
        .args(["validate-catalog", "tests/fixtures/catalog/broken.yaml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not resolve"));
}

#[test]
fn resolve_env_prints_the_canonical_name() {
    depgraph()
        .args(["resolve-env", "staging"])
        .assert()
        .success()
        .stdout(predicate::str::contains("acc"));
    depgraph().args(["resolve-env", "qa"]).assert().failure();
}

#[test]
fn propose_catalog_dry_run_writes_a_draft() {
    let dir = tempfile::tempdir().unwrap();
    depgraph()
        .args([
            "propose-catalog",
            "--from",
            "tests/fixtures/repos/payments-api",
            "--output",
            dir.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("catalog-info.yaml"));
    let draft = fs::read_to_string(dir.path().join("payments-api/catalog-info.yaml")).unwrap();
    assert!(draft.contains("name: payments-api"));
    assert!(draft.contains("depgraph/gitlab-environments: production"));
    assert!(draft.contains("owner: ai-platform"));
}

#[test]
fn open_mr_creates_a_request_and_never_merges() {
    let mock = common::gitlab_mock::spawn();
    let dir = tempfile::tempdir().unwrap();
    depgraph()
        .env("GITLAB_TOKEN", "fixture-token")
        .args([
            "propose-catalog",
            "--from",
            "tests/fixtures/repos/payments-api",
            "--output",
            dir.path().to_str().unwrap(),
            "--open-mr",
            "--project",
            "example-group/pilot/payments-api",
            "--base-url",
            &mock.base_url,
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("merge_requests/1"));
    let requests = mock.requests.lock().unwrap();
    assert!(
        requests.iter().any(|request| {
            request.method == "POST" && request.path.ends_with("/merge_requests")
        })
    );
    assert!(
        requests
            .iter()
            .all(|request| !request.path.ends_with("/merge"))
    );
}
