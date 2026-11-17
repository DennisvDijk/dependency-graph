//! MCP over standard input and output.
//!
//! The agent starts `depgraph serve-mcp --stdio` as a child process and talks
//! newline-delimited JSON-RPC. There is no network port and no token: the
//! process boundary is the trust boundary. Nothing but protocol messages may
//! be written to standard output; logs go to standard error.

use std::io::{BufRead, Write};

use serde_json::{Value, json};

use crate::mcp::server::McpState;

/// Read one message per line from `input`, write replies to `output`.
/// Returns at end of input.
pub fn run<R: BufRead, W: Write>(
    state: &McpState,
    input: R,
    output: &mut W,
) -> std::io::Result<()> {
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let replies: Vec<Value> = batch
                    .iter()
                    .filter_map(|message| crate::mcp::rpc::handle(state, message))
                    .collect();
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(message) => crate::mcp::rpc::handle(state, &message),
            Err(_) => Some(json!({
                "jsonrpc": "2.0",
                "id": null,
                "error": { "code": -32700, "message": "parse error" }
            })),
        };
        if let Some(reply) = reply {
            writeln!(output, "{reply}")?;
            output.flush()?;
        }
    }
    Ok(())
}
