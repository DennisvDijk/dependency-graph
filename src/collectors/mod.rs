//! Collectors perform I/O and emit raw JSONL records.
//!
//! They do not normalize, and they do not read the clock: the caller passes
//! `finished_at` into [`crate::store::GraphStore::record_collector`]. A failed
//! collector marks itself stale and leaves the previous snapshot in place.
//!
//! The HTTP stack (reqwest, rustls, tokio, Retry-After) and the Kubernetes
//! clients (kube, k8s-openapi) arrive with the first real collector. See ADR-002.
//! None of these stubs contact a source system.

mod base;
mod gitlab_deployments;
mod gitlab_inventory;
mod k8s_workloads;
mod runtime_edges;
mod sbom_links;

pub use crate::raw::RawRecord;
pub use base::{CollectSet, Collector, all, collect};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_collector_is_a_stub() {
        let collectors = all();
        assert_eq!(collectors.len(), 5);
        for collector in collectors {
            let error = collector.collect().unwrap_err();
            assert!(error.to_string().contains("not implemented in F0-1"));
        }
    }

    #[test]
    fn collect_does_not_run_io() {
        let error = collect(CollectSet::Fast).unwrap_err();
        assert!(error.to_string().contains("not implemented in F0-1"));
        assert!(collect(CollectSet::Full).is_err());
    }
}
