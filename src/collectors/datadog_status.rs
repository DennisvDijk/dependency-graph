//! Optional Datadog deployment and health status.
//!
//! ADR-004: read-only, off unless `datadog.enabled` is set. API and app keys
//! are read from the environment variables named in config. This stub does
//! not call Datadog and does not accept key material as an argument.

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;

/// Datadog status collector stub.
#[derive(Debug, Default)]
pub struct DatadogStatus;

impl Collector for DatadogStatus {
    fn name(&self) -> &'static str {
        "datadog_status"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector datadog_status"))
    }
}
