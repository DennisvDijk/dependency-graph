# Runbook

What to check when the scheduled run goes wrong. Pipelines run on the GitLab
schedules described at the top of `.gitlab-ci.yml`.

## How you find out

- A failed scheduled pipeline emails the schedule owner. Set the owner to a
  shared mailbox or a group, not one person.
- The `freshness` job fails when `data/snapshot.json` is older than
  `FRESHNESS_MAX_HOURS` (default 36). It also fails when the file is missing.
- Run `depgraph freshness` or `depgraph doctor` anywhere to check by hand.

## Symptoms

| Symptom | Likely cause | What to do |
|---|---|---|
| `collect` fails with 401 or 403 | `GITLAB_TOKEN` expired or lost scope | Create a new read-only token, update the CI variable, re-run the schedule |
| `collect` fails with a timeout or 429 | GitLab is slow or rate limiting | Re-run. If it repeats, use `COLLECT_SET=fast`, or raise `DEPGRAPH_CLI_TIMEOUT_SECS` (default 120, see [docker.md](docker.md#timeouts)), and open an issue |
| `build` fails on validation | A collector produced a record the schema rejects | Download the `data` artifact, run `depgraph validate`, fix the collector or open an issue with the failing record (secrets are never in it) |
| `freshness` fails, nothing else failed | The schedule was paused or deleted | CI/CD, Schedules: turn it on and run it |
| Many rows in `data/unresolved.json` | Names do not match, or a new naming scheme | Run `depgraph suggest`, review, update `aliases.yaml` |
| Graph looks wrong after a deploy | A new tool version changed output | Compare with `depgraph diff`, roll back with the previous release (`docs/releases.md`) |

## Recover

1. Re-run the failed job. Most failures are network.
2. If the snapshot is damaged, `depgraph runs` lists archived snapshots and
   `depgraph at` reads one. Copy a good one back to `data/snapshot.json`.
3. Secrets: if a token leaks, revoke it in GitLab first, then rotate the CI
   variable.

## Who owns what

The schedule owner watches the failure email. The platform team owns the CI
variables and runner. Rules and aliases belong to the team that owns the
service.
