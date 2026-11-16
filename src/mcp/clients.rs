//! Copy-paste MCP client config. Stdout is JSON only.

use crate::error::Error;

/// Print the MCP client snippet for `client`.
///
/// `entra` does not put a token in the snippet. The client follows the
/// protected-resource metadata and opens a browser.
///
/// # Errors
///
/// Returns an error when `client` is unknown.
pub fn render(client: &str, bind: &str, entra: bool) -> Result<String, Error> {
    let url = mcp_url(bind);
    let note = if entra {
        "oauth-browser"
    } else {
        "insecure-local"
    };
    let body = match client {
        "cursor" => format!(
            "{{\n  \"mcpServers\": {{\n    \"depgraph\": {{\n      \"url\": \"{url}\"\n    }}\n  }}\n}}\n"
        ),
        "claude" => format!(
            "{{\n  \"mcpServers\": {{\n    \"depgraph\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\"\n    }}\n  }}\n}}\n"
        ),
        "vscode" => format!(
            "{{\n  \"servers\": {{\n    \"depgraph\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\"\n    }}\n  }}\n}}\n"
        ),
        "opencode" => format!(
            "{{\n  \"mcp\": {{\n    \"depgraph\": {{\n      \"type\": \"remote\",\n      \"url\": \"{url}\",\n      \"enabled\": true\n    }}\n  }}\n}}\n"
        ),
        _ => {
            return Err(Error::Usage(format!(
                "unknown MCP client `{client}`.\nnext: depgraph mcp-config cursor|claude|vscode|opencode\n"
            )));
        }
    };
    let _ = note;
    Ok(body)
}

fn mcp_url(bind: &str) -> String {
    let (host, port) = bind.rsplit_once(':').unwrap_or(("127.0.0.1", "8080"));
    let host = if host == "0.0.0.0" || host == "::" {
        "127.0.0.1"
    } else {
        host.trim_matches(['[', ']'])
    };
    format!("http://{host}:{port}/mcp")
}
