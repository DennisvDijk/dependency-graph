//! GitLab group and project inventory.
//!
//! The real collector will call the GitLab API with a token read from the
//! environment at call time. The token is not stored in config or logs.

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
