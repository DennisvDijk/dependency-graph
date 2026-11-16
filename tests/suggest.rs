use assert_cmd::Command;
use depgraph::normalize::merge::{Unresolved, UnresolvedReport};
use std::fs;

#[test]
fn suggestions_are_review_only_and_ignore_ambiguous_names() {
    let dir = tempfile::tempdir().unwrap();
    let report = UnresolvedReport {
        schema_version: "1.0.0".into(),
        records: vec![
            Unresolved {
                kind: "resource".into(),
                name: "orders-py-prod-sg".into(),
                reason: "unmatched".into(),
                volume: 0,
            },
            Unresolved {
                kind: "environment".into(),
                name: "shop-prod".into(),
                reason: "namespace is not mapped".into(),
                volume: 4,
            },
            Unresolved {
                kind: "resource".into(),
                name: "unknown,\"name".into(),
                reason: "unmatched".into(),
                volume: 0,
            },
        ],
    };
    let unresolved = dir.path().join("unresolved.json");
    let snapshot = dir.path().join("snapshot.json");
    let environments = dir.path().join("environments.yaml");
    fs::write(&unresolved, serde_json::to_vec(&report).unwrap()).unwrap();
    let source = fs::read_to_string("tests/fixtures/phase1/graph.json").unwrap();
    let mut graph: serde_json::Value = serde_json::from_str(&source).unwrap();
    let node = graph["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["type"] == "component")
        .unwrap();
    node["id"] = "component:orders-py".into();
    fs::write(&snapshot, serde_json::to_vec(&graph).unwrap()).unwrap();
    let before = fs::read(&unresolved).unwrap();
    let result = Command::cargo_bin("depgraph")
        .unwrap()
        .args(["suggest", "--unresolved"])
        .arg(&unresolved)
        .arg("--snapshot")
        .arg(&snapshot)
        .arg("--environments")
        .arg(&environments)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    assert!(
        text.contains("\"orders-py-prod-sg\": \"orders-py\""),
        "{text}"
    );
    assert!(text.contains("\"shop-prod\""), "{text}");
    assert!(!text.contains("unknown"), "{text}");
    assert_eq!(fs::read(&unresolved).unwrap(), before);
    assert!(!environments.exists());
}

#[test]
fn openai_compatible_proposals_are_checked_against_the_input() {
    test_compatible_endpoint(false);
}

#[test]
fn openai_key_can_come_from_a_mounted_secret_file() {
    test_compatible_endpoint(true);
}

fn test_compatible_endpoint(use_file: bool) {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    let dir = tempfile::tempdir().unwrap();
    let report = UnresolvedReport {
        schema_version: "1.0.0".into(),
        records: vec![Unresolved {
            kind: "resource".into(),
            name: "legacy-sg".into(),
            reason: "unmatched".into(),
            volume: 0,
        }],
    };
    let unresolved = dir.path().join("unresolved.json");
    let snapshot = dir.path().join("snapshot.json");
    fs::write(&unresolved, serde_json::to_vec(&report).unwrap()).unwrap();
    let source = fs::read_to_string("tests/fixtures/phase1/graph.json").unwrap();
    let mut graph: serde_json::Value = serde_json::from_str(&source).unwrap();
    let node = graph["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|n| n["type"] == "component")
        .unwrap();
    node["id"] = "component:orders-py".into();
    fs::write(&snapshot, serde_json::to_vec(&graph).unwrap()).unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(std::time::Duration::from_secs(10)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 2048];
        loop {
            let count = stream.read(&mut buffer).unwrap();
            if count == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..count]);
            if let Some(end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&request[..end]);
                let length: usize = headers
                    .lines()
                    .find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|n| n.trim().parse().ok())
                    })
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let text = String::from_utf8(request).unwrap();
        assert!(text.starts_with("POST /v1/chat/completions "), "{text}");
        assert!(
            text.to_ascii_lowercase()
                .contains("authorization: bearer test-token")
        );
        let body: serde_json::Value =
            serde_json::from_str(text.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["model"], "test-model");
        assert!(
            body["messages"][0]["content"]
                .as_str()
                .unwrap()
                .contains("legacy-sg")
        );
        let response = serde_json::json!({"choices":[{"message":{"content":"{\"legacy-sg\":\"orders-py\",\"invented\":\"orders-py\"}"}}]}).to_string();
        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
    });
    let key_file = dir.path().join("mounted-secret");
    fs::write(&key_file, "test-token\n").unwrap();
    let mut command = Command::cargo_bin("depgraph").unwrap();
    command
        .env("HOME", dir.path())
        .env("XDG_CONFIG_HOME", dir.path())
        .env("APPDATA", dir.path())
        .env_remove("OPENAI_API_KEY")
        .env_remove("OPENAI_API_KEY_FILE");
    if use_file {
        command.env("OPENAI_API_KEY_FILE", &key_file);
    } else {
        command.env("OPENAI_API_KEY", "test-token");
    }
    let output = command
        .args(["suggest", "--unresolved"])
        .arg(&unresolved)
        .arg("--snapshot")
        .arg(&snapshot)
        .arg("--environments")
        .arg(dir.path().join("absent.yaml"))
        .args(["--llm", "--model-id", "test-model"])
        .env("OPENAI_BASE_URL", format!("http://{address}/v1"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("\"legacy-sg\": \"orders-py\" # LLM suggestion, check it"),
        "{text}"
    );
    assert!(!text.contains("invented"), "{text}");
    server.join().unwrap();
}
