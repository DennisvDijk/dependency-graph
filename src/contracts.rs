//! API and event contracts found in repositories.
//!
//! Three formats: OpenAPI (and Swagger 2), AsyncAPI, and Protocol Buffers.
//! A spec file in a repository means that repository's component provides
//! that contract. The scanner reads the file, names the contract, and lists
//! its operations so a later change can be compared. It never fetches a
//! schema from the network and never follows `$ref` out of the file.

use serde_json::{Value, json};

use crate::raw::RawRecord;

/// Most operations kept per contract. A larger spec is cut, not refused.
const MAX_OPERATIONS: usize = 500;

/// Contract files read per repository.
pub const MAX_PER_PROJECT: usize = 50;

/// Names too generic to say which API this is. The repository name is used.
const GENERIC_TITLES: [&str; 8] = [
    "api", "openapi", "swagger", "asyncapi", "service", "rest-api", "api-spec", "untitled",
];

/// The kind of contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// OpenAPI 3 or Swagger 2.
    Openapi,
    /// AsyncAPI 2 or 3.
    Asyncapi,
    /// Protocol Buffers.
    Proto,
}

impl Format {
    /// Wire value, also the suffix of the contract's name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Openapi => "openapi",
            Self::Asyncapi => "asyncapi",
            Self::Proto => "proto",
        }
    }
}

/// One contract read from one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    /// Format of the file.
    pub format: Format,
    /// Kebab-case name, ending in the format: `payments-api-openapi`.
    pub name: String,
    /// Title or package as written in the file.
    pub title: String,
    /// Version as written in the file, or empty.
    pub version: String,
    /// Operations: `GET /orders`, `Orders/Create`, or a channel name.
    pub operations: Vec<String>,
    /// Files this one imports (proto only).
    pub imports: Vec<String>,
}

/// Whether a path is worth reading as a contract.
#[must_use]
pub fn is_contract_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if ["node_modules/", "vendor/", "third_party/", "/testdata/"]
        .iter()
        .any(|skip| lower.contains(skip))
    {
        return false;
    }
    if lower.ends_with(".proto") {
        return true;
    }
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    let data = base.ends_with(".yaml") || base.ends_with(".yml") || base.ends_with(".json");
    data && ["openapi", "swagger", "asyncapi"]
        .iter()
        .any(|word| base.contains(word))
}

/// Lowercase kebab-case from any text. Empty when nothing usable is left.
#[must_use]
pub fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

fn name_for(title: &str, project_path: &str, format: Format) -> String {
    let mut base = slug(title);
    if base.is_empty() || GENERIC_TITLES.contains(&base.as_str()) {
        base = slug(project_path.rsplit('/').next().unwrap_or(project_path));
    }
    if base.is_empty() {
        base = "contract".to_owned();
    }
    format!("{base}-{}", format.as_str())
}

/// Read one file. `None` when it is not a contract of a known format.
#[must_use]
pub fn parse(path: &str, text: &str, project_path: &str) -> Option<Contract> {
    if path.to_ascii_lowercase().ends_with(".proto") {
        return Some(parse_proto(path, text, project_path));
    }
    let doc: Value = yaml_serde::from_str(text).ok()?;
    let object = doc.as_object()?;
    let format = if object.get("openapi").is_some() || object.get("swagger").is_some() {
        Format::Openapi
    } else if object.get("asyncapi").is_some() {
        Format::Asyncapi
    } else {
        return None;
    };
    let info = doc.get("info");
    let title = info
        .and_then(|info| info.get("title"))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned();
    let version = info
        .and_then(|info| info.get("version"))
        .map(|value| match value {
            Value::String(text) => text.clone(),
            other => other.to_string(),
        })
        .unwrap_or_default();
    let mut operations = match format {
        Format::Openapi => openapi_operations(&doc),
        _ => asyncapi_operations(&doc),
    };
    operations.sort();
    operations.dedup();
    operations.truncate(MAX_OPERATIONS);
    Some(Contract {
        format,
        name: name_for(&title, project_path, format),
        title,
        version,
        operations,
        imports: Vec::new(),
    })
}

fn openapi_operations(doc: &Value) -> Vec<String> {
    const METHODS: [&str; 8] = [
        "get", "put", "post", "delete", "patch", "head", "options", "trace",
    ];
    let mut out = Vec::new();
    if let Some(paths) = doc.get("paths").and_then(Value::as_object) {
        for (path, item) in paths {
            for method in METHODS {
                if item.get(method).is_some() {
                    out.push(format!("{} {path}", method.to_ascii_uppercase()));
                }
            }
        }
    }
    out
}

fn asyncapi_operations(doc: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(channels) = doc.get("channels").and_then(Value::as_object) {
        for (key, channel) in channels {
            // Version 3 names a channel by key and gives its address inside.
            let name = channel
                .get("address")
                .and_then(Value::as_str)
                .unwrap_or(key);
            out.push(name.to_owned());
        }
    }
    out
}

/// What an AsyncAPI document says its own service does with each channel:
/// `(publishes, channel)`.
///
/// Version 2 describes the service's API: `publish` is what others may
/// publish to it, so the service consumes; `subscribe` means it sends.
/// Version 3 says so directly with `action: send|receive`.
#[must_use]
pub fn asyncapi_ties(text: &str) -> Vec<(bool, String)> {
    let Ok(doc) = yaml_serde::from_str::<Value>(text) else {
        return Vec::new();
    };
    let mut out: Vec<(bool, String)> = Vec::new();
    let Some(channels) = doc.get("channels").and_then(Value::as_object) else {
        return out;
    };
    let name_of = |key: &str, channel: &Value| {
        channel
            .get("address")
            .and_then(Value::as_str)
            .unwrap_or(key)
            .to_owned()
    };
    if let Some(operations) = doc.get("operations").and_then(Value::as_object) {
        for operation in operations.values() {
            let publishes = match operation.get("action").and_then(Value::as_str) {
                Some("send") => true,
                Some("receive") => false,
                _ => continue,
            };
            let key = operation
                .get("channel")
                .and_then(|channel| channel.get("$ref"))
                .and_then(Value::as_str)
                .and_then(|reference| reference.rsplit('/').next());
            if let Some((key, channel)) = key.and_then(|key| Some((key, channels.get(key)?))) {
                out.push((publishes, name_of(key, channel)));
            }
        }
    }
    for (key, channel) in channels {
        if channel.get("publish").is_some() {
            out.push((false, name_of(key, channel)));
        }
        if channel.get("subscribe").is_some() {
            out.push((true, name_of(key, channel)));
        }
    }
    out.sort();
    out.dedup();
    out.truncate(MAX_OPERATIONS);
    out
}

fn parse_proto(path: &str, text: &str, project_path: &str) -> Contract {
    let mut package = String::new();
    let mut service = String::new();
    let mut operations = Vec::new();
    let mut imports = Vec::new();
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if let Some(rest) = line.strip_prefix("package ") {
            package = rest.trim_end_matches(';').trim().to_owned();
        } else if let Some(rest) = line.strip_prefix("import ") {
            let rest = rest
                .trim_start_matches("public ")
                .trim_start_matches("weak ")
                .trim_end_matches(';')
                .trim();
            let file = rest.trim_matches('"');
            if !file.is_empty() {
                imports.push(file.to_owned());
            }
        } else if let Some(rest) = line.strip_prefix("service ") {
            service = rest
                .split(|c: char| c == '{' || c.is_whitespace())
                .next()
                .unwrap_or("")
                .to_owned();
        } else if let Some(rest) = line.strip_prefix("rpc ") {
            let method = rest.split('(').next().unwrap_or("").trim();
            if !method.is_empty() {
                operations.push(if service.is_empty() {
                    method.to_owned()
                } else {
                    format!("{service}/{method}")
                });
            }
        }
    }
    operations.sort();
    operations.dedup();
    operations.truncate(MAX_OPERATIONS);
    let stem = path
        .rsplit('/')
        .next()
        .unwrap_or(path)
        .trim_end_matches(".proto");
    let title = if package.is_empty() {
        stem.to_owned()
    } else {
        package.clone()
    };
    let version = package
        .rsplit('.')
        .next()
        .filter(|last| {
            last.strip_prefix('v')
                .is_some_and(|rest| !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()))
        })
        .unwrap_or_default()
        .to_owned();
    Contract {
        format: Format::Proto,
        name: name_for(&title, project_path, Format::Proto),
        title,
        version,
        operations,
        imports,
    }
}

/// Raw records for one file: a `contract`, plus a `contract_use` for each
/// proto import. Empty when the file is not a contract.
#[must_use]
pub fn records(project_path: &str, path: &str, text: &str) -> Vec<RawRecord> {
    let Some(contract) = parse(path, text, project_path) else {
        return Vec::new();
    };
    let mut records = vec![RawRecord {
        collector: "contracts".to_owned(),
        kind: "contract".to_owned(),
        payload: json!({
            "project_path": project_path,
            "path": path,
            "format": contract.format.as_str(),
            "name": contract.name,
            "title": contract.title,
            "version": contract.version,
            "operations": contract.operations,
        }),
    }];
    // Who sends or receives each channel: a messaging tie to a topic, the same
    // record the source reader writes. Only AsyncAPI says it.
    if contract.format == Format::Asyncapi {
        let repo = project_path.rsplit('/').next().unwrap_or(project_path);
        for (publishes, channel) in asyncapi_ties(text) {
            let name = slug(&channel);
            if name.is_empty() {
                continue;
            }
            records.push(RawRecord {
                collector: "contracts".to_owned(),
                kind: "messaging".to_owned(),
                payload: json!({
                    "source": repo,
                    "direction": if publishes { "publish" } else { "consume" },
                    "kind": "topic",
                    "name": name,
                }),
            });
        }
    }
    for import in &contract.imports {
        // Well-known imports (google/protobuf/...) are not ours.
        if import.starts_with("google/") {
            continue;
        }
        records.push(RawRecord {
            collector: "contracts".to_owned(),
            kind: "contract_use".to_owned(),
            payload: json!({
                "project_path": project_path,
                "path": path,
                "format": "proto",
                "import": import,
            }),
        });
    }
    records
}

/// What changed in one contract between two versions of its file.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct ContractDiff {
    /// Path of the file.
    pub path: String,
    /// Format, when either side could be read.
    pub format: String,
    /// Operations in the old version and not in the new one. Callers can break.
    pub removed: Vec<String>,
    /// Operations only in the new version. Safe for existing callers.
    pub added: Vec<String>,
    /// Old version string, or empty.
    pub old_version: String,
    /// New version string, or empty.
    pub new_version: String,
    /// The file was deleted.
    pub deleted: bool,
    /// An operation list hit the size cut, so removals may be wrong. Nothing is claimed.
    pub truncated: bool,
}

impl ContractDiff {
    /// Removing an operation, or the whole file, can break a caller.
    #[must_use]
    pub fn is_breaking(&self) -> bool {
        !self.truncated && (self.deleted || !self.removed.is_empty())
    }
}

/// Compare two versions of one file. `None` for old means the file is new;
/// `None` for new means it was deleted. `None` result: neither side is a
/// contract, or nothing changed at operation level.
///
/// Only operations are compared (paths and methods, rpc names, channels).
/// Changed fields, types or required parameters are not seen.
#[must_use]
pub fn diff(path: &str, old: Option<&str>, new: Option<&str>) -> Option<ContractDiff> {
    let old = old.and_then(|text| parse(path, text, ""));
    let new = new.and_then(|text| parse(path, text, ""));
    if old.is_none() && new.is_none() {
        return None;
    }
    let ops = |c: &Option<Contract>| c.as_ref().map(|c| c.operations.clone()).unwrap_or_default();
    let (old_ops, new_ops) = (ops(&old), ops(&new));
    let removed: Vec<String> = old_ops
        .iter()
        .filter(|op| !new_ops.contains(op))
        .cloned()
        .collect();
    let added: Vec<String> = new_ops
        .iter()
        .filter(|op| !old_ops.contains(op))
        .cloned()
        .collect();
    let truncated = old_ops.len() >= MAX_OPERATIONS || new_ops.len() >= MAX_OPERATIONS;
    let deleted = new.is_none();
    if removed.is_empty() && added.is_empty() && !deleted {
        return None;
    }
    let format = new
        .as_ref()
        .or(old.as_ref())
        .map(|c| c.format.as_str().to_owned())
        .unwrap_or_default();
    let version = |c: &Option<Contract>| c.as_ref().map(|c| c.version.clone()).unwrap_or_default();
    Some(ContractDiff {
        path: path.to_owned(),
        format,
        removed,
        added,
        old_version: version(&old),
        new_version: version(&new),
        deleted,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn removed_operations_are_breaking_and_added_ones_are_not() {
        let old = "openapi: 3.0.0\ninfo: {title: Pay API, version: '1'}\npaths:\n  /a: {get: {}}\n  /b: {post: {}}\n";
        let new = "openapi: 3.0.0\ninfo: {title: Pay API, version: '2'}\npaths:\n  /a: {get: {}}\n  /c: {get: {}}\n";
        let d = diff("api/openapi.yaml", Some(old), Some(new)).unwrap();
        assert_eq!(d.removed, vec!["POST /b"]);
        assert_eq!(d.added, vec!["GET /c"]);
        assert!(d.is_breaking());
        assert_eq!((d.old_version.as_str(), d.new_version.as_str()), ("1", "2"));
        let additive = diff(
            "api/openapi.yaml",
            Some(old),
            Some(&format!("{old}  /z: {{get: {{}}}}\n")),
        )
        .unwrap();
        assert!(!additive.is_breaking());
        assert_eq!(additive.added, vec!["GET /z"]);
        assert!(diff("api/openapi.yaml", Some(old), Some(old)).is_none());
    }

    #[test]
    fn a_deleted_or_new_file_and_a_non_contract_are_handled() {
        let old =
            "syntax = \"proto3\";\npackage p.v1;\nservice S {\n  rpc Get (A) returns (B);\n}\n";
        let gone = diff("p.proto", Some(old), None).unwrap();
        assert!(gone.deleted && gone.is_breaking());
        let fresh = diff("p.proto", None, Some(old)).unwrap();
        assert!(!fresh.is_breaking());
        assert_eq!(fresh.added.len(), 1);
        assert!(diff("x/openapi.yaml", Some("hello: 1"), Some("hello: 2")).is_none());
    }

    const OPENAPI: &str = "openapi: 3.0.3\ninfo:\n  title: Payments API\n  version: 2.1.0\npaths:\n  /payments:\n    post: {}\n    get: {}\n  /payments/{id}:\n    get: {}\n";

    #[test]
    fn an_openapi_file_is_named_after_its_title_and_lists_operations() {
        let contract = parse("api/openapi.yaml", OPENAPI, "org/payments").expect("contract");
        assert_eq!(contract.format, Format::Openapi);
        assert_eq!(contract.name, "payments-api-openapi");
        assert_eq!(contract.version, "2.1.0");
        assert_eq!(
            contract.operations,
            vec!["GET /payments", "GET /payments/{id}", "POST /payments"]
        );
    }

    #[test]
    fn a_generic_title_falls_back_to_the_repository_name() {
        let text = "openapi: 3.0.0\ninfo:\n  title: API\n  version: '1'\npaths: {}\n";
        let contract = parse("openapi.yml", text, "org/ledger-service").expect("contract");
        assert_eq!(contract.name, "ledger-service-openapi");
    }

    #[test]
    fn asyncapi_channels_are_listed_for_both_major_versions() {
        let v2 = "asyncapi: 2.6.0\ninfo:\n  title: Order Events\n  version: 1.0.0\nchannels:\n  order.created: {}\n  order.paid: {}\n";
        let contract = parse("asyncapi.yaml", v2, "org/orders").expect("v2");
        assert_eq!(contract.format, Format::Asyncapi);
        assert_eq!(contract.name, "order-events-asyncapi");
        assert_eq!(contract.operations, vec!["order.created", "order.paid"]);
        let v3 = "asyncapi: 3.0.0\ninfo:\n  title: Order Events\n  version: 1.0.0\nchannels:\n  created:\n    address: orders/created\n";
        let contract = parse("asyncapi.yaml", v3, "org/orders").expect("v3");
        assert_eq!(contract.operations, vec!["orders/created"]);
    }

    #[test]
    fn a_proto_file_gives_package_services_and_imports() {
        let text = "syntax = \"proto3\";\npackage payments.v1;\nimport \"orders/v1/orders.proto\";\nimport \"google/protobuf/timestamp.proto\";\nservice Payments {\n  rpc Charge(ChargeRequest) returns (ChargeReply); // pay\n  rpc Refund(RefundRequest) returns (RefundReply);\n}\n";
        let contract =
            parse("proto/payments/v1/payments.proto", text, "org/payments").expect("contract");
        assert_eq!(contract.name, "payments-v1-proto");
        assert_eq!(contract.version, "v1");
        assert_eq!(
            contract.operations,
            vec!["Payments/Charge", "Payments/Refund"]
        );
        assert_eq!(contract.imports.len(), 2);
        let records = records("org/payments", "proto/payments/v1/payments.proto", text);
        let uses: Vec<_> = records
            .iter()
            .filter(|r| r.kind == "contract_use")
            .collect();
        assert_eq!(uses.len(), 1, "google/ imports are skipped");
        assert_eq!(uses[0].payload["import"], "orders/v1/orders.proto");
    }

    #[test]
    fn files_that_are_not_contracts_are_ignored() {
        assert!(parse("openapi.yaml", "name: not a spec\n", "org/x").is_none());
        assert!(parse("openapi.json", "{ not json", "org/x").is_none());
        assert!(!is_contract_path("vendor/foo/openapi.yaml"));
        assert!(!is_contract_path("deploy/deployment.yaml"));
        assert!(is_contract_path("docs/Swagger.json"));
        assert!(is_contract_path("a/b/c.proto"));
    }

    #[test]
    fn json_specs_parse_like_yaml() {
        let text = r#"{"openapi":"3.0.0","info":{"title":"Inventory","version":"3"},"paths":{"/items":{"get":{}}}}"#;
        let contract = parse("openapi.json", text, "org/inventory").expect("contract");
        assert_eq!(contract.name, "inventory-openapi");
        assert_eq!(contract.version, "3");
    }

    #[test]
    fn asyncapi_directions_follow_the_version() {
        let v2 = "asyncapi: 2.6.0\ninfo: {title: T, version: '1'}\nchannels:\n  orders/created:\n    publish: {}\n  invoice.paid:\n    subscribe: {}\n";
        assert_eq!(
            asyncapi_ties(v2),
            vec![
                (false, "orders/created".into()),
                (true, "invoice.paid".into())
            ]
        );
        let v3 = "asyncapi: 3.0.0\ninfo: {title: T, version: '1'}\nchannels:\n  paid:\n    address: invoice.paid\noperations:\n  onPaid:\n    action: receive\n    channel: {$ref: '#/channels/paid'}\n";
        assert_eq!(asyncapi_ties(v3), vec![(false, "invoice.paid".into())]);
        let recs = records("g/billing-ex", "asyncapi.yaml", v2);
        let msgs: Vec<_> = recs.iter().filter(|r| r.kind == "messaging").collect();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[1].payload["name"], "invoice-paid");
        assert_eq!(msgs[1].payload["direction"], "publish");
    }
}
