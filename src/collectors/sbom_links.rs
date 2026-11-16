//! CycloneDX SBOM links.
//!
//! F0-1 expects CycloneDX JSON as a GitLab job artifact. ADR-001 question 8:
//! there are no internal libraries. While `libraries.enabled` is false this
//! collector must not emit `Library` nodes or `uses_library` edges. It does
//! not download artifacts.

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
