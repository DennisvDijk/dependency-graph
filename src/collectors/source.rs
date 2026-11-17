//! Read application source for the ties it spells out: the services it calls,
//! the databases it connects to and the queues and topics it publishes to or
//! consumes from.
//!
//! This is deliberately shallow. It looks for literal names (a URL, a
//! connection string, `queue_url="…"`) in Python, Elixir, Rust, JavaScript,
//! TypeScript, Go and Java files. A name built at run time is not guessed.
//! Only the host or queue name is kept; no credential in a connection string
//! ever leaves this module.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use regex::Regex;
use serde_json::json;

use crate::raw::RawRecord;

/// Source files read per project. Files that usually hold integration code
/// come first, so a large repository does not hide them behind the cap.
pub const MAX_SOURCE_FILES: usize = 100;
/// Larger files are generated or data, not application code.
const MAX_FILE_BYTES: usize = 400_000;

const EXTENSIONS: [&str; 12] = [
    ".py",
    ".ex",
    ".exs",
    ".rs",
    ".js",
    ".mjs",
    ".ts",
    ".go",
    ".java",
    ".kt",
    ".cs",
    ".properties",
];
const SKIPPED_DIRS: [&str; 23] = [
    "node_modules",
    "vendor",
    "deps",
    "_build",
    "target",
    ".venv",
    "venv",
    "dist",
    "build",
    "test",
    "tests",
    "spec",
    "__tests__",
    "docs",
    "examples",
    "fixtures",
    "migrations",
    "__mocks__",
    "mocks",
    "mock",
    "testdata",
    "e2e",
    "generated",
];

/// Whether a path is application source worth reading.
#[must_use]
pub fn is_source_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if !EXTENSIONS.iter().any(|ext| lower.ends_with(ext)) {
        return false;
    }
    let file = lower.rsplit('/').next().unwrap_or(&lower);
    if file.starts_with("test_")
        || file.contains("_test.")
        || file.contains(".test.")
        || file.contains(".spec.")
        || file.contains(".stories.")
        || file.contains(".generated.")
        || file.contains(".pb.")
        || file.contains("_pb2")
        || file.contains("_mock.")
        || file.starts_with("mock_")
        || file.ends_with(".min.js")
        || file.ends_with(".d.ts")
    {
        return false;
    }
    !lower.split('/').any(|part| SKIPPED_DIRS.contains(&part))
}

/// Lower numbers are read first.
#[must_use]
pub fn priority(path: &str) -> u8 {
    let lower = path.to_ascii_lowercase();
    let hints = [
        "client",
        "consumer",
        "producer",
        "publisher",
        "subscriber",
        "queue",
        "event",
        "worker",
        "handler",
        "config",
        "settings",
        "runtime",
        "application",
        "app.",
        "main.",
        "service",
        "gateway",
        "adapter",
        "integration",
        "broker",
    ];
    u8::from(!hints.iter().any(|hint| lower.contains(hint)))
}

/// Pick the files to read from a list of paths, best first, within the cap.
#[must_use]
pub fn select_paths(paths: &[String]) -> Vec<String> {
    let mut chosen: Vec<&String> = paths.iter().filter(|path| is_source_path(path)).collect();
    chosen.sort_by_key(|path| (priority(path), path.len()));
    chosen.into_iter().take(MAX_SOURCE_FILES).cloned().collect()
}

/// Whether a file's text is small enough to read.
#[must_use]
pub fn is_readable(text: &str) -> bool {
    text.len() <= MAX_FILE_BYTES
}

static URL: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)\bhttps?://([a-z0-9][a-z0-9.\-]*[a-z0-9])(?::\d+)?"#).expect("url pattern")
});
static DATABASE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|rediss?)://(?:[^@\s"'/]*@)?([a-z0-9][a-z0-9.\-]*[a-z0-9])"#,
    )
    .expect("database pattern")
});
/// `mongodb://${db:carts-db}:27017`: the default after the colon is the host.
static DATABASE_DEFAULT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|rediss?)://(?:[^@\s"'/]*@)?\$\{[a-z0-9_.]+:([a-z0-9][a-z0-9.\-]*[a-z0-9])\}"#,
    )
    .expect("database default pattern")
});
/// A Go SQL data source name: `user:pass@tcp(catalogue-db:3306)/socks`.
static DSN_TCP: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?i)@tcp\(([a-z0-9][a-z0-9.\-]*[a-z0-9])(?::\d+)?\)"#).expect("dsn pattern")
});
/// A whole line `spring.rabbitmq.host=rabbitmq` or `redis.host: cache`.
static HOST_SETTING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)^\s*[a-z0-9_.\-]+\.host\s*[=:]\s*["']?([a-z0-9][a-z0-9.\-]*[a-z0-9])["']?\s*$"#,
    )
    .expect("host setting pattern")
});
static PARAMETER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)\b(queue[_ ]?url|queue[_ ]?name|queue|topic[_ ]?arn|topic[_ ]?name|topic|topics)\b["']?\s*(?:=>|=|:|\()\s*&?\[?\s*["']([^"'\s${}#]+)["']"#,
    )
    .expect("parameter pattern")
});
static KAFKA_CONSUME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?:KafkaConsumer|AIOKafkaConsumer|\.subscribe)\(\s*\[?\s*["']([^"'\s${}#]+)["']"#)
        .expect("consume pattern")
});
static KAFKA_PRODUCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)producer\w*\.(?:send|send_and_wait|produce|publish)\(\s*["']([^"'\s${}#]+)["']"#,
    )
    .expect("produce pattern")
});
static MARKER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)sqs|sns|kafka|rabbit|amqp|pika|nats|pubsub|pub/sub|broadway|lapin|rdkafka|brod\b|kinesis|eventbridge",
    )
    .expect("marker pattern")
});

/// Hosts that appear in code without being a dependency.
const IGNORED_HOST_SUFFIXES: [&str; 27] = [
    "localhost",
    "example.com",
    "example.org",
    "example.net",
    "w3.org",
    "schemas.xmlsoap.org",
    "json-schema.org",
    "github.com",
    "gitlab.com",
    "githubusercontent.com",
    "apache.org",
    "hex.pm",
    "hexdocs.pm",
    "pypi.org",
    "crates.io",
    "docs.rs",
    "npmjs.com",
    "python.org",
    "rust-lang.org",
    "elixir-lang.org",
    "golang.org",
    "wikipedia.org",
    // Documentation links that show up in messages and help text.
    "go.microsoft.com",
    "learn.microsoft.com",
    "docs.microsoft.com",
    "aka.ms",
    "grpc.io",
];
/// Names too generic to be one particular database.
const GENERIC_DATABASES: [&str; 9] = [
    "localhost",
    "db",
    "database",
    "postgres",
    "mysql",
    "mongo",
    "mongodb",
    "redis",
    "cache",
];

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Finding {
    Service(String),
    External(String),
    Database(String),
    Publishes(&'static str, String),
    Consumes(&'static str, String),
}

fn kebab(value: &str) -> Option<String> {
    let name: String = value
        .to_ascii_lowercase()
        .chars()
        .map(|c| if matches!(c, '_' | '.') { '-' } else { c })
        .collect();
    crate::ids::is_kebab_case(&name).then_some(name)
}

enum Host {
    Service(String),
    External(String),
    Database(String),
    Skip,
}

/// Hosts that stand for "put a real one here" or for the machine itself.
const PLACEHOLDER_HOSTS: [&str; 12] = [
    "host",
    "hostname",
    "host-name",
    "your-service",
    "your-host",
    "my-service",
    "my-host",
    "myhost",
    "service",
    "server",
    "host.docker.internal",
    "kubernetes.docker.internal",
];

/// A dead-letter queue is where failed messages land. Application code that
/// names one is almost always a redrive or a config value, not a publisher.
fn is_dead_letter(name: &str) -> bool {
    name.contains("dlq") || name.contains("dead-letter") || name.contains("deadletter")
}

/// Drop a trailing `# ...` or `// ...` comment. `://` is left alone.
fn strip_trailing_comment(line: &str) -> &str {
    let bytes = line.as_bytes();
    let mut cut = line.len();
    if let Some(at) = line
        .match_indices("//")
        .map(|(at, _)| at)
        .find(|&at| at == 0 || bytes[at - 1] != b':')
    {
        cut = at;
    }
    if let Some(at) = line.find(" #") {
        cut = cut.min(at);
    }
    &line[..cut]
}

/// Tracks block comments and docstrings across lines. Returns true when the
/// line is inside one (or opens one) and must not be read.
fn in_block(line: &str, open: &mut Option<&'static str>) -> bool {
    if let Some(closer) = *open {
        if line.contains(closer) {
            *open = None;
        }
        return true;
    }
    let trimmed = line.trim_start();
    if trimmed.starts_with("/*") && !trimmed[2..].contains("*/") {
        *open = Some("*/");
        return true;
    }
    for quote in ["\"\"\"", "'''"] {
        if line.matches(quote).count() % 2 == 1 {
            *open = Some(quote);
            return true;
        }
    }
    false
}

/// True for `%s`, `%v` or `%d` followed by the end of the host.
fn format_suffix_follows(rest: &str) -> bool {
    let mut chars = rest.chars();
    chars.next();
    matches!(chars.next(), Some('s' | 'v' | 'd'))
        && matches!(
            chars.next(),
            None | Some('/' | ':' | '"' | '\'' | '`' | ')' | ',')
        )
}

fn classify_host(host: &str) -> Host {
    let host = host.to_ascii_lowercase();
    if PLACEHOLDER_HOSTS.contains(&host.as_str()) {
        return Host::Skip;
    }
    if host.chars().all(|c| c.is_ascii_digit() || c == '.')
        || IGNORED_HOST_SUFFIXES
            .iter()
            .any(|suffix| host == *suffix || host.ends_with(&format!(".{suffix}")))
    {
        return Host::Skip;
    }
    let first = host.split('.').next().unwrap_or(&host);
    if host.ends_with(".rds.amazonaws.com") || host.ends_with(".cache.amazonaws.com") {
        return kebab(first).map_or(Host::Skip, Host::Database);
    }
    if host.ends_with("amazonaws.com") || host.ends_with("azure.com") {
        return Host::Skip;
    }
    if !host.contains('.')
        || host.ends_with(".svc")
        || host.contains(".svc.")
        || host.ends_with(".internal")
        || host.ends_with(".local")
    {
        return kebab(first).map_or(Host::Skip, Host::Service);
    }
    kebab(&host).map_or(Host::Skip, Host::External)
}

fn is_comment(line: &str) -> bool {
    let line = line.trim_start();
    line.starts_with('#')
        || line.starts_with("//")
        || line.starts_with("/*")
        || line.starts_with('*')
}

/// Words that say a queue name is being written to or read from.
fn direction(line: &str, window: &str) -> Option<bool> {
    const WRITE: [&str; 7] = [
        "send",
        "publish",
        "produce",
        "push",
        "enqueue",
        "emit",
        "put_record",
    ];
    const READ: [&str; 7] = [
        "receive",
        "consume",
        "subscribe",
        "poll",
        "listen",
        "pull",
        "broadway",
    ];
    let lower = line.to_ascii_lowercase();
    if lower.contains("broadway") {
        return Some(false);
    }
    let decide = |text: &str| {
        let write = WRITE.iter().any(|word| text.contains(word));
        let read = READ.iter().any(|word| text.contains(word));
        match (write, read) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            _ => None,
        }
    };
    decide(&lower).or_else(|| decide(&window.to_ascii_lowercase()))
}

/// The queue or topic name inside a literal such as a queue URL or an ARN.
fn messaging_name(key: &str, value: &str) -> Option<(&'static str, String)> {
    let mut kind = if key.to_ascii_lowercase().contains("topic") {
        "topic"
    } else {
        "queue"
    };
    let mut name = value;
    if value.starts_with("arn:aws:") {
        kind = if value.contains(":sns:") {
            "topic"
        } else {
            "queue"
        };
        name = value.rsplit(':').next()?;
    } else if value.starts_with("http") {
        name = value.rsplit('/').next()?;
    }
    let name = name.strip_suffix(".fifo").unwrap_or(name);
    Some((kind, kebab(name)?))
}

fn scan(text: &str, findings: &mut BTreeSet<Finding>) {
    let messaging = MARKER.is_match(text);
    let lines: Vec<&str> = text.lines().collect();
    let mut block = None;
    for (index, raw) in lines.iter().enumerate() {
        if in_block(raw, &mut block) || is_comment(raw) {
            continue;
        }
        let line = strip_trailing_comment(raw);
        for capture in HOST_SETTING.captures_iter(line) {
            match classify_host(&capture[1]) {
                Host::Service(name) => {
                    findings.insert(Finding::Service(name));
                }
                Host::Database(name) => {
                    findings.insert(Finding::Database(name));
                }
                Host::External(_) | Host::Skip => {}
            }
        }
        for capture in DATABASE
            .captures_iter(line)
            .chain(DATABASE_DEFAULT.captures_iter(line))
            .chain(DSN_TCP.captures_iter(line))
        {
            let host = capture[1].to_ascii_lowercase();
            let first = host.split('.').next().unwrap_or(&host);
            if GENERIC_DATABASES.contains(&first) {
                continue;
            }
            if let Some(name) = kebab(first) {
                findings.insert(Finding::Database(name));
            }
        }
        for capture in URL.captures_iter(line) {
            let whole = capture.get(0).expect("whole match");
            let next = line[whole.end()..].chars().next();
            if matches!(next, Some('-' | '_' | '{' | '$' | '#')) {
                continue;
            }
            // `http://catalogue%s/tags`: the host is whole, a format fills in
            // only a domain suffix. `http://%s` never gets here (no host).
            if next == Some('%') && !format_suffix_follows(&line[whole.end()..]) {
                continue;
            }
            match classify_host(&capture[1]) {
                Host::Service(name) => {
                    findings.insert(Finding::Service(name));
                }
                Host::External(name) => {
                    findings.insert(Finding::External(name));
                }
                Host::Database(name) => {
                    findings.insert(Finding::Database(name));
                }
                Host::Skip => {}
            }
        }
        if !messaging {
            continue;
        }
        let window = lines[index.saturating_sub(6)..=index].join("\n");
        for capture in PARAMETER.captures_iter(line) {
            let Some((kind, name)) = messaging_name(&capture[1], &capture[2]) else {
                continue;
            };
            match direction(line, &window) {
                Some(true) if is_dead_letter(&name) => false,
                Some(true) => findings.insert(Finding::Publishes(kind, name)),
                Some(false) => findings.insert(Finding::Consumes(kind, name)),
                None => false,
            };
        }
        for capture in KAFKA_CONSUME.captures_iter(line) {
            if let Some(name) = kebab(&capture[1]) {
                findings.insert(Finding::Consumes("topic", name));
            }
        }
        for capture in KAFKA_PRODUCE.captures_iter(line) {
            if let Some(name) = kebab(&capture[1]).filter(|name| !is_dead_letter(name)) {
                findings.insert(Finding::Publishes("topic", name));
            }
        }
    }
}

/// Records for one repository. `files` holds `(path, text)` of source files.
#[must_use]
pub fn source_records(project_path: &str, files: &[(String, String)]) -> Vec<RawRecord> {
    let repo = project_path.rsplit('/').next().unwrap_or(project_path);
    if !crate::ids::is_kebab_case(repo) {
        return Vec::new();
    }
    let mut findings = BTreeSet::new();
    for (path, text) in files {
        if is_source_path(path) && is_readable(text) {
            scan(text, &mut findings);
        }
    }
    let record = |kind: &str, payload: serde_json::Value| RawRecord {
        collector: "source_code".to_owned(),
        kind: kind.to_owned(),
        payload,
    };
    let mut records = Vec::new();
    for finding in findings {
        match finding {
            Finding::Service(name) if name != repo => records.push(record(
                "candidate_call",
                json!({"source": repo, "destination": name, "loose": true}),
            )),
            Finding::External(name) => records.push(record(
                "candidate_call",
                json!({"source": repo, "destination": name, "external": true}),
            )),
            Finding::Database(name) => records.push(record(
                "candidate_call",
                json!({"source": repo, "destination": name, "destination_kind": "database", "loose": true}),
            )),
            Finding::Publishes(kind, name) => records.push(record(
                "messaging",
                json!({"source": repo, "direction": "publish", "kind": kind, "name": name}),
            )),
            Finding::Consumes(kind, name) => records.push(record(
                "messaging",
                json!({"source": repo, "direction": "consume", "kind": kind, "name": name}),
            )),
            Finding::Service(_) => {}
        }
    }
    records
}

#[cfg(test)]
mod tests {
    use super::*;

    fn found(path: &str, text: &str) -> Vec<String> {
        source_records("team/orders-py", &[(path.into(), text.into())])
            .iter()
            .map(|r| {
                let p = &r.payload;
                if r.kind == "messaging" {
                    format!(
                        "{} {} {}",
                        p["direction"].as_str().unwrap(),
                        p["kind"].as_str().unwrap(),
                        p["name"].as_str().unwrap()
                    )
                } else {
                    format!(
                        "call {} {}",
                        p["destination"].as_str().unwrap(),
                        p["destination_kind"].as_str().unwrap_or(
                            if p["external"].as_bool() == Some(true) {
                                "external"
                            } else {
                                "service"
                            }
                        )
                    )
                }
            })
            .collect()
    }

    #[test]
    fn python_calls_services_and_databases_by_literal_name() {
        let text = r#"
import requests
r = requests.get("http://billing-ex:4000/charge")
s = requests.post("https://api.stripe.com/v1/charges")
d = connect("postgres://app:secret@orders-db.abc.eu-west-1.rds.amazonaws.com:5432/orders")
l = requests.get("http://localhost:8080/health")
# requests.get("http://commented-out/x")
"#;
        let got = found("src/client.py", text);
        assert!(got.contains(&"call billing-ex service".to_owned()));
        assert!(got.contains(&"call api-stripe-com external".to_owned()));
        assert!(got.contains(&"call orders-db database".to_owned()));
        assert_eq!(got.len(), 3, "{got:?}");
        assert!(!got.iter().any(|line| line.contains("secret")));
    }

    // Precision traps: code that mentions a service without calling it.
    // Each must produce nothing. They exist so a heuristic change that
    // starts guessing breaks a test instead of a graph.

    #[test]
    fn traps_do_not_hide_real_calls_next_to_them() {
        let text = r#"
app.use("/api/*", handler)
r = get("http://billing-ex:4000/charge")  # charge the card
s = get("http://ledger-ex:4000/post") // post it
"""
docstring mentioning http://old-svc/x
"""
t = get("http://invoice-ex:4000/new")
/* http://commented/x */
u = get("http://tax-ex:4000/rate")
"#;
        let got = found("src/a.js", text);
        for name in ["billing-ex", "ledger-ex", "invoice-ex", "tax-ex"] {
            assert!(
                got.contains(&format!("call {name} service")),
                "{name} missing: {got:?}"
            );
        }
        assert!(
            !got.iter()
                .any(|g| g.contains("old-svc") || g.contains("commented")),
            "{got:?}"
        );
    }

    #[test]
    fn trap_trailing_comments_are_not_calls() {
        let text =
            "r = get(url)  # was http://billing-ex:4000/charge\nx = 1 // http://billing-ex/old\n";
        assert!(
            found("src/a.py", text).is_empty(),
            "{:?}",
            found("src/a.py", text)
        );
    }

    #[test]
    fn trap_docstrings_and_block_comments_are_not_calls() {
        let text = "\"\"\"\nCalls http://billing-ex:4000/charge in production.\n\"\"\"\n/*\n http://billing-ex/old\n*/\n";
        assert!(
            found("src/a.py", text).is_empty(),
            "{:?}",
            found("src/a.py", text)
        );
    }

    #[test]
    fn trap_placeholder_and_local_hosts_are_not_calls() {
        let text = r#"
a = get("http://<host>/x")
b = get("http://your-service/x")
c = get("http://host.docker.internal:8080/x")
d = get("http://0.0.0.0:8080/x")
e = get("http://[::1]:8080/x")
f = get("http://127.0.0.1:8080/x")
g = get("http://10.0.0.5/x")
h = get("https://hostname/x")
"#;
        assert!(
            found("src/a.py", text).is_empty(),
            "{:?}",
            found("src/a.py", text)
        );
    }

    #[test]
    fn trap_generated_mock_and_test_support_files_are_skipped() {
        for path in [
            "src/api.pb.go",
            "src/api_pb2.py",
            "src/api_pb2_grpc.py",
            "src/client_mock.go",
            "src/mock_client.py",
            "src/__mocks__/client.js",
            "src/testdata/client.go",
            "e2e/client.ts",
            "src/Client.stories.ts",
            "src/client.generated.ts",
        ] {
            assert!(!is_source_path(path), "{path} should be skipped");
        }
    }

    #[test]
    fn trap_dead_letter_queues_are_not_published_to_by_application_code() {
        let text = r#"
sqs.send_message(QueueUrl="https://sqs.eu-west-1.amazonaws.com/123456789012/orders-dlq", MessageBody=b)
sqs.send_message(QueueUrl="https://sqs.eu-west-1.amazonaws.com/123456789012/orders-dead-letter", MessageBody=b)
"#;
        let got = found("src/a.py", text);
        assert!(got.is_empty(), "{got:?}");
    }

    #[test]
    fn trap_format_strings_and_templates_are_not_guessed() {
        let text = r#"
a = get("http://%s:8080/x" % host)
b = get(f"http://{svc}.internal/x")
c = get("http://" + name + "/x")
d = get("http://{{ .Values.host }}/x")
e = get("http://${HOST}:8080/x")
"#;
        assert!(
            found("src/a.py", text).is_empty(),
            "{:?}",
            found("src/a.py", text)
        );
    }

    #[test]
    fn documentation_links_in_messages_are_not_calls() {
        let text = r#"write("Learn more at https://go.microsoft.com/fwlink/?linkid=1 and https://grpc.io/docs")"#;
        assert!(found("a.cs", text).is_empty());
    }

    #[test]
    fn built_names_are_not_guessed() {
        let text = r#"requests.get(f"http://{host}/x"); get("http://billing-${ENV}.svc/x")"#;
        assert!(found("a.py", text).is_empty());
    }

    #[test]
    fn queues_and_topics_get_a_direction() {
        let python = r#"
sqs.send_message(QueueUrl="https://sqs.eu-west-1.amazonaws.com/123456789012/orders-created.fifo", MessageBody=b)
messages = sqs.receive_message(QueueUrl="https://sqs.eu-west-1.amazonaws.com/123/invoice-requests")
sns.publish(TopicArn="arn:aws:sns:eu-west-1:123456789012:order-events", Message=m)
producer.send("payments.captured", value)
consumer = KafkaConsumer("fraud-signals")
"#;
        let got = found("src/events.py", python);
        for expected in [
            "publish queue orders-created",
            "consume queue invoice-requests",
            "publish topic order-events",
            "publish topic payments-captured",
            "consume topic fraud-signals",
        ] {
            assert!(got.contains(&expected.to_owned()), "{expected} in {got:?}");
        }
    }

    #[test]
    fn elixir_broadway_consumes_and_ex_aws_publishes() {
        let text = r#"
producer: [module: {BroadwaySQS.Producer, queue_url: "https://sqs.eu-west-1.amazonaws.com/1/billing-jobs"}]
ExAws.SQS.send_message(queue: "receipts-out", body)
"#;
        let got = found("lib/billing/pipeline.ex", text);
        assert!(
            got.contains(&"consume queue billing-jobs".to_owned()),
            "{got:?}"
        );
        assert!(
            got.contains(&"publish queue receipts-out".to_owned()),
            "{got:?}"
        );
    }

    #[test]
    fn rust_aws_sdk_chains_are_read() {
        let text = r#"
client.send_message().queue_url("https://sqs.eu-west-1.amazonaws.com/1/edge-events").message_body(b).send().await?;
"#;
        assert!(found("src/main.rs", text).contains(&"publish queue edge-events".to_owned()));
    }

    #[test]
    fn a_queue_name_without_a_verb_is_left_alone() {
        let text = "// sqs\nQUEUE_NAME = \"orders-created\"\n";
        assert!(found("src/settings.py", text).is_empty());
    }

    #[test]
    fn tests_vendored_code_and_other_files_are_skipped() {
        for path in [
            "tests/test_client.py",
            "src/client_test.go",
            "vendor/x/client.go",
            "node_modules/a/index.js",
            "README.md",
            "src/app.test.ts",
        ] {
            assert!(!is_source_path(path), "{path}");
        }
        assert!(is_source_path("lib/billing/client.ex"));
    }

    #[test]
    fn integration_files_are_read_first_within_the_cap() {
        let mut paths: Vec<String> = (0..150).map(|i| format!("src/m{i}.py")).collect();
        paths.push("src/queue_client.py".into());
        let chosen = select_paths(&paths);
        assert_eq!(chosen.len(), MAX_SOURCE_FILES);
        assert_eq!(chosen[0], "src/queue_client.py");
    }

    #[test]
    fn a_service_calling_itself_is_not_a_dependency() {
        assert!(found("a.py", r#"get("http://orders-py:80/x")"#).is_empty());
    }

    #[test]
    fn a_host_followed_by_a_format_domain_is_still_a_host() {
        let text = r#"
catalogueUrl: util.format("http://catalogue%s", domain),
cartsUrl: util.format("http://carts%s/carts", domain),
"#;
        let mut got = found("src/endpoints.js", text);
        got.sort();
        assert_eq!(got, ["call carts service", "call catalogue service"]);
        // Still not guessed when the format sits inside the name.
        assert!(found("src/a.js", r#"get("http://shop%s.example.com/x")"#).is_empty());
    }

    #[test]
    fn database_defaults_dsns_and_host_settings_name_their_host() {
        let text = r#"
spring.data.mongodb.uri=mongodb://${db:carts-db}:27017/data
spring.rabbitmq.host=rabbitmq
dsn = "catalogue_user:default_password@tcp(catalogue-db:3306)/socksdb"
spring.redis.host=localhost
"#;
        let mut got = found("src/main/resources/application.properties", text);
        got.sort();
        assert_eq!(
            got,
            [
                "call carts-db database",
                "call catalogue-db database",
                "call rabbitmq service"
            ]
        );
    }
}
