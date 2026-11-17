# 6. Run it on real data

This is for whoever runs depgraph for a team. At the end you have a snapshot that is refreshed on a schedule, a static site, a weekly drift issue, and a snapshot for the MR check and MCP server.

Work in a separate folder (or a fork of this repo) that holds your `config/` and `data/`. Never commit tokens: depgraph reads them from environment variables or `glab`.

## Checklist

- [ ] GitLab read access (`glab auth login` or `GITLAB_TOKEN`)
- [ ] `config/config.yaml`: GitLab URL and groups
- [ ] `config/environments.yaml`: GitLab environment names and Kubernetes namespaces → dev/acc/prod
- [ ] `config/aliases.yaml` for workloads whose name differs from their catalog name
- [ ] Prometheus URL (and token variable if needed)
- [ ] First `collect` + `build`, and `depgraph doctor` is all `ok`
- [ ] Schedule, site, drift issue
- [ ] `depgraph stats` numbers recorded in ADR-012

## 1. Config

```sh
depgraph init          # writes config/ if missing; never stores a token
```

`config/config.yaml`:

```yaml
gitlab:
  base_url: https://gitlab.yourcompany.com
  groups:
    - platform              # recursive: every project beneath it
  projects: []              # extra single projects outside those groups
prometheus:
  url: https://prometheus.yourcompany.com
  # token_env: DEPGRAPH_PROMETHEUS_TOKEN   # name of the env var, never the token
```

Leave `groups` and `projects` empty to collect every project the token can see.

## 2. Environments: the step people miss

`config/environments.yaml` maps names from each source onto `dev`, `acc` and `prod`:

```yaml
prod:
  gitlab: [production]
  otel: [prod, production]
  k8s_namespaces:
    - payments-prod
    - checkout-prod
```

**Istio calls are placed in an environment through the Kubernetes namespace.** With `k8s_namespaces: []` (the default) runtime calls have no environment, so `--env prod` shows no observed edges and drift shows everything as `declared_only`. List every namespace for each environment. `depgraph resolve-env <name>` checks a single name.

You do not have to know the namespaces up front. Run `depgraph build` once: it lists each namespace whose calls were left out, guesses the environment from a `-prod` / `-acc` / `-dev` (or `staging`, `production`…) prefix or suffix, and prints the YAML to paste:

```text
hint: calls in 2 namespace(s) were left out because config/environments.yaml does not map them:
  shop-prod (500 calls): looks like prod
  qa (5 calls): no guess; map it yourself or ignore it
  check the guesses, then add them under the matching keys in config/environments.yaml:
    prod:
      k8s_namespaces: [shop-prod]
```

The guess is only a hint. Nothing is mapped until it is in the file (ADR-008).

## 3. Aliases

When a workload's name differs from its catalog name, map it in `config/aliases.yaml`:

```yaml
aliases:
  payments: payments-api      # workload name → canonical component name
```

One alias names exactly one component. `depgraph build` reports the number of unresolved names; that number going down is how you know aliases are working.

## 4. Check access

```sh
glab auth login --hostname gitlab.yourcompany.com     # or: export GITLAB_TOKEN=...
depgraph doctor --online    # read-only GET of /api/v4/version
depgraph inventory --output inventory.csv             # which projects it can see
```

`doctor` prints `ok` or `fail` per check, each `fail` with a `next:` line saying what to do.

## 5. First real graph

```sh
depgraph collect --set full --prometheus --output data
depgraph build --raw data/raw --output data/snapshot.json
depgraph doctor
depgraph drift --env prod
```

`--set full` runs every collector; `--set fast` only the incremental ones, for frequent refreshes. A failing collector is marked `stale` and the previous data is kept, so one broken source does not empty the graph.

Sanity checks on the first run:

- `depgraph scorecard`: components per owner, and how many have a system and telemetry.
- `depgraph orphans`: a long list usually means missing namespaces or aliases.
- `depgraph drift --env prod`: a few `observed_only` edges are expected and useful; hundreds mean names do not match.

## 6. Keep it fresh

Example GitLab schedule in the repo that holds your config:

```yaml
depgraph-refresh:
  image: registry.example.com/tools/depgraph:latest
  rules:
    - if: $CI_PIPELINE_SOURCE == "schedule"
  script:
    - depgraph collect --set full --prometheus --output data
    - depgraph build --raw data/raw --output data/snapshot.json
    - depgraph site --snapshot data/snapshot.json --output public
    - depgraph publish snapshot --snapshot data/snapshot.json --output data/published.json
    - depgraph publish drift-issue --snapshot data/snapshot.json --project <tool-group>/depgraph
    - depgraph retain --data data          # drops runs older than retention_days (90)
  artifacts:
    paths: [data/snapshot.json, data/published.json, public/]
```

`publish drift-issue` keeps exactly one open issue labelled `depgraph-drift` up to date and needs the bot token. Run it with `--dry-run` first to see the body. `public/` can be served by GitLab Pages.

Daily `full` is a good start; add `fast` runs if people want fresher answers. `doctor` fails once the snapshot gets old, so it doubles as a freshness check.

## 7. Record the graph-database numbers

```sh
depgraph stats --snapshot data/snapshot.json
```

Paste `active_edges` and `query_p95_ms` into [ADR-012](../decisions/ADR-012-graph-database.md). That decides whether the in-memory store stays enough or a graph database is worth it.

## Share a run summary

If depgraph runs somewhere nobody else can look, such as inside a firewall, `depgraph report` writes a short Markdown summary that is safe to hand over:

```bash
depgraph discover --repos-only      # or the sources you use
depgraph report --output report.md
```

It holds counts only: nodes, edges and the evidence behind them, which collectors worked, and what kinds of names could not be resolved ("built from a variable", "host name or reference", "plain name with no matching service"). It never names a repository, host or service unless you add `--include-names`, which lists up to 40 unresolved names with secrets removed (host and service names stay). Read the file before you share it.

## Optional sources

- **Kubernetes live read** (off): `depgraph discover --kubernetes` reads the current `kubectl` context with read-only calls and adds a cluster node and its workloads. Secrets are never read. Without it, depgraph reads the manifests in Git. Kubeconfigs are referenced by env var name only.
- **Tempo** (`tempo.enabled`, off): stays off until its metric names are verified (ADR-003).
- **Datadog** (`datadog.enabled`, off): read-only status. Keys by env var name only.
