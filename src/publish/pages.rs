//! Static site published as GitLab Pages when this tool runs inside GitLab.

use crate::error::Error;

/// Generate the static site.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn site() -> Result<(), Error> {
    Err(Error::NotImplemented("publish site"))
}
