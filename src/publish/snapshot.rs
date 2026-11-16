//! Publish a canonical snapshot to the tool's own artifact store.
//!
//! This does not push to the source GitLab projects.

use crate::error::Error;

/// Publish the current snapshot.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn publish_snapshot() -> Result<(), Error> {
    Err(Error::NotImplemented("publish snapshot"))
}
