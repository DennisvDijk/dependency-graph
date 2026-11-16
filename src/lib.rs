//! Dependency graph for runtime dependencies and deployments.
//!
//! Collectors do I/O and emit raw JSONL. Normalization is pure: no network,
//! no clock, and no randomness. Tokens and kubeconfig contents never enter
//! the graph, the config files, or logs.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod cli;
pub mod collectors;
pub mod config;
pub mod error;
pub mod ids;
pub mod models;
pub mod normalize;
pub mod publish;
pub mod raw;
pub mod redact;
pub mod store;

pub use error::Error;
