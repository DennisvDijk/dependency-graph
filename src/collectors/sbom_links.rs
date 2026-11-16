//! CycloneDX SBOM links.
//!
//! F0-1 expects CycloneDX JSON as a GitLab job artifact. Internal package
//! prefixes are empty, so this collector must not invent `Library` nodes.
//! It does not download artifacts.

use crate::collectors::base::Collector;
use crate::error::Error;
use crate::raw::RawRecord;

/// SBOM collector stub.
#[derive(Debug, Default)]
pub struct SbomLinks;

impl Collector for SbomLinks {
    fn name(&self) -> &'static str {
        "sbom_links"
    }

    fn collect(&self) -> Result<Vec<RawRecord>, Error> {
        Err(Error::NotImplemented("collector sbom_links"))
    }
}
