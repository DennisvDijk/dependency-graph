---
name: analyzing-service-dependencies
description: Answers "what depends on this and what breaks if I change it?" with the depgraph CLI. Finds consumers, dependencies, blast radius, owners to notify, deploy order, cycles and drift for services, queues, databases and gateways, and builds or refreshes the graph when none exists. Use when the user asks about dependencies, impact of a change, who to tell before a release, deploy order, or mentions depgraph, a service graph or snapshot.json.
compatibility: Needs the depgraph binary on PATH and a snapshot (data/snapshot.json). Works offline once the snapshot exists.
metadata:
  version: "1.0"
  source: https://github.com/DennisvDijk/dependency-graph
---

# Analyzing service dependencies with depgraph

depgraph reads repositories, manifests, infrastructure code, cloud and telemetry into a graph of components, resources and the ties between them. Every tie keeps its evidence. Answer from the graph, not from guesses.

## Before anything: is there a graph?

```bash
depgraph doctor
```

- A snapshot exists and is fresh: go to "Answering questions".
- No snapshot: go to "Building the graph".
- Snapshot is stale: tell the user its age, then refresh only if they agree (a refresh may call GitLab or a cloud account).

Default snapshot path is `data/snapshot.json`. Pass `--snapshot <path>` to any read command to use another.

## Answering questions

Pick the command by the question. Add `--env prod` when the user cares about production.

| The user asks | Run |
|---|---|
| What breaks if I change X? | `depgraph impact X --env prod` |
| What does X call or need? | `depgraph deps X` |
| Who calls X? | `depgraph consumers X` |
| Show me X | `depgraph show X` |
| In what order do I deploy A and B? | `depgraph deploy-order component:a,component:b` (comma-separated ids; no target means every component) |
| Are there circular dependencies? | `depgraph cycles` |
| What differs between declared and observed? | `depgraph drift` |
| Which ties are hard (would fail without it)? | `depgraph critical` |
| What changed since last run? | `depgraph diff` (two newest runs; or `--from <run> --to <run>`) |
| A picture | add `--format mermaid` |
| Data for a script | add `--format json` |

Names are kebab-case component names (`payments-api`). If the name is not found, run `depgraph show` on a likely spelling, or look in `data/unresolved.json`; do not invent a node.

## Reading an answer (always do this)

1. **Say who to tell.** `impact` ends with `owners to tell:`. Report those owners. An empty owner means the catalog file names none; say so rather than guessing.
2. **Say how sure each tie is.** Ties marked `candidate` come from reading source code and are suggestions, not facts. `declared_only` means a file says so but no traffic was seen. `observed_only` means traffic but no declaration. `both` is strongest. Never present a `candidate` as certain.
3. **Flag hubs.** A `hub` is linked to many things; a change there reaches far.
4. **Say what the graph cannot see.** If the answer is empty or small, check coverage first (see [references/limits.md](references/limits.md)) before saying "nothing depends on it".

Details of statuses, scores and what each edge type means: [references/reading-results.md](references/reading-results.md).

## Building the graph

Run in this order. Stop and report if a step fails; do not continue on a broken step.

```
- [ ] 1. depgraph init            (only if there is no config; writes a minimal config, stores no tokens)
- [ ] 2. depgraph doctor          (config, glab, snapshot freshness)
- [ ] 3. depgraph discover --repos-only     (repositories only, no live calls)
- [ ] 4. depgraph validate data/snapshot.json   (must pass)
- [ ] 5. depgraph report          (safe to share; read it before answering)
```

`discover` without `--repos-only` may call Kubernetes, Prometheus, AWS or Azure. Those are opt-in flags (`--kubernetes`, `--prometheus`, `--aws`, `--azure`) and read-only. Add one only when the user asked for it or already uses it. Never ask for, paste or store tokens in chat; the user sets credentials in their environment.

Collect and build separately when the user wants control: [references/commands.md](references/commands.md).

## When the result looks wrong

- Few or no edges: the sources may not be readable (private repos, wrong group). Run `depgraph report` and check the unresolved list.
- A service is missing: it may use a name that is not kebab-case or not matched. Suggest an alias: `depgraph suggest`, then have the user review `config/aliases.yaml`.
- An edge is wrong: it is almost always a `candidate`. Do not delete it; tell the user which source produced it (`depgraph show`) so they can fix the source or add an alias.

## Boundaries

- depgraph is read-only. Do not edit the user's repositories or clusters to "fix" a dependency.
- Do not run `publish`, `mr-check` or `retain` unless asked; they write to GitLab or delete archived runs.
- Share `depgraph report` output, not raw `data/raw/` files, which may contain internal hostnames.

More: [references/commands.md](references/commands.md), [references/reading-results.md](references/reading-results.md), [references/limits.md](references/limits.md).
