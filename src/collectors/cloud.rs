//! Shared pieces of the opt-in live cloud readers (AWS and Azure).
//!
//! Both readers call the provider's own CLI (`aws`, `az`) with read-only
//! list and describe commands, using whatever login that CLI already has.
//! depgraph never sees or stores a key. Off by default: `aws.enabled` and
//! `azure.enabled` in `config/config.yaml`, or `--aws` / `--azure` on one run.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

use crate::error::Error;
use crate::raw::RawRecord;

/// Where a cloud observation came from, written into `evidence_ref`.
#[derive(Debug, Clone, Copy)]
pub(super) enum Evidence {
    Xray,
    AppInsights,
}

impl Evidence {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Xray => "aws-xray",
            Self::AppInsights => "azure-app-insights",
        }
    }
}

/// The destination of a call seen in a cloud service map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Target {
    /// Another service; matched loosely against known components.
    Service(String),
    /// A database, queue or topic: a resource node of that kind.
    Resource(&'static str, String),
    /// A host outside the estate.
    External(String),
}

/// One call seen by X-Ray or Application Insights.
pub(super) struct Seen<'a> {
    pub source: &'a str,
    pub target: Target,
    pub calls: u64,
    pub errors: u64,
    pub env: Option<&'a str>,
    pub window: &'a str,
    pub evidence: Evidence,
}

/// An `observed_call` record. Names are matched loosely in the merge.
pub(super) fn observed(seen: &Seen<'_>) -> Option<RawRecord> {
    let source = kebab_name(seen.source);
    let (destination, kind, external, host) = match &seen.target {
        Target::Service(name) => (kebab_name(name), None, false, String::new()),
        Target::Resource(kind, name) => (kebab_name(name), Some(*kind), false, String::new()),
        Target::External(host) => {
            let name = kebab_name(host);
            (name.clone(), None, true, name)
        }
    };
    if source.is_empty() || destination.is_empty() || (kind.is_none() && source == destination) {
        return None;
    }
    Some(RawRecord {
        collector: "runtime_edges".to_owned(),
        kind: "observed_call".to_owned(),
        payload: json!({
            "source": source,
            "destination": destination,
            "destination_kind": kind,
            "external": external,
            "external_host": host,
            "calls": seen.calls,
            "errors": seen.errors.min(seen.calls),
            "window": seen.window,
            "env": seen.env.unwrap_or(""),
            "loose": true,
            "evidence": seen.evidence.as_str(),
        }),
    })
}

/// Lower-case kebab name: `Orders_Py.Prod` -> `orders-py-prod`.
pub(super) fn kebab_name(text: &str) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_matches('-').to_owned()
}

/// A JSON number, or a number in a string (Log Analytics returns strings).
pub(super) fn count(value: Option<&Value>) -> u64 {
    match value {
        Some(Value::Number(number)) => number
            .as_u64()
            .or_else(|| number.as_f64().map(|float| float.max(0.0) as u64))
            .unwrap_or(0),
        Some(Value::String(text)) => text.trim().parse::<f64>().map_or(0, |n| n.max(0.0) as u64),
        _ => 0,
    }
}

/// Run a read-only CLI command and parse its JSON output.
///
/// Errors name the command, never its output, so no account detail or token
/// ends up in logs.
pub(super) fn run_json(program: &str, args: &[String]) -> Result<Value, Error> {
    let shown = format!(
        "{program} {}",
        args.iter()
            .take_while(|arg| !arg.starts_with("--"))
            .cloned()
            .collect::<Vec<_>>()
            .join(" ")
    );
    let output = Command::new(program).args(args).output().map_err(|_| {
        Error::Config(format!(
            "could not run `{program}`. Install it and sign in first (docs/tutorials/09-cloud-discovery.md)."
        ))
    })?;
    if !output.status.success() {
        return Err(Error::Config(format!(
            "`{shown}` failed. Check that you are signed in and may read this; the output was not logged."
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|_| Error::Config(format!("`{shown}` did not return JSON")))
}

/// Read `dir/name` as JSON, if it exists.
///
/// # Errors
///
/// Returns an error when the file exists but is not JSON.
pub(super) fn read_json(dir: &Path, name: &str) -> Result<Option<Value>, Error> {
    let path = dir.join(name);
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|err| Error::Config(format!("{}: {err}", path.display())))
}

/// String array at `value[key]`, or the `key` field of each object in it.
pub(super) fn strings(value: Option<&Value>, field: Option<&str>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| match field {
            Some(field) => item.get(field).and_then(Value::as_str),
            None => item.as_str(),
        })
        .map(str::to_owned)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_become_kebab_case() {
        assert_eq!(kebab_name("Orders_Py.Prod"), "orders-py-prod");
        assert_eq!(kebab_name("api.stripe.com"), "api-stripe-com");
        assert_eq!(kebab_name("--"), "");
    }

    #[test]
    fn counts_accept_strings() {
        assert_eq!(count(Some(&json!("12"))), 12);
        assert_eq!(count(Some(&json!(3))), 3);
        assert_eq!(count(None), 0);
    }

    #[test]
    fn self_calls_are_dropped() {
        let seen = Seen {
            source: "orders",
            target: Target::Service("Orders".to_owned()),
            calls: 1,
            errors: 0,
            env: None,
            window: "6h",
            evidence: Evidence::Xray,
        };
        assert!(observed(&seen).is_none());
    }
}
