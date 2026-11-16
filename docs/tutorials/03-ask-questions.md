# 3. Ask the graph questions

Every query reads a snapshot file. The default is `data/snapshot.json`; pass `--snapshot <file>` for another one. Component names can be given bare (`billing-api`) or as ids (`component:billing-api`).

Common options:

- `--env dev|acc|prod` keeps only edges of that environment (plus edges without one, such as declared dependencies).
- `--format table|json|mermaid`: `json` for scripts, `mermaid` to paste a diagram into a merge request or wiki.
- `--depth N` limits how far a walk goes.

## Questions developers ask, and the command for each

| Question | Command |
|---|---|
| What is this thing, who owns it, where is it deployed? | `depgraph show billing-api` |
| What do we call? | `depgraph deps payments-api --env prod` |
| Who calls us? | `depgraph consumers billing-api --env prod` |
| If we break, who is affected, most affected first? | `depgraph impact billing-api --env prod` |
| In what order should these services go out? | `depgraph deploy-order` |
| Do we have circular dependencies? | `depgraph cycles` |
| Which nodes are single points of failure? | `depgraph critical` |
| What did we declare that nobody uses, and the reverse? | `depgraph drift --env prod` (writes `drift_report/prod.md`) |
| What is not connected to anything? | `depgraph orphans` |
| How is a staging name mapped? | `depgraph resolve-env staging` → `acc` |

## Before you change an API

```sh
depgraph consumers billing-api --env prod
depgraph impact billing-api --env prod --format mermaid > impact.mmd
```

Paste the Mermaid block in your merge request description so reviewers see who is affected. If the MR check is enabled ([tutorial 5](05-mr-check.md)), this happens on its own.

## Reading `impact`

```text
depth  path_score  id
1      0.463       component:payments-api
2      0.120       component:checkout-web
```

- `depth` is how many hops away the caller is.
- `path_score` is the product, along the strongest path, of *call share* (what part of the caller's traffic goes that way) and *hardness* (hard 1.0, unknown 0.5, soft 0.2). Higher means more affected. It is a ranking, not a probability, and it is 0 for edges without observed calls.

## Looking back in time

Each `depgraph build` archives the snapshot under `data/runs/<run>/`.

```sh
depgraph runs                                            # list archived runs
depgraph diff --from 2026-01-08T00-00-00Z --to 2026-01-15T00-00-00Z
depgraph at --date 2026-01-10T00:00:00Z                  # graph as it was then
```

`diff` answers "what changed since last week?", useful right after an incident.

## Deployments next to incidents

```sh
depgraph timeline --env prod --since 2026-01-14T00:00:00Z \
  --deployments deployments.yaml --incidents incidents.yaml
```

Both files are exports, not live calls. The shapes are in `tests/fixtures/phase4/`. Output is one sorted list, so a deploy ten minutes before an incident is easy to spot.

## For scripts

```sh
depgraph consumers billing-api --env prod --format json | jq -r '.visits[].id'
```
