//! Collectors perform I/O and emit raw JSONL records.
//!
//! They do not normalize, and they do not read the clock: the caller passes
//! `finished_at` into [`crate::store::GraphStore::record_collector`]. A failed
//! collector marks itself stale and leaves the previous snapshot in place.
//!
//! The HTTP stack (reqwest, rustls, tokio, Retry-After) and the Kubernetes
//! clients (kube, k8s-openapi) arrive with the first real collector. See ADR-002.
//! None of these stubs contact a source system.

mod aws;
mod base;
mod datadog_status;
mod gather;
mod gitlab_deployments;
mod gitlab_inventory;
mod k8s_workloads;
mod runtime_edges;
mod sbom_links;

pub use crate::raw::RawRecord;
pub use base::{CollectSet, Collector, all, collect};
pub use gather::{
    GatherSet, GitlabScope, KubectlRead, LiveRead, candidate_records, records_from_fixture,
    records_from_gitlab,
};
pub use runtime_edges::{
    RuntimeFilter, query_prometheus, query_window, records_from_prometheus_bodies,
    records_from_prometheus_dir,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_collector_is_a_stub() {
        let collectors = all();
        assert_eq!(collectors.len(), 6);
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
