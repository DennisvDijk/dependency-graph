//! Runtime edges. ADR-003 selects Istio: `istio_requests_total` with
//! `reporter="destination"`. Tempo is optional and stays off until
//! `traces_service_graph_*` is verified. This stub does not query Prometheus.

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
