//! Snapshot storage.
//!
//! `snapshot.json` is the graph. `meta.json` is collector bookkeeping and is
//! the only place a redacted failure message is kept. A failed collector does
//! not replace the previous snapshot or the previous raw JSONL.

mod base;
mod json_store;

pub use base::{CollectorMeta, Coverage, DeploymentMismatch, GraphStore, Meta};
pub use json_store::JsonStore;
