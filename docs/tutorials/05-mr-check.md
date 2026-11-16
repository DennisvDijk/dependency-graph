# 5. Merge request check

The MR check posts **one** note on a merge request when it changes a contract (OpenAPI, protobuf, schemas) of a service that has consumers in prod. Later pipelines update that same note instead of adding more.

## See what it would say

```sh
depgraph mr-check --dry-run \
  --snapshot data/snapshot.json \
  --project example-group/pilot/billing-api --mr-iid 1 \
  --changed-file openapi/billing.yaml
```

```text
project: example-group/pilot/billing-api
component: billing-api
contract_change: yes
- openapi/billing.yaml
prod consumers:
- component:payments-api (depth 1)
blocking: off
```

What counts as a contract is set by `contract_paths` in `config/config.yaml` (default `openapi/**`, `**/*.proto`, `schema/**`).

## Add it to your repository's pipeline

In your application repo's `.gitlab-ci.yml`:

```yaml
include:
  - project: <tool-group>/depgraph
    file: ci/depgraph-mr-check.yml

variables:
  DEPGRAPH_IMAGE: registry.example.com/tools/depgraph:latest   # an image that contains the depgraph binary
  DEPGRAPH_SNAPSHOT: /data/snapshot.json                        # where the job finds the current snapshot
```

Three things must be in place, and the person who runs depgraph usually sets them up once for everyone:

1. **An image with `depgraph` in it.** The job's fallback image (`debian:bookworm-slim`) does not contain it. Build one from the repo's `Dockerfile` and push it to your registry.
2. **A current snapshot** the job can read: a shared file, an artifact of the scheduled collection pipeline, or the file `depgraph publish snapshot` writes.
3. **A bot token** in the CI/CD variable named by `gitlab.bot_token_env` (default `DEPGRAPH_GITLAB_BOT_TOKEN`), with permission to post MR notes. Mask it and protect it.

The job is `allow_failure: true`: a missing snapshot or token never blocks a merge.

## Blocking mode (off by default)

With `mr_check.blocking: true`, a contract change with observed prod consumers asks for extra approval, unless the MR carries the label `depgraph-override` (`mr_check.override_label`). Turning this on for a team is a deliberate decision, so the default stays off. To try it on one pipeline without changing config, pass `--blocking`.
