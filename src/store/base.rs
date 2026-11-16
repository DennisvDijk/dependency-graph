//! [`GraphStore`] and the fail-soft collector policy.
//!
//! On success the raw JSONL for that collector is replaced. On failure the
//! previous raw file and the previous snapshot stay where they are, and
//! `meta.json` marks the collector `stale`. Messages are redacted first.
//! Collector errors are not copied onto the published graph, so a snapshot
//! cannot grow a secret that only appeared in an error string.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::models::{GraphSnapshot, RunStatus, Timestamp};
use crate::raw::RawRecord;
use crate::redact;

/// Bookkeeping for one collector.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CollectorMeta {
    /// `ok` when the latest run was kept, otherwise `stale` after a failure.
    pub status: RunStatus,
    /// When the outcome was recorded. The caller supplies the clock.
    pub finished_at: Timestamp,
    /// Record count from the last successful run. A failure keeps the old count.
    pub count: u64,
    /// Redacted failure text. Absent after success.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// `meta.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Meta {
    /// Collector name to the latest outcome.
    #[serde(default)]
    pub collectors: BTreeMap<String, CollectorMeta>,
    /// Deployments dropped because another version won for the same component and env.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deployment_mismatches: Vec<DeploymentMismatch>,
    /// How much of the graph is declared, observed, or still unresolved.
    #[serde(default, skip_serializing_if = "Coverage::is_empty")]
    pub coverage: Coverage,
}

/// Counts written next to the snapshot. Derived from the graph, not the clock.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Coverage {
    /// Component nodes.
    pub components: u64,
    /// `declared_depends_on` edges.
    pub declared_edges: u64,
    /// `observed_calls` edges.
    pub observed_edges: u64,
    /// Edges whose status is `both`.
    pub both: u64,
    /// Edges whose status is `observed_only`.
    pub observed_only: u64,
    /// Edges whose status is `declared_only`.
    pub declared_only: u64,
    /// Edges whose status is `candidate`.
    pub candidates: u64,
    /// Rows in `unresolved.csv`.
    pub unresolved: u64,
    /// Resource nodes created because the destination was outside the mesh.
    pub inferred_resources: u64,
}

impl Coverage {
    fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// One component and environment had more than one deployment version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeploymentMismatch {
    /// Canonical component name.
    pub component: String,
    /// Canonical environment.
    pub env: String,
    /// Version that was kept.
    pub chosen: String,
    /// Versions that were not kept.
    pub discarded: Vec<String>,
}

/// Persistence for snapshots, raw JSONL, and collector metadata.
pub trait GraphStore {
    /// Load and canonicalize a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the file is missing, unreadable, or invalid.
    fn load_snapshot(&self) -> Result<GraphSnapshot, Error>;

    /// Validate and write a canonical snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the graph breaks an invariant or the write fails.
    fn save_snapshot(&self, graph: &GraphSnapshot) -> Result<(), Error>;

    /// Load metadata. A missing file is an empty meta document.
    ///
    /// # Errors
    ///
    /// Returns an error when the file exists but cannot be parsed.
    fn load_meta(&self) -> Result<Meta, Error>;

    /// Write metadata.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be written.
    fn save_meta(&self, meta: &Meta) -> Result<(), Error>;

    /// Replace `raw/<collector>.jsonl`.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is unsafe or the write fails.
    fn write_raw(&self, collector: &str, records: &[RawRecord]) -> Result<(), Error>;

    /// Apply one collector result without dropping the previous snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the collector name or timestamp is unusable, or
    /// when metadata cannot be saved. A collector failure is recorded as
    /// `stale` and then returns `Ok`.
    fn record_collector(
        &self,
        name: &str,
        result: &Result<Vec<RawRecord>, Error>,
        finished_at: &Timestamp,
    ) -> Result<(), Error> {
        validate_collector_name(name)?;
        if finished_at.to_datetime().is_err() {
            return Err(Error::Store(
                "finished_at is not an RFC 3339 timestamp".to_owned(),
            ));
        }
        let mut meta = self.load_meta()?;
        let previous_count = meta
            .collectors
            .get(name)
            .map(|item| item.count)
            .unwrap_or(0);
        let entry = match result {
            Ok(records) => {
                if let Some(kind) = secret_in_records(records) {
                    CollectorMeta {
                        status: RunStatus::Stale,
                        finished_at: finished_at.clone(),
                        count: previous_count,
                        message: Some(format!(
                            "collector output was discarded because it contains {kind}"
                        )),
                    }
                } else {
                    self.write_raw(name, records)?;
                    CollectorMeta {
                        status: RunStatus::Ok,
                        finished_at: finished_at.clone(),
                        count: u64::try_from(records.len()).unwrap_or(u64::MAX),
                        message: None,
                    }
                }
            }
            Err(err) => CollectorMeta {
                status: RunStatus::Stale,
                finished_at: finished_at.clone(),
                count: previous_count,
                message: Some(redact::redact(&err.to_string())),
            },
        };
        meta.collectors.insert(name.to_owned(), entry);
        self.save_meta(&meta)
    }
}

/// Reject collector names that could escape the raw directory.
///
/// # Errors
///
/// Returns an error when `name` is not a lowercase slug.
pub fn validate_collector_name(name: &str) -> Result<(), Error> {
    if !name.is_empty()
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        Ok(())
    } else {
        Err(Error::Store(format!("invalid collector name `{name}`")))
    }
}

fn secret_in_records(records: &[RawRecord]) -> Option<&'static str> {
    for record in records {
        if let Some(kind) =
            redact::secret_kind(&record.collector).or_else(|| redact::secret_kind(&record.kind))
        {
            return Some(kind);
        }
        if let Some(hit) = redact::find_secrets(&record.payload).into_iter().next() {
            return Some(hit.kind);
        }
    }
    None
}
