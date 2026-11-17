# depgraph: notes for coding agents

depgraph builds a dependency graph of services, queues, databases and gateways and answers "what depends on this, and what breaks if I change it?".

## Ask the graph before you change shared code

- `depgraph impact <component> --env prod`: what breaks, closest first, with the owners to tell.
- `depgraph consumers <component>` / `depgraph deps <component>`: who uses it, what it uses.
- `depgraph show <component>`: one node, its owner and every tie with status and confidence.

A tie marked `candidate` was inferred from source code and is not confirmed. An empty answer can mean the graph does not see it.

## Ways to reach it

| Need | Use |
|---|---|
| Teach the agent how to use depgraph | `depgraph skill install` (Claude Code, OpenCode, Pi and the shared `.agents/skills`) |
| Tools over MCP, started by the agent | `depgraph serve-mcp --stdio` (see `.mcp.json`, `opencode.json`) |
| Tools over MCP, shared server | `depgraph serve-mcp` with a token (see `docs/mcp.md`) |

Details and verification status per agent: `docs/agents.md`.

## Working on this repository

- Rust. CI runs `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test --all-targets`, `cargo deny check`.
- Golden files: `UPDATE_GOLDEN=1 cargo test --test phase1`. Schema: `UPDATE_SCHEMA=1 cargo test --test schema`.
- Benchmarks have a ratchet (`tests/benchmark.rs`); do not lower it to make a change pass.
- Decisions live in `docs/adr/`. Vocabulary is closed and defined in Rust (ADR-015).
