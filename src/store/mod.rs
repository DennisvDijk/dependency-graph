//! Snapshot storage.
//!
//! `snapshot.json` is the graph. `meta.json` is collector bookkeeping and is
//! the only place a redacted failure message is kept. A failed collector does
//! not replace the previous snapshot or the previous raw JSONL.

mod base;
mod history;
mod json_store;

pub use base::{CollectorMeta, Coverage, DeploymentMismatch, GraphStore, Meta};
pub(crate) use history::runs_dir;
pub use history::{archive_snapshot, list_runs, load_run, retain, run_at};
pub use json_store::JsonStore;
