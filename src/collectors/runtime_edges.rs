//! Istio runtime edges from Prometheus.
//!
//! Queries are read-only `GET /api/v1/query` calls. Fixtures are recorded
//! responses of those calls. The sample timestamp is `last_seen`; this module
//! does not read the clock. ADR-008.

use std::collections::BTreeMap;
use std::path::Path;

use serde::Deserialize;
use serde_json::{Value, json};

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;
use crate::redact;

/// Paths and workloads dropped before an edge is emitted.
#[derive(Debug, Clone, Default)]
pub struct RuntimeFilter {
    /// Values of `request_path` or `path` that are health checks.
    pub health_paths: Vec<String>,
    /// `source_workload` values that are known test clients.
    pub test_clients: Vec<String>,
}

/// Runtime-edge collector entry used by the trait list.
///
/// The live query is [`query_prometheus`]. This method stays a stub so the
/// trait cannot open a socket by itself.
#[derive(Debug, Default)]
pub struct RuntimeEdges;

impl Collector for RuntimeEdges {
    fn name(&self) -> &'static str {
        "runtime_edges"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector runtime_edges"))
    }
}

/// PromQL range. Retention shorter than the configured window uses one day.
#[must_use]
pub fn query_window(window_days: u32, retention_days: Option<u32>) -> &'static str {
    match retention_days {
        Some(days) if days < window_days => "1d",
        _ => "7d",
    }
}

/// Total requests, grouped by source and destination workload and namespace.
#[must_use]
pub fn calls_query(window: &str) -> String {
    format!(
        "sum by (source_workload, source_workload_namespace, destination_workload, destination_workload_namespace, destination_service, request_protocol) (increase(istio_requests_total{{reporter=\"destination\"}}[{window}]))"
    )
}

/// 5xx requests, same grouping as [`calls_query`].
#[must_use]
pub fn errors_query(window: &str) -> String {
    format!(
        "sum by (source_workload, source_workload_namespace, destination_workload, destination_workload_namespace, destination_service, request_protocol) (increase(istio_requests_total{{reporter=\"destination\",response_code=~\"5..\"}}[{window}]))"
    )
}

/// 95th percentile of `istio_request_duration_milliseconds_bucket`.
#[must_use]
pub fn latency_query(window: &str) -> String {
    format!(
        "histogram_quantile(0.95, sum by (le, source_workload, source_workload_namespace, destination_workload, destination_workload_namespace, request_protocol) (rate(istio_request_duration_milliseconds_bucket{{reporter=\"destination\"}}[{window}])))"
    )
}

/// Read `calls.json`, `errors.json`, and `latency.json` from `dir`.
///
/// # Errors
///
/// Returns an error when a file is missing or is not a Prometheus vector.
pub fn records_from_prometheus_dir(
    dir: &Path,
    window: &str,
    filter: &RuntimeFilter,
) -> Result<Vec<RawRecord>, Error> {
    let calls = std::fs::read_to_string(dir.join("calls.json"))
        .map_err(|err| Error::Config(format!("reading calls.json: {err}")))?;
    let errors = std::fs::read_to_string(dir.join("errors.json"))
        .map_err(|err| Error::Config(format!("reading errors.json: {err}")))?;
    let latency = std::fs::read_to_string(dir.join("latency.json"))
        .map_err(|err| Error::Config(format!("reading latency.json: {err}")))?;
    records_from_prometheus_bodies(&calls, &errors, &latency, window, filter)
}

/// Turn three Prometheus instant-query bodies into observed-call records.
///
/// # Errors
///
/// Returns an error when a body is not a successful vector.
pub fn records_from_prometheus_bodies(
    calls: &str,
    errors: &str,
    latency: &str,
    window: &str,
    filter: &RuntimeFilter,
) -> Result<Vec<RawRecord>, Error> {
    let mut totals: BTreeMap<CallKey, Agg> = BTreeMap::new();
    absorb(
        &mut totals,
        &parse_vector(calls)?,
        filter,
        SampleKind::Calls,
    )?;
    absorb(
        &mut totals,
        &parse_vector(errors)?,
        filter,
        SampleKind::Errors,
    )?;
    absorb(
        &mut totals,
        &parse_vector(latency)?,
        filter,
        SampleKind::Latency,
    )?;
    let mut records = Vec::new();
    for (key, agg) in totals {
        if agg.calls == 0 && agg.errors == 0 && agg.p95_ms.is_none() {
            continue;
        }
        let calls = agg.calls.max(agg.errors);
        records.push(RawRecord {
            collector: "runtime_edges".to_owned(),
            kind: "observed_call".to_owned(),
            payload: json!({
                "source": key.source,
                "destination": key.destination,
                "source_namespace": key.source_namespace,
                "destination_namespace": key.destination_namespace,
                "protocol": key.protocol,
                "calls": calls,
                "errors": agg.errors,
                "p95_ms": agg.p95_ms,
                "window": window,
                "last_seen": agg.last_seen,
                "external": key.external,
                "external_host": key.external_host,
            }),
        });
    }
    records.sort_by_key(|record| record.payload.to_string());
    Ok(records)
}

/// Query Prometheus with three GETs and parse the bodies.
///
/// `token` is sent as a bearer token when present. It is not written into
/// errors. The URL must not contain userinfo.
///
/// # Errors
///
/// Returns an error on HTTP failure or a Prometheus error status.
pub async fn query_prometheus(
    base_url: &str,
    window: &str,
    filter: &RuntimeFilter,
    token: Option<&str>,
) -> Result<Vec<RawRecord>, Error> {
    if base_url.contains('@') {
        return Err(Error::Config(
            "prometheus url must not contain credentials".to_owned(),
        ));
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|err| Error::Config(redact::redact(&err.to_string())))?;
    let calls = get_query(&client, base_url, &calls_query(window), token).await?;
    let errors = get_query(&client, base_url, &errors_query(window), token).await?;
    let latency = get_query(&client, base_url, &latency_query(window), token).await?;
    records_from_prometheus_bodies(&calls, &errors, &latency, window, filter)
}

async fn get_query(
    client: &reqwest::Client,
    base_url: &str,
    query: &str,
    token: Option<&str>,
) -> Result<String, Error> {
    let url = format!(
        "{}/api/v1/query?query={}",
        base_url.trim_end_matches('/'),
        encode_query(query)
    );
    let mut request = client.get(&url);
    if let Some(token) = token {
        request = request.bearer_auth(token);
    }
    let response = request
        .send()
        .await
        .map_err(|err| Error::Config(redact::redact(&format!("prometheus query failed: {err}"))))?;
    if !response.status().is_success() {
        return Err(Error::Config(format!(
            "prometheus query returned {}",
            response.status()
        )));
    }
    response
        .text()
        .await
        .map_err(|err| Error::Config(redact::redact(&format!("prometheus body failed: {err}"))))
}

fn encode_query(query: &str) -> String {
    let mut out = String::new();
    for byte in query.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct CallKey {
    source: String,
    destination: String,
    source_namespace: String,
    destination_namespace: String,
    protocol: String,
    external: bool,
    external_host: String,
}

struct Agg {
    calls: u64,
    errors: u64,
    p95_ms: Option<f64>,
    last_seen: String,
}

enum SampleKind {
    Calls,
    Errors,
    Latency,
}

#[derive(Debug, Deserialize)]
struct PromResponse {
    status: String,
    data: Option<PromData>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PromData {
    #[serde(default)]
    result: Vec<PromSample>,
}

#[derive(Debug, Deserialize)]
struct PromSample {
    #[serde(default)]
    metric: BTreeMap<String, String>,
    value: Option<Vec<Value>>,
}

fn parse_vector(body: &str) -> Result<Vec<PromSample>, Error> {
    let parsed: PromResponse = serde_json::from_str(body)
        .map_err(|err| Error::Config(format!("prometheus response was not JSON: {err}")))?;
    if parsed.status != "success" {
        let message = parsed.error.unwrap_or_else(|| "query failed".to_owned());
        return Err(Error::Config(format!(
            "prometheus query failed: {}",
            redact::redact(&message)
        )));
    }
    Ok(parsed.data.map(|data| data.result).unwrap_or_default())
}

fn absorb(
    totals: &mut BTreeMap<CallKey, Agg>,
    samples: &[PromSample],
    filter: &RuntimeFilter,
    kind: SampleKind,
) -> Result<(), Error> {
    for sample in samples {
        if !keep(&sample.metric, filter) {
            continue;
        }
        let Some(key) = call_key(&sample.metric) else {
            continue;
        };
        let Some((number, seen)) = sample_number(sample)? else {
            continue;
        };
        let entry = totals.entry(key).or_insert_with(|| Agg {
            calls: 0,
            errors: 0,
            p95_ms: None,
            last_seen: seen.clone(),
        });
        if seen > entry.last_seen {
            entry.last_seen = seen;
        }
        match kind {
            SampleKind::Calls => entry.calls += number.round() as u64,
            SampleKind::Errors => entry.errors += number.round() as u64,
            SampleKind::Latency => {
                if number.is_finite() {
                    entry.p95_ms = Some(number);
                }
            }
        }
    }
    Ok(())
}

fn keep(metric: &BTreeMap<String, String>, filter: &RuntimeFilter) -> bool {
    if let Some(path) = metric.get("request_path").or_else(|| metric.get("path"))
        && filter.health_paths.iter().any(|item| item == path)
    {
        return false;
    }
    if let Some(source) = metric.get("source_workload")
        && filter.test_clients.iter().any(|item| item == source)
    {
        return false;
    }
    true
}

fn call_key(metric: &BTreeMap<String, String>) -> Option<CallKey> {
    let source = metric.get("source_workload")?.trim();
    if source.is_empty() || !crate::ids::is_kebab_case(source) {
        return None;
    }
    let external = is_external(metric);
    let service = metric
        .get("destination_service")
        .map(String::as_str)
        .unwrap_or("");
    let (destination, external_host) = if external {
        let host = external_name(service);
        (host.clone(), host)
    } else {
        let workload = metric
            .get("destination_workload")
            .map(String::as_str)
            .unwrap_or("");
        let name = if crate::ids::is_kebab_case(workload) {
            workload.to_owned()
        } else {
            service_name(service)
        };
        if !crate::ids::is_kebab_case(&name) {
            return None;
        }
        (name, String::new())
    };
    Some(CallKey {
        source: source.to_owned(),
        destination,
        source_namespace: metric
            .get("source_workload_namespace")
            .cloned()
            .unwrap_or_default(),
        destination_namespace: metric
            .get("destination_workload_namespace")
            .cloned()
            .unwrap_or_default(),
        protocol: metric.get("request_protocol").cloned().unwrap_or_default(),
        external,
        external_host,
    })
}

fn is_external(metric: &BTreeMap<String, String>) -> bool {
    let workload = metric
        .get("destination_workload")
        .map(String::as_str)
        .unwrap_or("");
    if matches!(workload, "PassthroughCluster" | "BlackHoleCluster") {
        return true;
    }
    let service = metric
        .get("destination_service")
        .map(String::as_str)
        .unwrap_or("");
    service.contains('.')
        && !service.ends_with(".svc.cluster.local")
        && !service.ends_with(".cluster.local")
}

fn service_name(service: &str) -> String {
    let host = service.split(':').next().unwrap_or(service);
    if host.ends_with(".svc.cluster.local") {
        return host.split('.').next().unwrap_or(host).to_owned();
    }
    host.to_owned()
}

fn external_name(service: &str) -> String {
    let host = service.split(':').next().unwrap_or(service);
    let mut out = String::new();
    for byte in host.bytes() {
        if byte.is_ascii_alphanumeric() {
            out.push((byte as char).to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "external".to_owned()
    } else {
        trimmed.to_owned()
    }
}

fn sample_number(sample: &PromSample) -> Result<Option<(f64, String)>, Error> {
    let value = sample
        .value
        .as_ref()
        .and_then(|pair| pair.get(1))
        .and_then(|item| item.as_str())
        .ok_or_else(|| Error::Config("prometheus sample has no value".to_owned()))?;
    let number: f64 = value
        .parse()
        .map_err(|_| Error::Config(format!("prometheus sample `{value}` is not a number")))?;
    if !number.is_finite() {
        return Ok(None);
    }
    if number < 0.0 {
        return Err(Error::Config(
            "prometheus sample is not a finite count".to_owned(),
        ));
    }
    let stamp = sample
        .value
        .as_ref()
        .and_then(|pair| pair.first())
        .and_then(unix_to_rfc3339)
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_owned());
    Ok(Some((number, stamp)))
}

fn unix_to_rfc3339(value: &Value) -> Option<String> {
    let secs = value
        .as_i64()
        .or_else(|| value.as_f64().map(|number| number as i64))?;
    let time = jiff::Timestamp::from_second(secs).ok()?;
    Some(time.strftime("%Y-%m-%dT%H:%M:%SZ").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_retention_uses_one_day() {
        assert_eq!(query_window(7, Some(3)), "1d");
        assert_eq!(query_window(7, Some(15)), "7d");
        assert_eq!(query_window(7, None), "7d");
        assert!(calls_query("7d").contains("reporter=\"destination\""));
        assert!(errors_query("7d").contains("response_code=~\"5..\""));
        assert!(latency_query("7d").contains("histogram_quantile(0.95"));
        assert!(latency_query("7d").contains("istio_request_duration_milliseconds_bucket"));
    }

    #[test]
    fn health_checks_and_test_clients_are_dropped() {
        let filter = RuntimeFilter {
            health_paths: vec!["/health".to_owned()],
            test_clients: vec!["k6".to_owned()],
        };
        let calls = r#"{
            "status": "success",
            "data": {"resultType": "vector", "result": [
                {"metric": {"source_workload": "billing-api", "source_workload_namespace": "payments-prod", "destination_workload": "payments-api", "destination_workload_namespace": "payments-prod", "request_protocol": "http", "request_path": "/health"}, "value": [1760000000, "9"]},
                {"metric": {"source_workload": "k6", "source_workload_namespace": "payments-prod", "destination_workload": "payments-api", "destination_workload_namespace": "payments-prod", "request_protocol": "http"}, "value": [1760000000, "4"]},
                {"metric": {"source_workload": "billing-api", "source_workload_namespace": "payments-prod", "destination_workload": "payments-api", "destination_workload_namespace": "payments-prod", "destination_service": "payments-api.payments-prod.svc.cluster.local", "request_protocol": "http"}, "value": [1760000000, "100"]}
            ]}
        }"#;
        let empty = r#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
        let records = records_from_prometheus_bodies(calls, empty, empty, "7d", &filter).unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].payload["calls"], 100);
        assert_eq!(records[0].payload["destination"], "payments-api");
    }

    #[tokio::test]
    async fn query_is_a_get_and_does_not_keep_the_token() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::thread;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let body = br#"{"status":"success","data":{"resultType":"vector","result":[]}}"#;
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0_u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let text = String::from_utf8_lossy(&buf[..n]);
                assert!(text.starts_with("GET /api/v1/query?"), "{text}");
                assert!(
                    text.contains("istio_requests_total")
                        || text.contains("istio_request_duration_milliseconds_bucket"),
                    "{text}"
                );
                assert!(text.contains("reporter"), "{text}");
                assert!(
                    text.to_ascii_lowercase()
                        .contains("authorization: bearer fixture-token-value-123456"),
                    "{text}"
                );
                let header = format!(
                    "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                stream.write_all(header.as_bytes()).unwrap();
                stream.write_all(body).unwrap();
            }
        });
        let records = query_prometheus(
            &format!("http://{address}"),
            "7d",
            &RuntimeFilter::default(),
            Some("fixture-token-value-123456"),
        )
        .await
        .unwrap();
        assert!(records.is_empty());
    }
}
