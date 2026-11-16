# depgraph

Self-hosted dependency graph for runtime dependencies and deployments.

The tool is Rust. This repository lives on GitHub. The systems it will read stay where they are: GitLab, Kubernetes, Prometheus/Tempo (or later Istio/Hubble), and CycloneDX SBOMs. It does not write back to those systems. Decisions that differ from the original plan are in [ADR-002](docs/decisions/ADR-002-rust-en-github.md). Working assumptions for the open questions are in [ADR-001](docs/decisions/ADR-001-aannames.md).

Status: **F0-1 only**. Collectors, graph build, analysis, and publishers return `not implemented in F0-1`. `depgraph validate` checks a snapshot for real. No tokens, kubeconfigs, or production data belong in this repo. Config uses `example.com`.

## Layout

```text
config/                 example config, environments, aliases, pilot, sdk hints
schemas/                graph.schema.json (generated) and the catalog-info stub
src/                    models, ids, config, collectors, normalize, store, analysis, publish, cli
ci/depgraph-mr-check.yml
tests/fixtures/         empty graph and a sample catalog-info.yaml
docs/decisions/         ADRs (Dutch)
docs/naming.md          id and hash rules
docs/for-teams.md       short note for application teams (Dutch)
docs/reports/fase-0.md
```

Collectors only do I/O and emit raw JSONL. The normalizer is pure: no network, no clock, no randomness. A collector failure marks that collector `stale` in `meta.json` and keeps the previous snapshot and raw file. Credentialed URLs and known token shapes are rejected in the graph and redacted in errors.

## Develop

Toolchain is pinned in `rust-toolchain.toml` (Rust 1.100.0).

```sh
just fmt
just lint
just test
just ci
```

`just ci` is format-check, clippy (`-D warnings`), and tests. GitHub Actions also runs `cargo deny check`.

```sh
cargo run -- validate tests/fixtures/empty-graph.json
cargo run -- collect --set fast   # exits 2 until a later phase
```

Read commands accept `--env dev|acc|prod`, `--format table|json|mermaid`, and `--snapshot`.

## Configuration

`config/config.yaml` names a kubeconfig environment variable and never the file contents. GitLab tokens are not a config field. `config/environments.yaml` maps source names onto `dev`, `acc`, and `prod`; each source name must resolve to exactly one canonical environment.

Id rules: [docs/naming.md](docs/naming.md).
