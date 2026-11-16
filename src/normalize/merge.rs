//! Merge raw JSONL into a snapshot.
//!
//! The merge is pure and deterministic. F0-1 does not implement it: there is
//! no collector output to merge yet.

use crate::error::Error;

/// Build a snapshot from raw records.
///
/// # Errors
///
/// Always returns [`Error::NotImplemented`] in F0-1.
pub fn merge() -> Result<(), Error> {
    Err(Error::NotImplemented("normalize merge"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_is_a_stub() {
        let error = merge().unwrap_err();
        assert!(error.to_string().contains("not implemented in F0-1"));
    }
}
