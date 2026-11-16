//! Publishers write derived artifacts.
//!
//! They do not write back to GitLab, Kubernetes, Prometheus, or the SBOM
//! store. F0-1 only reserves the commands.

mod drift_issue;
mod mr_comment;
mod pages;
mod snapshot;

pub use drift_issue::{drift_issue_body, upsert_drift_issue};
pub use mr_comment::{MrCheck, MrNote, build_note, upsert_note};
pub use pages::write_site;
pub use snapshot::write_snapshot;
