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

#[cfg(test)]
mod tests {
    use super::*;

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
}
