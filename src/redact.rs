//! Redaction for logs, errors, and graph fields.
//!
//! Credentialed URLs and known token shapes are never written to a snapshot,
//! to `meta.json`, or to stderr.

use std::sync::LazyLock;

use regex::Regex;
use serde_json::Value;

struct Pattern {
    regex: Regex,
    kind: &'static str,
}

static PATTERNS: LazyLock<Vec<Pattern>> = LazyLock::new(|| {
    vec![
        pattern(
            r"(?i)[a-z][a-z0-9+.-]*://[^\s/@]+@",
            "a URL with credentials",
        ),
        pattern(
            r"(?i)[?&](?:access[_-]?token|api[_-]?key|password|passwd|secret|token)=[^&#\s]+",
            "a URL with credentials",
        ),
        pattern(
            r"(?i)(?:api[_-]?key|secret|password|passwd|access[_-]?token|auth[_-]?token)\s*[=:]\s*\S+",
            "a known secret pattern",
        ),
        pattern(r"glpat-[A-Za-z0-9_\-]{16,}", "a known secret pattern"),
        pattern(r"\bghp_[A-Za-z0-9]{20,}\b", "a known secret pattern"),
        pattern(r"github_pat_[A-Za-z0-9_]{20,}", "a known secret pattern"),
        pattern(r"\bAKIA[0-9A-Z]{16}\b", "a known secret pattern"),
        pattern(r"\bxox[baprs]-[A-Za-z0-9-]{10,}", "a known secret pattern"),
        pattern(
            r"-----BEGIN [A-Z ]*PRIVATE KEY-----",
            "a known secret pattern",
        ),
        pattern(
            r"(?i)\bbearer\s+[A-Za-z0-9\-._~+/]{16,}={0,2}",
            "a known secret pattern",
        ),
        pattern(
            r"(?i)client-(?:key|certificate)-data:\s*\S+",
            "a known secret pattern",
        ),
    ]
});

fn pattern(expression: &str, kind: &'static str) -> Pattern {
    Pattern {
        regex: Regex::new(expression).expect("secret pattern compiles"),
        kind,
    }
}

/// Replace secret-shaped substrings with `***`.
#[must_use]
pub fn redact(input: &str) -> String {
    let mut output = input.to_owned();
    for pattern in PATTERNS.iter() {
        output = pattern.regex.replace_all(&output, "***").into_owned();
    }
    output
}

/// Classify the first secret pattern in `input`.
#[must_use]
pub fn secret_kind(input: &str) -> Option<&'static str> {
    PATTERNS
        .iter()
        .find(|pattern| pattern.regex.is_match(input))
        .map(|pattern| pattern.kind)
}

/// A secret found while walking JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretHit {
    /// JSON path of the offending string. Secret text in a key is redacted.
    pub path: String,
    /// What was found, without the secret itself.
    pub kind: &'static str,
}

/// Walk `value` and report every secret-shaped string, including object keys.
#[must_use]
pub fn find_secrets(value: &Value) -> Vec<SecretHit> {
    let mut hits = Vec::new();
    walk(value, "$", &mut hits);
    hits
}

fn walk(value: &Value, path: &str, hits: &mut Vec<SecretHit>) {
    match value {
        Value::String(text) => {
            if let Some(kind) = secret_kind(text) {
                hits.push(SecretHit {
                    path: redact(path),
                    kind,
                });
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                walk(item, &format!("{path}[{index}]"), hits);
            }
        }
        Value::Object(map) => {
            for (key, child) in map {
                let child_path = format!("{path}.{key}");
                if let Some(kind) = secret_kind(key) {
                    hits.push(SecretHit {
                        path: redact(&child_path),
                        kind,
                    });
                }
                walk(child, &child_path, hits);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_credentialed_urls_and_tokens() {
        let url = "https://user:password@gitlab.example.com/api/v4";
        let redacted = redact(url);
        assert!(!redacted.contains("password"));
        assert_eq!(secret_kind(url), Some("a URL with credentials"));
        assert!(secret_kind("https://gitlab.example.com").is_none());

        let token = "glpat-ABCDEFGHIJKLMNOPQRST";
        assert_eq!(secret_kind(token), Some("a known secret pattern"));
        assert!(!redact(token).contains("ABCDEFG"));
        assert_eq!(
            secret_kind("AKIAEXAMPLEKEY123456"),
            Some("a known secret pattern")
        );
        assert!(secret_kind("app.kubernetes.io/name").is_none());
    }

    #[test]
    fn walks_nested_json_without_echoing_the_secret() {
        let value = json!({"attrs": {"url": "https://user:s3cret@example.com"}});
        let hits = find_secrets(&value);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "$.attrs.url");
        assert!(!format!("{hits:?}").contains("s3cret"));
    }
}
