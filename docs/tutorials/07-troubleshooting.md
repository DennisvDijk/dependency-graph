# 7. Troubleshooting

Start with `depgraph doctor`. Every `fail` line ends with a `next:` line that says what to do.

| Symptom | Cause | Fix |
|---|---|---|
| `error: rustc 1.99… is not supported` or a toolchain error on install | Older Rust than the pinned 1.100.0 | Install with [rustup](https://rustup.rs); it reads `rust-toolchain.toml`. On Homebrew: `brew install rustup` and put it first on `PATH`. |
| `fail token: no token` | No GitLab credentials | `glab auth login --hostname <host>` or `export GITLAB_TOKEN=…` |
| `fail glab: glab is not on PATH` | glab not installed | Install [glab](https://gitlab.com/gitlab-org/cli), or use `GITLAB_TOKEN` |
| `fail snapshot: … is missing` | No graph built yet | `depgraph collect --set full --output data && depgraph build` |
| `fail snapshot: snapshot is N days old` | Scheduled refresh stopped | Check the refresh pipeline. With sample data this is expected: `doctor --now <time>`. |
| `--env prod` shows no observed edges; drift is all `declared_only` | Kubernetes namespaces are not mapped | Paste the block that `depgraph build` prints after `hint:` into `config/environments.yaml` ([tutorial 6](06-real-data.md#2-environments-the-step-people-miss)) |
| My service shows up as two nodes, such as `payments` and `payments-api` | Workload name differs from catalog name | Add an alias in `config/aliases.yaml`, or align `app.kubernetes.io/name` |
| `build` reports many unresolved names | Missing aliases or catalog files | See above; `depgraph orphans` lists what is loose |
| Impact treats a `dependsOn` as 0.5, not as hard | By design: `dependsOn` is *unknown* until a failed chaos experiment marks it hard | Mark what you can live without in `depgraph/soft-dependencies`; hard needs experiment evidence |
| A soft dependency shows no edge | The name in `depgraph/soft-dependencies` does not resolve | It accepts a name, project path or `component:default/<name>`; check `unresolved.json` for a `soft_dependency` row |
| A network policy or firewall rule is not linked to my service | No `app.kubernetes.io/name` label, and the repo holds several services | Add the label to the manifest |
| A StatefulSet, CronJob or DaemonSet does not show up | No `app.kubernetes.io/name` label on it | Add the label; the name comes from there, not from `metadata.name` |
| `languages` is missing on a node | The repo has no recognized build file at any depth, or only inside a build folder | Expected for deploy-only repos; see [tutorial 8](08-terraform-python-open-source.md#python-elixir-rust-and-other-languages) |
| `unresolved.json` says "no component matches this cloud resource" | An AWS security group (or the service it is attached to) has a name that contains no known service | Add one line to `config/aliases.yaml`: `<name-from-the-row>: <service>` |
| `unresolved.json` says "environment is not known; set env in the cloud config" | A call seen by X-Ray or Application Insights has no environment word in its service name | Set `env: prod` (or dev/acc) under `aws:` or `azure:` in `config/config.yaml` ([tutorial 9](09-cloud-discovery.md#environments)) |
| `collect --aws` / `--azure` fails with "could not run" or "failed. Check that you are signed in" | The CLI is missing, not signed in, or the role may not list security groups | Run `aws sts get-caller-identity` or `az account show`; other calls that fail (no Lambda access, no X-Ray) only print a warning |
| `unresolved.json` lists `postgres`, `redis` or similar as `candidate_call` | A docker-compose `depends_on` points at a local helper, not a service | Harmless. Declare real databases as catalog `Resource`s |
| `path_score` is 0 | The edge has no observed calls (declared only, or no Prometheus data for that env) | Expected. Scores need Istio traffic. |
| A Helm-deployed or open-source app is missing, or has no owner | Helm templates are not rendered | Add a `catalog-info.yaml` in its deploy repo ([tutorial 8](08-terraform-python-open-source.md#open-source-apps-you-run)) |
| A Terraform stack shows as one node | By design: one `resource:infra` node per repo | Split stacks into repos if you need them apart |
| `library-impact` says `libraries disabled` | Package dependencies are not tracked yet | See [tutorial 8, limits](08-terraform-python-open-source.md#what-is-not-covered-yet) |
| `validate-catalog` says a `dependsOn` does not resolve | The target's file was not passed | Pass the target's `catalog-info.yaml` too |
| `timeline`: `missing field …` | Export has a different shape | Match `tests/fixtures/phase4/deployments.yaml` and `incidents.yaml` |
| MCP client cannot connect | Server not running, other port, or running in Docker with `--insecure-local` | Start `depgraph serve-mcp --insecure-local` on the host; check the URL from `mcp-config` |
| `serve-mcp --insecure-local` refuses to start with `--bind 0.0.0.0:…` | By design: no auth means loopback only | Configure an identity provider ([mcp.md](../mcp.md)) for shared use |
| MR check job is green but posts nothing | Image without `depgraph`, no snapshot, or no bot token; the job is `allow_failure` | Check the job log; see [tutorial 5](05-mr-check.md) |

Still stuck? Run the failing command with the sample data from [tutorial 1](01-try-it.md). If it works there, the issue is config or data; if not, open an issue with the command and its output, after removing anything internal.

## Review suggestions for unresolved names

After `depgraph build`, run `depgraph suggest`. It reads the versioned
`data/unresolved.json` report and `data/snapshot.json`, and prints possible
alias and namespace additions. Review each suggestion and add it to the
existing configuration if it is correct; the command never edits files or
changes the graph. Ambiguous and unrecognized names can remain unresolved.
An older `unresolved.csv` may remain on disk after upgrading; it is no longer
updated and should not be used as the current report.

For optional AI suggestions, run `depgraph suggest --llm --model-id <model-id>`
with an Amazon Bedrock Claude model ID available in your AWS account and
region. This sends unmatched names and known component names to Bedrock using
your configured AWS CLI credentials, may incur charges, and requires permission
to invoke the model. AI output is marked for review; it cannot change your
configuration. The default command does not call Bedrock.
