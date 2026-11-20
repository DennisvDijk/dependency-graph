//! Dependency graph for runtime dependencies and deployments.
//!
//! Collectors do I/O and emit raw JSONL. Normalization is pure: no network,
//! no clock, and no randomness. Tokens and kubeconfig contents never enter
//! the graph, the config files, or logs.

#![forbid(unsafe_code)]

pub mod analysis;
pub mod catalog;
pub mod classify;
pub mod cli;
pub mod collectors;
pub mod config;
pub mod contracts;
pub mod doctor;
pub mod error;
pub mod eval;
pub mod gitlab;
pub mod ids;
pub mod inventory;
pub mod mcp;
pub mod model_config;
pub mod models;
pub mod normalize;
pub mod proc;
pub mod propose;
pub mod publish;
pub mod quickstart;
pub mod raw;
pub mod redact;
pub mod repo_scan;
pub mod report;
pub mod score;
pub mod skill;
pub mod store;
pub mod suggest;
pub mod timeline;

pub use error::Error;
