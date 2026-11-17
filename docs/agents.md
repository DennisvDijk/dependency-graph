# Using depgraph from a coding agent

Four facets, all optional, all read-only.

| Facet | What it gives the agent | How to get it |
|---|---|---|
| Skill | When and how to use depgraph, how to read the answers | `depgraph skill install` |
| MCP tools (stdio) | `impact`, `deps`, `consumers`, `show`, `component_context` and more, as tool calls | `depgraph serve-mcp --stdio` |
| MCP tools (HTTP) | The same tools from one shared server, with a token | `depgraph serve-mcp`, see [mcp.md](mcp.md) |
| Project notes | A short brief for any agent working in a repo | `AGENTS.md` (Claude Code reads `CLAUDE.md`, which imports it) |

## Per agent

| Agent | Skill | MCP config | Notes |
|---|---|---|---|
| Claude Code | `depgraph skill install --agent claude` | `.mcp.json` in this repo, or `claude mcp add depgraph -- depgraph serve-mcp --stdio` | Plugin: `claude plugin marketplace add DennisvDijk/dependency-graph`, then `claude plugin install depgraph@depgraph` (adds the skill and `/depgraph:impact`; MCP and binary still needed) |
| OpenCode | `--agent opencode` (also reads `.claude/skills` and `.agents/skills`) | `opencode.json` in this repo | Reads `AGENTS.md` |
| Pi | `--agent pi` (also reads `.agents/skills`) | Pi has no built-in MCP client: use the skill, which calls the CLI | |
| Others (Codex, Cursor, Copilot, ...) | `--agent agents` installs to `~/.agents/skills` | Point the agent's MCP setting at `depgraph serve-mcp --stdio` | Read `AGENTS.md` |

`depgraph skill install` with no flags installs for every agent it finds on the machine. `--project` installs into the current repository instead of the home folder.

## Protocol

The server speaks MCP revisions 2025-03-26, 2025-06-18, 2025-11-25 and 2026-07-28 (stateless: `server/discover`, no session). A bad name or argument comes back as a tool result with `isError: true`, so the model can read it and retry. Tool lists are cacheable for five minutes.

## What has been checked

- Checked by tests: the protocol replies above, the tool schemas, the stdio transport against the real binary, the skill's layout rules and install locations.
- Not checked: a live session in OpenCode, Pi, Cursor or Codex, and `claude plugin validate` on the plugin files. Install paths for OpenCode and Pi come from their public documentation and third-party summaries. Run each once and tell us what differs.
