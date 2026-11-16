# 8. Terraform, Python and open-source apps

Not everything is a service your team wrote. This tutorial covers the three other things most landscapes have: infrastructure in Terraform, Python projects, and open-source apps you run but did not write (Keycloak, Redis, Grafana, an ingress controller…).

The short version:

| What | Shows up as | Without a `catalog-info.yaml` | With one |
|---|---|---|---|
| Terraform repo | `resource:infra:<repo-name>` | Yes, one node per repo that has `.tf` files | Owner, and services can `dependsOn` it |
| Python service | `component:<name>`, like any service | Yes, from its Kubernetes `Deployment` label | Owner, system, declared deps |
| Python package (pip library) | Not tracked | — | — (see [limits](#what-is-not-covered-yet)) |
| Open-source app you deploy | `component:<name>` | From plain-YAML `Deployment`s and Istio traffic only | Recommended: owner, system, a stable name |
| SaaS / external API | `resource:external-api:<host>` | Yes, from Istio egress traffic | — |

## Terraform

depgraph does **not** parse Terraform resources or modules. A repository that contains `.tf` files becomes **one** infra node named after the repo:

```text
example-group/platform/infra-network  →  resource:infra:infra-network
```

That is deliberate (ADR-007): the graph answers "which services depend on the network stack?", not "which security group allows port 443?".

To make the node useful, put a `catalog-info.yaml` of kind `Resource` in the Terraform repo:

```yaml
apiVersion: backstage.io/v1alpha1
kind: Resource
metadata:
  name: infra-network          # same as the repo name
spec:
  type: infra
  owner: platform-team
```

and let services that need it declare it:

```yaml
# in the service's catalog-info.yaml
spec:
  dependsOn:
    - resource:default/infra-network
```

Now `depgraph consumers resource:infra:infra-network` lists every service that breaks when that stack does. Run it before a risky `terraform apply`.

Tips:

- **One stack, one repo** gives the clearest graph. A monorepo with many stacks shows up as a single node. If you need them apart, split the stacks, or describe each one in a separate repo's catalog file.
- Terraform *inside* a service repo (a bucket or queue for that service) becomes `resource:infra:<service-repo>` and is linked to that service automatically.
- Use `spec.type: database`, `queue`, `topic` and so on for a single managed resource with its own repo, such as an RDS instance. These are the kinds from ADR-007: `database`, `topic`, `queue`, `external-api`, `model-endpoint`, `identity-provider`, `network-policy`, `firewall-rule`, `infra`, `other`.

## Python

To depgraph a Python service is just a service: the language does not matter. It is found through:

1. its `catalog-info.yaml` (recommended; same file as in [tutorial 2](02-add-your-service.md)),
2. the `app.kubernetes.io/name` label on its `Deployment` manifest,
3. the Istio traffic it sends and receives.

Make the names match: `metadata.name` = `app.kubernetes.io/name` = OpenTelemetry `service.name`. For a Python service that last one is usually `OTEL_SERVICE_NAME=orders-py` in the deployment's environment.

Batch jobs and workers: use `spec.type: job`. They rarely receive traffic, so expect them to show their outgoing calls only.

`propose-catalog` works on Python repos too: it reads the manifests, not the Python code.

## Open-source apps you run

Keycloak, Redis, Grafana or an ingress controller deployed by your team are part of the graph, but nobody writes their code, so nobody adds a catalog file by default. Without one you may see them only as names in Istio traffic, with no owner, or not at all if they come from a Helm chart.

The fix is a small **deploy repo per app** (or the repo that already holds its Helm values) with a `catalog-info.yaml`:

```yaml
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: keycloak
  annotations:
    depgraph/k8s-app-name: keycloak       # the label its pods actually carry
spec:
  type: service
  lifecycle: production
  owner: platform-team                    # the team that runs it, not upstream
  system: identity
```

One repo that deploys several apps can list them all in one file, separated by `---`:

```yaml
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: keycloak
spec: { type: service, lifecycle: production, owner: platform-team, system: identity }
---
apiVersion: backstage.io/v1alpha1
kind: Resource
metadata:
  name: redis-cache
spec: { type: database, lifecycle: production, owner: platform-team }
```

Run `depgraph validate-catalog catalog-info.yaml` before you push; it checks every entity in the file.

Then services declare it like anything else (`dependsOn: [component:default/keycloak]`), and `depgraph consumers keycloak --env prod` tells you who is affected by a Keycloak upgrade.

Things to know:

- **Helm charts are not rendered.** depgraph notes that a repo has a chart, but it reads plain Kubernetes YAML only. A templated label (`{{ .Chart.Name }}`) is not a name. The catalog file above is how a Helm-deployed app gets its node.
- **Check the real pod label.** Upstream charts often label pods `app.kubernetes.io/name: keycloak` but sometimes `app: keycloak-0` or similar. If Istio reports a different name, add an alias in `config/aliases.yaml` (`keycloak-0: keycloak`).
- **Upgrades:** the deploy repo's GitLab `environment:` deployments show up in `depgraph timeline`, so a Keycloak upgrade sits next to the incident it caused.
- **Out of the mesh?** Apps without an Istio sidecar produce no observed traffic. Their declared edges still count; drift then shows them as `declared_only`, which is expected.

## SaaS and external APIs

Calls that leave the cluster through Istio show up as `resource:external-api:<host>` without any work, for example `resource:external-api:api-stripe-com`. They appear as `observed_only` in drift until somebody declares them. That is a cheap way to find every service that talks to a payment provider or an LLM API.

## Try it on sample data

The repository's tests show each case. To see a Terraform repo, a Python service and Keycloak together, make three folders and a `fixture.yaml` that lists them, then:

```sh
depgraph collect --set full --fixture <folder> --output data
depgraph build --raw data/raw --output data/snapshot.json
depgraph deps orders-py --snapshot data/snapshot.json
# component:keycloak, deployment:orders-py@prod, resource:infra:infra-network
```

The layout of a fixture folder is in `tests/fixtures/phase1/` (`fixture.yaml`, `deployments.yaml`, one folder per repo).

## What is not covered yet

- **Python, npm or other package dependencies** (who uses library X at version Y). Libraries are switched off on purpose (ADR-001 question 8, ADR-009), so `depgraph library-impact` answers `libraries disabled`. If you need this, it is a new phase: read `requirements.txt` / `pyproject.toml` / SBOMs and turn libraries back on.
- **Resources inside a Terraform stack.** One node per repo, by design.
- **Rendered Helm output.** Use a catalog file, as above.
