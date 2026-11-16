//! Read-only questions over a snapshot.
//!
//! Analysis is pure once it has a graph. F0-1 accepts the questions and
//! returns a stub error instead of inventing answers. `petgraph` arrives with
//! the first algorithm (ADR-002).

use std::path::PathBuf;

use crate::error::Error;
use crate::models::Env;

/// How a read command prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// Aligned text.
    Table,
    /// One JSON document.
    Json,
    /// A Mermaid diagram.
    Mermaid,
}

/// Arguments shared by the read commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadRequest {
    /// Command name, used in the stub error.
    pub command: &'static str,
    /// Optional component, library, or other target.
    pub target: Option<String>,
    /// Second target, reserved for `diff`.
    pub against: Option<String>,
    /// Canonical environment filter.
    pub env: Option<Env>,
    /// Requested output format.
    pub format: OutputFormat,
    /// Snapshot file. `None` means the store default, once that exists.
    pub snapshot: Option<PathBuf>,
}

/// Show one node.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn show(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Dependencies of a component.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn deps(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// What calls or depends on a component.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn consumers(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Transitive impact of a node.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn impact(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// A deploy order that respects hard edges.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn deploy_order(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Cycles in the graph.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn cycles(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Components affected by a library.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn library_impact(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Declared versus observed drift.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn drift(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Edges marked hard.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn critical(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Nodes with no edges.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn orphans(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// Difference between two snapshots.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn diff(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

/// The graph as of a point in time.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1.
pub fn at(request: &ReadRequest) -> Result<(), Error> {
    reject(request)
}

fn reject(request: &ReadRequest) -> Result<(), Error> {
    let _ = (
        request.target.as_ref(),
        request.against.as_ref(),
        request.env,
        request.format,
        request.snapshot.as_ref(),
    );
    Err(Error::NotImplemented(request.command))
}
