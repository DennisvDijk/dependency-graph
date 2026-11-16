//! Pure normalization. No network, no clock, and no randomness.
//!
//! Collectors hand over raw records. This module is the only place that
//! decides which node and edge those records become.

pub mod merge;
pub mod resolve;
pub mod validate;

pub use resolve::resolve_source_env;
pub use validate::{ValidationReport, Violation, validate};
