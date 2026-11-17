# 8. Terraform, firewalls, Docker, any language and open-source apps

Not everything is a Kubernetes Deployment your team wrote. This tutorial covers the rest of a typical landscape: Terraform, firewall rules, Docker and docker-compose, services in Python, Elixir, Rust or any other language, and open-source apps you run but did not write (Keycloak, Redis, Grafana, an ingress controller…).

The short version:

| What | Shows up as | Without a `catalog-info.yaml` | With one |
|---|---|---|---|
| Terraform repo | `resource:infra:<repo-name>` | Yes, one node per repo that has `.tf` files | Owner, and services can `dependsOn` it |
| Service in any language (Python, Elixir, Rust, Go, Java, Node…) | `component:<name>`, with `languages` on the node | Yes, from its workload label | Owner, system, declared deps |
| Kubernetes workload | `component:<name>` | `Deployment`, `StatefulSet`, `DaemonSet`, `Job`, `CronJob`, Argo `Rollout` with an `app.kubernetes.io/name` label | — |
| AWS security group (Terraform or CloudFormation) | `resource:firewall-rule:<group-name>` on its service; a rule between two groups becomes a candidate edge; an RDS/ElastiCache group becomes `resource:database:<id>` | Yes, fully automatic | — |
| Firewall rule (YAML, `kind: FirewallRule`) | `resource:firewall-rule:<name>`, plus a candidate edge per allowed destination | Yes | — |
| Docker / docker-compose | `container: true` on the node; compose `depends_on` becomes candidate edges | Yes | — |
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

Now `depgraph consumers resource:infra:infra-network` lists every service that depends on that stack. Run it before a risky `terraform apply`.

Tips:

- **One stack, one repo** gives the clearest graph. A monorepo with many stacks shows up as a single node. If you need them apart, split the stacks, or describe each one in a separate repo's catalog file.
- Terraform *inside* a service repo (a bucket or queue for that service) becomes `resource:infra:<service-repo>` and is linked to that service automatically. Network policies and firewall rules in a service repo are linked the same way: by their `app.kubernetes.io/name` label, or, without a label, to the only service in that repo.
- Infra, network-policy and firewall edges are structural: they have status `n/a` and never show up in drift, since no runtime call ever "confirms" a VPC.
- Use `spec.type: database`, `queue`, `topic` and so on for a single managed resource with its own repo, such as an RDS instance. These are the kinds from ADR-007: `database`, `topic`, `queue`, `external-api`, `model-endpoint`, `identity-provider`, `network-policy`, `firewall-rule`, `infra`, `other`.

## Python, Elixir, Rust and other languages

To depgraph a service is a service; the language does not change how it is found. It is found through:

1. its `catalog-info.yaml` (recommended; same file as in [tutorial 2](02-add-your-service.md)),
2. the `app.kubernetes.io/name` label on its `Deployment` manifest,
3. the Istio traffic it sends and receives.

Make the names match: `metadata.name` = `app.kubernetes.io/name` = OpenTelemetry `service.name`. That last one is usually `OTEL_SERVICE_NAME=orders-py` in the deployment's environment, in Python, Elixir (`opentelemetry` hex package), Rust (`opentelemetry` crate) alike.

depgraph also records **which languages a repo uses**, from file names only: `requirements.txt` / `pyproject.toml` → python, `mix.exs` → elixir, `Cargo.toml` → rust, `go.mod` → go, `package.json` → javascript, `pom.xml` / `build.gradle` → java, `Gemfile` → ruby, `*.csproj` → dotnet, `*.tf` → terraform. Build and dependency folders (`target/`, `deps/`, `_build/`, `node_modules/`, `vendor/`, `.venv/`) are skipped. You see it in `depgraph show <name>` and in the MCP `show` tool:

```text
$ depgraph show billing-ex
component:billing-ex
  container: true
  languages: elixir
```

Batch jobs and workers: use `spec.type: job`. They rarely receive traffic, so expect them to show their outgoing calls only.

`propose-catalog` works on Python repos too: it reads the manifests, not the Python code.

## Firewall rules

A YAML file with `kind: FirewallRule` becomes `resource:firewall-rule:<name>`. When it says what it connects, depgraph also draws the connection:

```yaml
kind: FirewallRule
metadata:
  name: billing-egress
spec:
  from:
    - app: billing-ex                              # or just: billing-ex
  to:
    - host: api.stripe.com:443                     # -> resource:external-api:api-stripe-com
    - notifications.shop-prod.svc.cluster.local    # -> component:notifications
    - 0.0.0.0/0                                    # CIDRs and wildcards are skipped
```

- The rule is linked to its `from` service. Without `from` it is linked by its `app.kubernetes.io/name` label, or to the only service in the repo.
- `from` / `source` / `sources` and `to` / `destination` / `destinations` are all accepted, as a single value or a list. List entries may be plain strings or maps with `app`, `component`, `service`, `name` or `host`.
- Each `from → to` pair becomes a **candidate** edge: the network allows it, nobody has seen it yet. If Istio sees the call, the observed edge appears next to it.
- Does your firewall-as-code use a different format (a vendor's export, a CSV, a different YAML shape)? Send one sample file; mapping it is a small change in `src/collectors/gather.rs` (`firewall_endpoints`).

## AWS security groups

On AWS the firewall is the security group. depgraph reads them from Terraform (`.tf`, all files of a repo together) and from CloudFormation YAML, with nothing to configure:

```hcl
resource "aws_security_group" "billing" { name = "payments-sg" }

resource "aws_ecs_service" "billing" {
  name = "billing-ex"                                   # payments-sg belongs to billing-ex
  network_configuration { security_groups = [aws_security_group.billing.id] }
}

resource "aws_security_group_rule" "billing_to_keycloak" {
  type                     = "ingress"
  security_group_id        = aws_security_group.keycloak.id   # name = "prod-keycloak"
  source_security_group_id = aws_security_group.billing.id
}
# -> candidate edge  component:billing-ex -> component:keycloak
```

**Which service owns a group**, first match wins:

1. What it is attached to: `aws_ecs_service` (`name`), `aws_lambda_function` (`function_name`), `aws_db_instance` / `aws_rds_cluster` (`identifier`), `aws_elasticache_*` (`cluster_id`). CloudFormation: `AWS::ECS::Service`, `AWS::Lambda::Function`, `AWS::RDS::DBInstance` / `DBCluster`. A database owner turns into `resource:database:<identifier>`.
2. Its `app`, `service`, `component` or `Name` tag.
3. Its group name (`name`, `name_prefix`, `GroupName`), or else the Terraform label.

**Names are matched to services you already have.** `orders-py-prod-sg`, `prod-orders-py` and `${var.env}-orders-py-sg` all find `orders-py`: depgraph drops words like `sg`, `ecs`, `lambda`, `svc` and environment names (`prod`, `staging`, your own GitLab names) from either end until a known service matches, and applies `config/aliases.yaml`. Nothing new is invented: a group whose name matches no service goes to `unresolved.json` ("no component matches this cloud resource"). Fix it with one alias line, for example `legacy-batch-sg: batch-runner`.

**Which rules count**: inline `ingress` / `egress` blocks with `security_groups`, `aws_security_group_rule`, `aws_vpc_security_group_ingress_rule` / `egress_rule`, and in CloudFormation `SecurityGroupIngress` / `SecurityGroupEgress` (inline or standalone) with `SourceSecurityGroupId` / `DestinationSecurityGroupId`. `!Ref` and `!GetAtt` work. CIDR and prefix-list rules add nothing, because an IP range is not a service.

When a service's own name matches no component, depgraph reads the container image it runs (ECS task definitions and Lambda container images, in Terraform or CloudFormation) and uses the image's repository name instead: `…/orders-py:1.4` links the service to `orders-py`. The service name is always tried first, an image is used only when it names a known component, and nothing is guessed from interpolated image names.

A service or function that names a database in its settings also gets a candidate edge to it. That covers a reference to the database resource (`aws_db_instance.orders.address`, `!GetAtt Db.Endpoint.Address`) and a literal RDS or ElastiCache host name (`orders-db.abc.eu-west-1.rds.amazonaws.com`). It reads task definitions and Lambda settings; secret values are never needed, and environment variable names are ignored.

Local Terraform modules (`source = "./modules/service"` or `"../…"`) are expanded once per `module` call, with the call's plain-string inputs and the module's variable defaults filled in, so `name = "orders-api"` gives each instance its own names. Registry and Git modules, `for_each`/`count`, and values that come from another module's outputs are not followed.

Edges are **candidates**: the security group allows the traffic, nobody has seen it yet. Group ids typed as literals (`sg-0abc…`) and groups from other repos via `data` sources without a name cannot be followed.

## Docker and docker-compose

- A `Dockerfile` (or compose file) in a repo sets `container: true` on the repo's components.
- Image names come from the `depgraph/image-repository` catalog annotation; depgraph does not parse `FROM` lines.
- In `docker-compose.yml` / `compose.yaml`, each `depends_on` becomes a candidate edge **when both services are components** in the graph. Local-only helpers such as `postgres` are not services, so they end up in `unresolved.json` and not in the graph. Declare a real database as a catalog `Resource` instead.
- Containers outside Kubernetes (plain Docker hosts) have no Istio traffic, so they only get declared and candidate edges.

## Kubernetes clusters

depgraph assumes one cluster with one namespace set per environment (ADR-001 question 3). Clusters are not nodes; an environment is `dev`, `acc` or `prod`, mapped from namespaces in `config/environments.yaml`. Several clusters per environment work as long as their namespace names do not collide. A cluster as a first-class node (for "what runs on cluster X?") is not built yet.

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

- **Helm charts are not rendered.** depgraph reads plain Kubernetes YAML only and skips any file containing `{{`, so a chart adds no nodes and no noise. The catalog file above is how a Helm-deployed app gets its node.
- **Check the real pod label.** Upstream charts often label pods `app.kubernetes.io/name: keycloak` but sometimes `app: keycloak-0` or similar. If Istio reports a different name, add an alias in `config/aliases.yaml` (`keycloak-0: keycloak`).
- **Upgrades:** the deploy repo's GitLab `environment:` deployments show up in `depgraph timeline`, so a Keycloak upgrade sits next to the incident it caused.
- **Out of the mesh?** Apps without an Istio sidecar produce no observed traffic. Their declared edges still count; drift then shows them as `declared_only`, which is expected.

## SaaS and external APIs

Calls that leave the cluster through Istio show up as `resource:external-api:<host>` without any work, for example `resource:external-api:api-stripe-com`. They appear as `observed_only` in drift until somebody declares them. That is a cheap way to find every service that talks to a payment provider or an LLM API.

## Try it on sample data

`tests/fixtures/landscape/` holds all of this in one place: a Python service (`orders-py`), an Elixir and a Rust service, AWS security groups in Terraform (`aws-platform/`) and CloudFormation (`notifications/cloudformation/`), a Terraform-only repo, Keycloak and Redis in one deploy repo with a Helm chart, a soft dependency, an OpenAI egress call and an unmapped namespace. `tests/landscape.rs` asserts every node and edge it must produce, so it doubles as a reference for what to expect. To run it yourself from a checkout:

```sh
mkdir -p /tmp/landscape/config && cp config/*.yaml /tmp/landscape/config/
cp tests/fixtures/landscape/{aliases,environments}.yaml /tmp/landscape/config/
cd /tmp/landscape
depgraph collect --set full --output data \
  --fixture "$OLDPWD/tests/fixtures/landscape" \
  --prometheus-fixture "$OLDPWD/tests/fixtures/landscape/prometheus"
depgraph build
depgraph deps orders-py --snapshot data/snapshot.json
depgraph impact keycloak --env prod --snapshot data/snapshot.json   # 0.300: 60% of calls x unknown 0.5
```

The build prints one hint, for the `qa` namespace, which the sample leaves unmapped on purpose.

## What is not covered yet

- **Python, npm or other package dependencies** (who uses library X at version Y). Libraries are switched off on purpose (ADR-001 question 8, ADR-009), so `depgraph library-impact` answers `libraries disabled`. If you need this, it is a new phase: read `requirements.txt` / `pyproject.toml` / SBOMs and turn libraries back on.
- **Resources inside a Terraform stack.** One node per repo, by design.
- **Clusters as nodes.** See [Kubernetes clusters](#kubernetes-clusters).
- **Firewall formats other than `kind: FirewallRule` YAML and AWS security groups** (AWS Network Firewall domain lists, WAF, NACLs). Send a sample.
- **AWS CDK or Pulumi code.** Run `cdk synth` and commit the template, or describe the groups in Terraform/CloudFormation.
- **Rendered Helm output.** Use a catalog file, as above.
