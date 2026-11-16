//! Runtime edges from Tempo, Jaeger, Istio, or Hubble.
//!
//! Metric names (`traces_service_graph_*`), the environment label, and Hubble
//! flow context are unverified. See ADR-001, "Verifieer vóór gebruik". This
//! stub does not query Prometheus or a mesh.

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;

/// Runtime-edge collector stub.
#[derive(Debug, Default)]
pub struct RuntimeEdges;

impl Collector for RuntimeEdges {
    fn name(&self) -> &'static str {
        "runtime_edges"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector runtime_edges"))
    }
}
