//! Read-only MCP server.
//!
//! ADR-005. Streamable HTTP, Protected Resource Metadata (RFC 9728), and
//! audience-bound tokens (RFC 8707). PKCE stays with the authorization server.
//! The access token is validated here and is not forwarded.

mod auth;
mod clients;
mod server;

pub use clients::render as render_client_config;

pub use auth::{
    AccessPolicy, AuthKind, EntraEndpoints, JwksCache, authorize_with_jwks, entra_endpoints,
    jwks_from_discovery, policy_from_config,
};
pub use server::{McpState, router, serve};

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
    Tool {
        name: "component_context",
        read_only: true,
    },
];

/// Start the MCP server on `bind`.
///
/// # Errors
///
/// Returns an error when the bind address is refused or insecure mode is
/// pointed at a public address.
pub fn serve_blocking(
    bind: &str,
    insecure: bool,
    policy: auth::AccessPolicy,
    jwks_url: &str,
    snapshot: &std::path::Path,
    aliases: crate::config::Aliases,
) -> Result<(), Error> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::Io)?;
    runtime.block_on(serve(bind, insecure, policy, jwks_url, snapshot, aliases))
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
                "component_context",
            ]
        );
        assert!(TOOLS.iter().all(|tool| tool.read_only));
    }

    #[test]
    fn insecure_local_refuses_a_public_bind() {
        let error = serve_blocking(
            "0.0.0.0:9",
            true,
            auth::AccessPolicy::generic("https://idp.example.com", "http://127.0.0.1:8080"),
            "https://idp.example.com/jwks",
            std::path::Path::new("missing-snapshot.json"),
            crate::config::Aliases::default(),
        )
        .unwrap_err();
        assert!(error.to_string().contains("non-loopback"));
    }
}
