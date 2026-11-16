# 4. Connect your AI agent

`depgraph serve-mcp` is a read-only [MCP](https://modelcontextprotocol.io) server. Your agent can then ask "what calls billing-api in prod?" or "where is the repo for payments-api?" and get answers from the graph, instead of guessing from code.

## Local, in two terminals

```sh
# terminal 1: start the server on a snapshot (the sample works fine)
depgraph serve-mcp --insecure-local --snapshot tests/fixtures/phase1/graph.json
```

```sh
# terminal 2: print the config for your client
depgraph mcp-config cursor      # or: claude, vscode, opencode
```

The output is the snippet to add to your client's MCP settings:

```json
{
  "mcpServers": {
    "depgraph": { "url": "http://127.0.0.1:8080/mcp" }
  }
}
```

Restart or reload the client and the `depgraph` tools appear. `--insecure-local` turns off authentication, so it refuses to listen on anything except your own machine (loopback) and prints a warning. Do not forward that port.

> Docker: the image's default command uses `--insecure-local`, which only listens on the container's own loopback, so a published port is not reachable from your host. For local use, run the binary.

## Tools the agent gets

| Tool | Use |
|---|---|
| `component_context` | Repo path, URL, default branch, owner, manifests, direct deps and consumers. Good first call before an agent opens a repo. |
| `show`, `deps`, `consumers`, `impact` | Same as the CLI commands. `impact` includes path scores. |
| `deploy_order`, `cycles`, `critical_nodes`, `orphans` | Structure questions. |
| `drift`, `diff` | Declared versus observed; changes between runs. |
| `scorecard` | Data quality per owner. |
| `library_impact` | Returns `libraries disabled` for now. |

Prompts that work well:

- "Use depgraph to find everything that calls billing-api in prod, then check those repos for usages of `/v1/invoices`."
- "Before you change payments-api's API, ask depgraph who the prod consumers are and list them in the MR description."

## A shared server for the team

A shared server needs real auth. depgraph is an OAuth 2.1 resource server: the client does the browser sign-in at your identity provider, and depgraph only checks the token. Nobody pastes a token anywhere.

- Generic IdP (default): set `mcp.issuer`, `mcp.audience` and `mcp.jwks_url`; tokens need scope `depgraph:read`.
- Microsoft Entra ID: set `auth.provider: entra` and supply the tenant and client ids through `DEPGRAPH_ENTRA_TENANT_ID` and `DEPGRAPH_ENTRA_CLIENT_ID`.

Step-by-step app registration, and which scope and role strings to create, are in [mcp.md](../mcp.md).
