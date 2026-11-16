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

fn inventory_csv(
    args: &[&str],
    token: &str,
    per_page: &str,
) -> (String, String, common::gitlab_mock::MockGitLab) {
    let mock = common::gitlab_mock::spawn();
    let output = tempfile::NamedTempFile::new().unwrap();
    let path = output.path().to_path_buf();
    let mut command_args = vec![
        "inventory",
        "--output",
        path.to_str().unwrap(),
        "--base-url",
        &mock.base_url,
        "--per-page",
        per_page,
    ];
    command_args.extend_from_slice(args);
    let asserted = depgraph()
        .env("GITLAB_TOKEN", token)
        .args(&command_args)
        .assert()
        .success();
    let stderr = String::from_utf8(asserted.get_output().stderr.clone()).unwrap();
    (fs::read_to_string(path).unwrap(), stderr, mock)
}

#[test]
fn subgroup_path_collects_projects_at_every_depth() {
    let (csv, _, mock) = inventory_csv(&["--group", "org/infra/platform"], "fixture-token", "100");
    assert!(csv.contains("org/infra/platform/api"));
    assert!(csv.contains("org/infra/platform/edge/gateway"));
    assert!(csv.contains("org/infra/platform/edge/mesh/sidecar"));
    assert!(!csv.contains("platform/root-app"));
    let requests = mock.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request
            .path
            .contains("/groups/org%2Finfra%2Fplatform/projects")
            && request.path.contains("include_subgroups=true")
    }));
}

#[test]
fn explicit_projects_are_fetched_by_path_and_skip_archived() {
    let (csv, stderr, mock) = inventory_csv(
        &[
            "--project",
            "org/other-team/ingress-gateway",
            "--project",
            "org/other-team/old-archived",
        ],
        "fixture-token",
        "100",
    );
    assert!(csv.contains("org/other-team/ingress-gateway"));
    assert!(!csv.contains("old-archived"));
    assert!(stderr.contains("warning: skipping archived project org/other-team/old-archived"));
    let requests = mock.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request
            .path
            .contains("/projects/org%2Fother-team%2Fingress-gateway")
    }));
    assert!(
        requests
            .iter()
            .all(|request| { request.path.split('?').next() != Some("/api/v4/projects") })
    );
}

#[test]
fn groups_and_projects_are_a_deduped_union() {
    let (csv, _, _) = inventory_csv(
        &[
            "--group",
            "platform",
            "--group",
            "org/infra/platform",
            "--project",
            "org/infra/platform/api",
        ],
        "fixture-token",
        "100",
    );
    let rows: Vec<_> = csv
        .lines()
        .skip(1)
        .filter(|line| !line.is_empty())
        .collect();
    assert_eq!(rows.len(), 4, "{csv}");
    assert_eq!(csv.matches("org/infra/platform/api").count(), 1);
    assert!(csv.contains("platform/root-app"));
    assert!(csv.contains("org/infra/platform/edge/gateway"));
    assert!(csv.contains("org/infra/platform/edge/mesh/sidecar"));
}

#[test]
fn empty_groups_and_projects_scan_every_visible_project() {
    let (csv, _, mock) = inventory_csv(&[], "fixture-token", "1");
    assert!(csv.contains("team/app"));
    assert!(csv.contains("other/public"));
    assert!(!csv.contains("team/fork"));
    assert!(!csv.contains("team/old"));
    let requests = mock.requests.lock().unwrap();
    assert!(requests.iter().any(|request| {
        request.path.starts_with("/api/v4/projects?")
            && request.path.contains("membership=false")
            && request.path.contains("simple=true")
            && request.path.contains("pagination=keyset")
            && request.path.contains("archived=false")
    }));
    assert!(
        requests
            .iter()
            .any(|request| request.path.contains("id_after="))
    );
}
