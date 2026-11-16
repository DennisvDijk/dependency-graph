//! Recorded GitLab API used by inventory and propose-catalog tests.
//!
//! The server runs on its own thread so a blocking CLI child cannot deadlock
//! the test runtime.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;

use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Recorded {
    pub method: String,
    pub path: String,
    pub token: String,
}

pub struct MockGitLab {
    pub base_url: String,
    pub requests: Arc<Mutex<Vec<Recorded>>>,
}

pub fn spawn() -> MockGitLab {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let recorded = requests.clone();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gitlab");
    thread::spawn(move || {
        let mut project_hits = 0_u32;
        let mut all_hits = 0_u32;
        for stream in listener.incoming() {
            let mut stream = match stream {
                Ok(stream) => stream,
                Err(_) => continue,
            };
            let mut buffer = [0_u8; 16384];
            let size = stream.read(&mut buffer).unwrap_or(0);
            let text = String::from_utf8_lossy(&buffer[..size]);
            let mut lines = text.lines();
            let request = lines.next().unwrap_or("");
            let mut parts = request.split_whitespace();
            let method = parts.next().unwrap_or("").to_owned();
            let target = parts.next().unwrap_or("").to_owned();
            let mut token = String::new();
            for line in text.lines() {
                if let Some((name, value)) = line.split_once(':')
                    && name.eq_ignore_ascii_case("private-token")
                {
                    token = value.trim().to_owned();
                }
            }
            recorded.lock().unwrap().push(Recorded {
                method: method.clone(),
                path: target.clone(),
                token,
            });
            let path = target.split('?').next().unwrap_or("");
            let query = target.split_once('?').map(|(_, query)| query).unwrap_or("");
            if path.ends_with("/merge") {
                let _ = write_response(&mut stream, 500, br#"{"message":"merge is forbidden"}"#);
                continue;
            }
            if method == "GET" && path == "/api/v4/projects" {
                all_hits += 1;
                if all_hits == 1 {
                    let _ = write_response_headers(
                        &mut stream,
                        429,
                        &[("retry-after", "0"), ("content-type", "application/json")],
                        br#"{"message":"rate limit"}"#,
                    );
                    continue;
                }
                let projects: Value = serde_json::from_str(
                    &std::fs::read_to_string(root.join("all-projects.json")).unwrap(),
                )
                .unwrap();
                let per_page = query_u32(query, "per_page").unwrap_or(100);
                let id_after = query_u32(query, "id_after").unwrap_or(0);
                let mut pending: Vec<Value> = projects
                    .as_array()
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|item| item["id"].as_u64().unwrap_or(0) > u64::from(id_after))
                    .collect();
                let page: Vec<Value> = pending
                    .drain(..pending.len().min(per_page as usize))
                    .collect();
                let link = page.last().and_then(|item| item["id"].as_u64()).and_then(|last| {
                    if pending.is_empty() {
                        None
                    } else {
                        Some(format!(
                            "</api/v4/projects?membership=false&archived=false&simple=true&pagination=keyset&order_by=id&sort=asc&per_page={per_page}&id_after={last}>; rel=\"next\""
                        ))
                    }
                });
                let body = serde_json::to_vec(&page).unwrap();
                let mut headers = vec![("content-type", "application/json")];
                let link_value = link.unwrap_or_default();
                if !link_value.is_empty() {
                    headers.push(("link", link_value.as_str()));
                }
                let _ = write_response_headers(&mut stream, 200, &headers, &body);
                continue;
            }
            if method == "GET" && path.contains("/groups/") && path.ends_with("/projects") {
                project_hits += 1;
                if project_hits == 1 {
                    let _ = write_response_headers(
                        &mut stream,
                        429,
                        &[("retry-after", "0"), ("content-type", "application/json")],
                        br#"{"message":"rate limit"}"#,
                    );
                    continue;
                }
                let group = group_id(path);
                let file = match group.as_str() {
                    "org/infra/platform" => "nested-platform.json",
                    "platform" => "platform-group.json",
                    _ => "projects.json",
                };
                let projects: Value =
                    serde_json::from_str(&std::fs::read_to_string(root.join(file)).unwrap())
                        .unwrap();
                let page = query_u32(query, "page").unwrap_or(1);
                let per_page = query_u32(query, "per_page").unwrap_or(100);
                let items = projects.as_array().cloned().unwrap_or_default();
                let start = ((page - 1) * per_page) as usize;
                let end = (start + per_page as usize).min(items.len());
                let slice = if start >= items.len() {
                    Vec::new()
                } else {
                    items[start..end].to_vec()
                };
                let next = if end < items.len() {
                    (page + 1).to_string()
                } else {
                    String::new()
                };
                let body = serde_json::to_vec(&slice).unwrap();
                let _ = write_response_headers(
                    &mut stream,
                    200,
                    &[
                        ("content-type", "application/json"),
                        ("x-next-page", next.as_str()),
                    ],
                    &body,
                );
                continue;
            }
            if method == "GET" && path.contains("/repository/tree") {
                let id = project_id(path);
                let body = std::fs::read(root.join(format!("tree/{id}.json")))
                    .unwrap_or_else(|_| b"[]".to_vec());
                let _ = write_response(&mut stream, 200, &body);
                continue;
            }
            if method == "GET" && path.contains("/jobs") {
                let id = project_id(path);
                let body = std::fs::read(root.join(format!("jobs/{id}.json")))
                    .unwrap_or_else(|_| b"[]".to_vec());
                let _ = write_response(&mut stream, 200, &body);
                continue;
            }
            if method == "GET" && path.contains("/repository/files/") {
                let id = project_id(path);
                let encoded = path
                    .split("/repository/files/")
                    .nth(1)
                    .unwrap_or("")
                    .split('?')
                    .next()
                    .unwrap_or("");
                let file = percent_decode(encoded);
                let disk = root.join(format!("files/{id}")).join(&file);
                if let Ok(bytes) = std::fs::read(&disk) {
                    let content =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes);
                    let payload = serde_json::json!({"content": content, "encoding": "base64"});
                    let _ =
                        write_response(&mut stream, 200, &serde_json::to_vec(&payload).unwrap());
                } else {
                    let _ = write_response(&mut stream, 404, br#"{"message":"404"}"#);
                }
                continue;
            }
            if method == "GET"
                && path.starts_with("/api/v4/projects/")
                && !path.contains("/repository/")
                && !path.contains("/jobs")
            {
                let encoded = path.trim_start_matches("/api/v4/projects/");
                let project_path = percent_decode(encoded);
                let payload = match project_path.as_str() {
                    "example-group/pilot/payments-api" => serde_json::json!({
                        "id": 101,
                        "path_with_namespace": project_path,
                        "default_branch": "main"
                    }),
                    "org/other-team/ingress-gateway" => serde_json::json!({
                        "id": 401,
                        "path_with_namespace": project_path,
                        "default_branch": "main",
                        "last_activity_at": "2026-06-01T00:00:00Z",
                        "archived": false,
                        "visibility": "private"
                    }),
                    "org/other-team/old-archived" => serde_json::json!({
                        "id": 402,
                        "path_with_namespace": project_path,
                        "default_branch": "main",
                        "archived": true
                    }),
                    "org/infra/platform/api" => serde_json::json!({
                        "id": 301,
                        "path_with_namespace": project_path,
                        "default_branch": "main",
                        "last_activity_at": "2026-06-01T00:00:00Z",
                        "archived": false
                    }),
                    _ => serde_json::Value::Null,
                };
                if payload.is_null() {
                    let _ = write_response(&mut stream, 404, br#"{"message":"404"}"#);
                } else {
                    let _ =
                        write_response(&mut stream, 200, &serde_json::to_vec(&payload).unwrap());
                }
                continue;
            }
            if method == "POST" && path.ends_with("/repository/commits") {
                let _ = write_response(&mut stream, 201, br#"{"id":"abc"}"#);
                continue;
            }
            if method == "POST" && path.ends_with("/merge_requests") {
                let payload = br#"{"web_url":"https://gitlab.example.com/example-group/pilot/payments-api/-/merge_requests/1"}"#;
                let _ = write_response(&mut stream, 201, payload);
                continue;
            }
            let _ = write_response(&mut stream, 404, br#"{"message":"not found"}"#);
        }
    });
    MockGitLab {
        base_url: format!("http://{address}"),
        requests,
    }
}

fn group_id(path: &str) -> String {
    let rest = path.split("/groups/").nth(1).unwrap_or("");
    let id = rest.split("/projects").next().unwrap_or("");
    percent_decode(id)
}

fn project_id(path: &str) -> &str {
    path.split("/projects/")
        .nth(1)
        .unwrap_or("")
        .split('/')
        .next()
        .unwrap_or("")
}

fn query_u32(query: &str, key: &str) -> Option<u32> {
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        if name == key {
            value.parse().ok()
        } else {
            None
        }
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::new();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = &value[index + 1..index + 3];
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn write_response(stream: &mut impl Write, status: u16, body: &[u8]) -> std::io::Result<()> {
    write_response_headers(
        stream,
        status,
        &[("content-type", "application/json")],
        body,
    )
}

fn write_response_headers(
    stream: &mut impl Write,
    status: u16,
    headers: &[(&str, &str)],
    body: &[u8],
) -> std::io::Result<()> {
    let reason = match status {
        200 => "OK",
        201 => "Created",
        404 => "Not Found",
        429 => "Too Many Requests",
        _ => "Error",
    };
    let mut head = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-length: {}\r\nconnection: close\r\n",
        body.len()
    );
    for (name, value) in headers {
        head.push_str(name);
        head.push_str(": ");
        head.push_str(value);
        head.push_str("\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    Ok(())
}
