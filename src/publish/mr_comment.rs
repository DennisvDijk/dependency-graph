//! Merge-request comment for a dependency check.
//!
//! The comment is produced locally. Posting it to GitLab is a later phase and
//! still does not change application source.

use crate::error::Error;

/// Render a merge-request comment.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn mr_comment() -> Result<(), Error> {
    Err(Error::NotImplemented("publish mr-comment"))
}
