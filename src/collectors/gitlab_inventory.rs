//! GitLab group and project inventory.
//!
//! Collectors may call the REST API directly. For a local run, `glab` auth
//! and host config is the expected access path (ADR-001). The token is never
//! stored in config or logs. F0-1 does not call GitLab.

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;

/// Inventory collector stub.
#[derive(Debug, Default)]
pub struct GitlabInventory;

impl Collector for GitlabInventory {
    fn name(&self) -> &'static str {
        "gitlab_inventory"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector gitlab_inventory"))
    }
}
