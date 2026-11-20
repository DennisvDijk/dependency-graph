//! `depgraph contract-diff` on the real binary.

use std::fs;
use std::process::Command;

const OLD: &str = "openapi: 3.0.3\ninfo:\n  title: Pay API\n  version: '1'\npaths:\n  /a:\n    get: {}\n  /b:\n    post: {}\n";
const NEW_BREAKING: &str = "openapi: 3.0.3\ninfo:\n  title: Pay API\n  version: '2'\npaths:\n  /a:\n    get: {}\n  /c:\n    get: {}\n";
const NEW_ADDITIVE: &str = "openapi: 3.0.3\ninfo:\n  title: Pay API\n  version: '2'\npaths:\n  /a:\n    get: {}\n  /b:\n    post: {}\n  /c:\n    get: {}\n";

fn run(new: &str, extra: &[&str]) -> (bool, String) {
    let dir = tempfile::tempdir().unwrap();
    let (old_path, new_path) = (dir.path().join("old.yaml"), dir.path().join("openapi.yaml"));
    fs::write(&old_path, OLD).unwrap();
    fs::write(&new_path, new).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_depgraph"))
        .args(["contract-diff", "--old"])
        .arg(&old_path)
        .arg("--new")
        .arg(&new_path)
        .args(extra)
        .output()
        .unwrap();
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn a_removed_operation_is_breaking_and_can_fail_the_run() {
    let (ok, out) = run(NEW_BREAKING, &[]);
    assert!(ok);
    assert!(
        out.contains("BREAKING") && out.contains("removed POST /b") && out.contains("added GET /c"),
        "{out}"
    );
    let (ok, _) = run(NEW_BREAKING, &["--fail-on-breaking"]);
    assert!(!ok);
}

#[test]
fn an_added_operation_is_additive() {
    let (ok, out) = run(NEW_ADDITIVE, &["--fail-on-breaking"]);
    assert!(ok, "{out}");
    assert!(
        out.contains("additive") && !out.contains("BREAKING"),
        "{out}"
    );
}
