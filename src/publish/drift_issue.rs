//! Weekly drift issue body.
//!
//! Opening the issue in GitLab is a later phase. This module does not call
//! the GitLab API.

use crate::error::Error;

/// Build or update the drift issue.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn drift_issue() -> Result<(), Error> {
    Err(Error::NotImplemented("publish drift-issue"))
}
