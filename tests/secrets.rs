//! No secret that sits in a scanned repository may reach any file or any
//! message depgraph writes. Canary values are planted in Terraform, Python,
//! Compose and Kubernetes files; the test then reads every output.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

const CANARIES: &[&str] = &[
    "CANARYtfpassword8f3a",
    "CANARYurlpass41bd",
    "CANARYapikey77c2",
    "CANARYbearer09ee",
    "CANARYcomposeenv5d1f",
    "CANARYk8ssecret3a9c",
];

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/landscape")
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn append(root: &Path, file: &str, text: &str) {
    let path = root.join("landscape").join(file);
    let mut body = fs::read_to_string(&path).unwrap();
    body.push('\n');
    body.push_str(text);
    fs::write(path, body).unwrap();
}

fn files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files_under(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn depgraph(root: &Path) -> Command {
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command.current_dir(root);
    command
}

#[test]
fn planted_secrets_never_reach_any_output() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    copy_tree(&fixture(), &root.join("landscape"));
    let config = root.join("config");
    fs::create_dir_all(&config).unwrap();
    let repo_config = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("config");
    for name in ["config.yaml", "pilot.yaml", "sdk_hints.yaml"] {
        fs::copy(repo_config.join(name), config.join(name)).unwrap();
    }
    for name in ["aliases.yaml", "environments.yaml"] {
        fs::copy(fixture().join(name), config.join(name)).unwrap();
    }

    append(
        root,
        "aws-platform/databases.tf",
        r#"resource "aws_db_instance" "leaky" {
  identifier = "orders-db"
  password   = "CANARYtfpassword8f3a"
  tags = { dsn = "postgres://admin:CANARYurlpass41bd@orders-db:5432/orders" }
}"#,
    );
    append(
        root,
        "orders-py/src/orders/app.py",
        r#"API_KEY = "CANARYapikey77c2"
DATABASE_URL = "postgres://app:CANARYurlpass41bd@orders-db/orders"
HEADERS = {"Authorization": "Bearer CANARYbearer09ee"}"#,
    );
    append(
        root,
        "billing-ex/docker-compose.yml",
        "    environment:\n      DB_PASSWORD: CANARYcomposeenv5d1f\n      DATABASE_URL: postgres://u:CANARYurlpass41bd@postgres/db",
    );
    fs::write(
        root.join("landscape/billing-ex/deploy/secret.yaml"),
        "apiVersion: v1\nkind: Secret\nmetadata:\n  name: billing-secret\nstringData:\n  password: CANARYk8ssecret3a9c\n",
    )
    .unwrap();

    let mut messages = String::new();
    let mut run = |args: &[&str]| {
        let output = depgraph(root).args(args).output().unwrap();
        messages.push_str(&String::from_utf8_lossy(&output.stdout));
        messages.push_str(&String::from_utf8_lossy(&output.stderr));
    };
    run(&[
        "collect",
        "--set",
        "full",
        "--fixture",
        "landscape",
        "--prometheus-fixture",
        "landscape/prometheus",
        "--output",
        "data",
    ]);
    run(&["build"]);
    run(&["impact", "orders-db", "--snapshot", "data/snapshot.json"]);
    run(&["show", "billing-ex", "--snapshot", "data/snapshot.json"]);

    assert!(
        root.join("data/snapshot.json").exists(),
        "the build produced no snapshot, so the scan proves nothing:\n{messages}"
    );
    let mut files = Vec::new();
    files_under(&root.join("data"), &mut files);
    let mut leaks = Vec::new();
    for canary in CANARIES {
        if messages.contains(canary) {
            leaks.push(format!("console output holds {canary}"));
        }
        for file in &files {
            if fs::read_to_string(file).is_ok_and(|body| body.contains(canary)) {
                leaks.push(format!("{} holds {canary}", file.display()));
            }
        }
    }
    assert!(leaks.is_empty(), "secrets leaked:\n{}", leaks.join("\n"));
}
