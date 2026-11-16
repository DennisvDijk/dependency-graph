//! Kubernetes workloads.
//!
//! The kubeconfig is selected by the environment variable named in config.
//! This module never accepts kubeconfig contents as an argument, and F0-1
//! does not open the Kubernetes API. The client will be `kube` plus
//! `k8s-openapi` (ADR-002). Workload labels default to `app.kubernetes.io/name`.

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;

/// Workload collector stub.
#[derive(Debug, Default)]
pub struct K8sWorkloads;

impl Collector for K8sWorkloads {
    fn name(&self) -> &'static str {
        "k8s_workloads"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector k8s_workloads"))
    }
}
