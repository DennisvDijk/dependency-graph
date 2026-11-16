//! Collector contract. Implementations return records or a stub error.
//! They do not write the graph.

use crate::error::Error;
use crate::raw::RawRecord;

/// Which scheduled set a run belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectSet {
    /// Incremental collectors.
    Fast,
    /// Every collector, including slower SBOM and inventory walks.
    Full,
}

/// One source-system reader.
pub trait Collector {
    /// Stable name used in `meta.json` and `raw/<name>.jsonl`.
    fn name(&self) -> &'static str;

    /// Read the source system and return raw records.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NotImplemented`] in F0-1. Later implementations return
    /// I/O errors. Callers record those through the store, which keeps the
    /// previous snapshot.
    fn collect(&self) -> Result<Vec<RawRecord>, Error>;
}

/// Collectors shipped with the binary, in a stable order.
#[must_use]
pub fn all() -> Vec<Box<dyn Collector>> {
    vec![
        Box::new(super::gitlab_inventory::GitlabInventory),
        Box::new(super::gitlab_deployments::GitlabDeployments),
        Box::new(super::k8s_workloads::K8sWorkloads),
        Box::new(super::runtime_edges::RuntimeEdges),
        Box::new(super::sbom_links::SbomLinks),
        Box::new(super::datadog_status::DatadogStatus),
    ]
}

/// Run a collect set.
///
/// # Errors
///
/// Always returns [`Error::NotImplemented`] in F0-1. The function does not
/// open a socket.
pub fn collect(set: CollectSet) -> Result<(), Error> {
    let _ = set;
    let collectors = all();
    if let Some(collector) = collectors.first() {
        collector.collect()?;
    }
    Err(Error::NotImplemented("collect"))
}
