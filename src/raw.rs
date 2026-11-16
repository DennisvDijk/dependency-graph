//! Raw collector output. One JSON object per JSONL line.
//!
//! Collectors stop here. The normalizer, which is not part of F0-1, is the
//! only code that turns these records into a graph.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One raw record emitted by a collector.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawRecord {
    /// Collector name. Also the JSONL file name.
    pub collector: String,
    /// Record kind inside that collector, such as `project` or `workload`.
    pub kind: String,
    /// Collector payload. Must not contain tokens or credentialed URLs.
    pub payload: Value,
}
