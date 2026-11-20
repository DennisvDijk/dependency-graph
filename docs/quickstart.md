# Quickstart

Want a guided tour instead? See the [tutorials](tutorials/README.md), or run `scripts/demo.sh` for the full flow on sample data.

From a checkout to a first answer, locally, without a token in the repo.

```sh
git clone https://github.com/DennisvDijk/dependency-graph.git
cd dependency-graph
cargo install --path . --locked
depgraph init
depgraph doctor --now "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
depgraph serve-mcp --insecure-local --snapshot tests/fixtures/phase1/graph.json
```

In another terminal:

```sh
depgraph show payments-api --snapshot tests/fixtures/phase1/graph.json
depgraph mcp-config cursor
```

`init` writes `config/` if it is missing and never stores a GitLab token. If `glab` is logged in, the host can replace `gitlab.example.com`. `doctor` says what to fix next. `--insecure-local` only listens on loopback and prints a warning.

A real graph:

```sh
glab auth login --hostname gitlab.example.com
# edit config/config.yaml gitlab.base_url
depgraph collect --set full --output data
depgraph build --raw data/raw --output data/snapshot.json
depgraph deps payments-api --snapshot data/snapshot.json --env prod
```

Install without a checkout, after a GitHub release exists:

```sh
curl -fsSL https://raw.githubusercontent.com/DennisvDijk/dependency-graph/main/packaging/install.sh | sh
```

Or `cargo install --git https://github.com/DennisvDijk/dependency-graph --locked`. The container image is `docker build -t depgraph .`; see [docker.md](docker.md). Homebrew: `packaging/homebrew/depgraph.rb`.

Entra ID is off unless `auth.provider: entra`. Then the MCP client uses the browser. Do not paste a token. Steps are in [mcp.md](mcp.md).
