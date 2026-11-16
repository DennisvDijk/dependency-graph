//! GitLab environments and deployments.
//!
//! Reads `environment:` names and maps them later through the canonical
//! environment index. F0-1 does not call GitLab.

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;

/// Deployment collector stub.
#[derive(Debug, Default)]
pub struct GitlabDeployments;

impl Collector for GitlabDeployments {
    fn name(&self) -> &'static str {
        "gitlab_deployments"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector gitlab_deployments"))
    }
}
