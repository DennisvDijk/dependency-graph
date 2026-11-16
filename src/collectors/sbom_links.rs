//! CycloneDX SBOM links.
//!
//! ADR-009: digest, sha, and tag linking is skipped. There are no internal
//! libraries, and `libraries.enabled` is false. This collector must not emit
//! `Library` nodes or `uses_library` edges.

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
