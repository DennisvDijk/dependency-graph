//! Read-only MCP server surface.
//!
//! ADR-005 makes this a core requirement (former plan task F3-6). OAuth 2.1,
//! Streamable HTTP, Protected Resource Metadata (RFC 9728), and audience-bound
//! tokens (RFC 8707) are not implemented. PKCE stays with the authorization
//! server. JWT checks against an external IdP JWKS land with the real server,
//! in phase 3 or earlier if asked.
//!
//! Every tool is read-only. None of them write to a source system.

use crate::error::Error;

/// One MCP tool. Handlers do not exist yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tool {
    /// Tool name exposed to the client.
    pub name: &'static str,
    /// Tools in this list never mutate a source system.
    pub read_only: bool,
}

/// The tool list from ADR-005. Order is stable.
pub const TOOLS: &[Tool] = &[
    Tool {
        name: "show",
        read_only: true,
    },
    Tool {
        name: "deps",
        read_only: true,
    },
    Tool {
        name: "consumers",
        read_only: true,
    },
    Tool {
        name: "impact",
        read_only: true,
    },
    Tool {
        name: "deploy_order",
        read_only: true,
    },
    Tool {
        name: "cycles",
        read_only: true,
    },
    Tool {
        name: "library_impact",
        read_only: true,
    },
    Tool {
        name: "drift",
        read_only: true,
    },
    Tool {
        name: "critical_nodes",
        read_only: true,
    },
    Tool {
        name: "orphans",
        read_only: true,
    },
    Tool {
        name: "diff",
        read_only: true,
    },
];

/// Start the MCP server.
///
/// # Errors
///
/// Returns [`Error::NotImplemented`] in F0-1. The process does not bind a port
/// and does not perform OAuth.
pub fn serve() -> Result<(), Error> {
    let _ = TOOLS;
    Err(Error::NotImplemented("serve-mcp"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_list_matches_adr_005_and_is_read_only() {
        let names: Vec<_> = TOOLS.iter().map(|tool| tool.name).collect();
        assert_eq!(
            names,
            vec![
                "show",
                "deps",
                "consumers",
                "impact",
                "deploy_order",
                "cycles",
                "library_impact",
                "drift",
                "critical_nodes",
                "orphans",
                "diff",
            ]
        );
        assert!(TOOLS.iter().all(|tool| tool.read_only));
    }

    #[test]
    fn serve_is_a_stub() {
        let error = serve().unwrap_err();
        assert!(error.to_string().contains("not implemented in F0-1"));
    }
}
