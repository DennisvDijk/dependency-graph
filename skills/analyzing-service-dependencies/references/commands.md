# Command reference

## Contents
- Building: init, collect, build, discover
- Reading: show, deps, consumers, impact
- Analysis: deploy-order, cycles, drift, critical, orphans
- History: diff, at, runs, timeline
- Health: doctor, freshness, validate, report, scorecard, stats
- Accuracy: eval

Every read command accepts `--snapshot <path>` (default `data/snapshot.json`), `--env dev|acc|prod`, `--format table|json|mermaid` and `--depth <n>`.

## Building

| Command | Does |
|---|---|
| `depgraph init` | Writes a minimal config. Stores no tokens. |
| `depgraph discover --repos-only` | One guided run over repositories only. |
| `depgraph discover --kubernetes --prometheus --aws --azure` | Adds live, read-only sources. Add only what the user uses. |
| `depgraph collect --set full --output data` | Writes raw records to `data/raw/`. |
| `depgraph build --raw data/raw --output data/snapshot.json --aliases config/aliases.yaml` | Merges raw records into the snapshot and writes `data/unresolved.json`. |
| `depgraph suggest` | Proposes aliases and namespace mappings for review. |
| `depgraph validate data/snapshot.json` | Checks the snapshot (the path is an argument, not `--snapshot`). A failure is a real problem, not a warning. |

`--fixture <dir>` runs against a saved landscape instead of GitLab, for trying things out.

## Reading

| Command | Does |
|---|---|
| `depgraph show <name>` | One node with its attributes (`owner`, `department`, `lifecycle`) and ties. |
| `depgraph deps <name>` | What it depends on. |
| `depgraph consumers <name>` | What depends on it. |
| `depgraph impact <name> --env prod` | Everything affected by a change, ranked, with an owner column and "owners to tell". |

## Analysis

| Command | Does |
|---|---|
| `depgraph deploy-order component:a,component:b` | Dependencies first, for the comma-separated ids; no target orders every component. A cycle is reported instead of an order. |
| `depgraph cycles` | Strongly connected groups. |
| `depgraph drift` | Declared versus observed. |
| `depgraph critical` | Hard ties. |
| `depgraph orphans` | Nodes with no ties (often a missed source, not a safe delete). |

## History

`depgraph runs` lists archived runs. `depgraph diff --from <run> --to <run>` compares two. `depgraph at --date <date>` shows the graph at a time. `depgraph timeline` shows deployments and incidents together.

## Health

| Command | Does |
|---|---|
| `depgraph doctor` | Config, `glab`, snapshot freshness. `--online` also checks GitLab. |
| `depgraph freshness` | Fails when the snapshot is older than a limit. For scheduled jobs. |
| `depgraph report` | Summary safe to share. `--include-names` adds up to 40 unresolved names. |
| `depgraph scorecard` | Data quality per owner. An aid, not a ranking for blame. |
| `depgraph stats` | Edge count and query speed. |

## Accuracy

`depgraph eval --snapshot S --truth truth.yaml --min-precision 0.9 --min-recall 0.7` scores a snapshot against a hand-written list (`edges: ["a -> b"]`). Use it only when the user has a truth list. Do not write a truth list from the tool's own output; that proves nothing.
