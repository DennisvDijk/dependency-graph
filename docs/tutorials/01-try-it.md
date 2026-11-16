# 1. Try it in five minutes

You will build a graph from the sample repos and recorded Istio metrics that ship with the repository, then ask it questions. No token, no network, nothing outside a temporary folder is changed.

## Install

You need Rust. The repository pins Rust 1.100.0 in `rust-toolchain.toml`; with [rustup](https://rustup.rs) that version installs itself on the first build.

```sh
git clone https://github.com/DennisvDijk/dependency-graph.git
cd dependency-graph
cargo install --path . --locked     # puts `depgraph` in ~/.cargo/bin
depgraph --help
```

> On macOS with Homebrew's `rust` instead of rustup you may get an older compiler. `brew install rustup`, then put `$(brew --prefix rustup)/bin` first on your `PATH`.

## Run the demo

```sh
scripts/demo.sh        # or: just demo
```

It walks through nine steps and prints each command before its output. The sample has three services: `payments-api` calls `billing-api` and uses a database `payments-db`.

What to look at:

**Step 3, dependencies.** `payments-api` depends on `billing-api` (declared *and* observed), on its database, and on `api.stripe.com`, which nobody declared but Istio saw.

**Step 4, impact.**

```text
depth  path_score  id
1      0.463       component:payments-api
```

If `billing-api` breaks, `payments-api` is affected. `path_score` ranks how much: the share of the caller's traffic that goes this way (100 of 108 calls) times a hardness factor (0.5, because nobody marked the dependency hard or soft yet). It is a ranking, not a probability.

**Step 5, drift.** Three lists: `both` (declared and seen), `observed_only` (seen but not declared: the Stripe call), and `declared_only` (declared but not seen in prod calls; normal for databases and infra, worth a look for services).

**Step 7, the merge request check.** A change to `openapi/billing.yaml` in `billing-api` is a contract change with a prod consumer, so the MR gets a note saying so.

**Step 9, the site.** Open the `public/index.html` path the script prints for a browsable graph and a data-quality scorecard.

## Try a few things yourself

The script prints its working folder at the end. From there:

```sh
cd /path/printed/by/the/demo
depgraph show payments-api --snapshot data/snapshot.json
depgraph consumers billing-api --snapshot data/snapshot.json
depgraph impact billing-api --env prod --format mermaid --snapshot data/snapshot.json
depgraph critical --snapshot data/snapshot.json
```

Next: [put your own service in the graph](02-add-your-service.md), or [connect your AI agent](04-ai-agent.md) to this sample right away.
