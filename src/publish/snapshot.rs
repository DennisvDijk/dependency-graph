//! Publish a canonical snapshot to the tool's own artifact store.
//!
//! This does not push to the source GitLab projects.

use std::fs;
use std::path::Path;

use crate::error::Error;
use crate::models::{GraphSnapshot, to_canonical_json};

/// Write a canonical snapshot file.
///
/// # Errors
///
/// Returns an error when the file cannot be written.
pub fn write_snapshot(graph: &GraphSnapshot, path: &Path) -> Result<(), Error> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        path,
        to_canonical_json(graph).map_err(|err| Error::Store(err.to_string()))?,
    )?;
    Ok(())
}
