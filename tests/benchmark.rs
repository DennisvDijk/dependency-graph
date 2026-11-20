//! Real-world benchmark: run the whole pipeline on a pinned open-source system
//! and score it against a hand-verified list of true edges.
//!
//! The thresholds are a ratchet. Raise them when the tool improves; do not
//! lower them to make a change pass.

use std::path::{Path, PathBuf};
use std::process::Command;

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_depgraph"))
}

fn run(dir: &Path, args: &[&str]) -> String {
    let out = Command::new(bin())
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run depgraph");
    assert!(
        out.status.success(),
        "depgraph {args:?} failed:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn benchmark(name: &str, min_precision: &str, min_recall: &str) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/benchmarks")
        .join(name);
    let work = tempfile::tempdir().expect("tempdir");
    let config = work.path().join("config");
    std::fs::create_dir_all(&config).expect("config dir");
    for entry in std::fs::read_dir(root.join("config")).expect("config") {
        let entry = entry.expect("entry");
        std::fs::copy(entry.path(), config.join(entry.file_name())).expect("copy");
    }
    let land = root.join("land");
    let land = if land.exists() {
        land
    } else {
        root.join("services")
    };
    // The landscape reader wants fixture.yaml and deployments.yaml beside the projects.
    let staged = work.path().join("land");
    copy_dir(&land, &staged);
    for file in ["fixture.yaml", "deployments.yaml"] {
        std::fs::copy(root.join(file), staged.join(file)).expect("landscape file");
    }
    run(
        work.path(),
        &[
            "collect",
            "--set",
            "full",
            "--fixture",
            staged.to_str().unwrap(),
            "--output",
            "data",
        ],
    );
    run(
        work.path(),
        &[
            "build",
            "--raw",
            "data/raw",
            "--output",
            "data/snapshot.json",
            "--aliases",
            "config/aliases.yaml",
        ],
    );
    let truth = root.join("truth.yaml");
    let report = run(
        work.path(),
        &[
            "eval",
            "--snapshot",
            "data/snapshot.json",
            "--truth",
            truth.to_str().unwrap(),
            "--min-precision",
            min_precision,
            "--min-recall",
            min_recall,
        ],
    );
    println!("{report}");
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).expect("mkdir");
    for entry in std::fs::read_dir(from).expect("read_dir") {
        let entry = entry.expect("entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("type").is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

#[test]
fn online_boutique() {
    benchmark("boutique", "0.95", "0.95");
}

#[test]
fn sock_shop() {
    benchmark("sockshop", "0.95", "0.95");
}
