//! Publishers write derived artifacts.
//!
//! They do not write back to GitLab, Kubernetes, Prometheus, or the SBOM
//! store. F0-1 only reserves the commands.

mod drift_issue;
mod mr_comment;
mod pages;
mod snapshot;

pub use drift_issue::drift_issue;
pub use mr_comment::mr_comment;
pub use pages::site;
pub use snapshot::publish_snapshot;
