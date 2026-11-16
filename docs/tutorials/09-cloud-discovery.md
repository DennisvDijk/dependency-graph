# 9. Discover ties from your AWS or Azure account

Repos show what is *written down*. Your cloud account shows what *exists* and, with X-Ray or Application Insights, what *actually talks to what*. This tutorial turns on the live cloud readers. Both are **off by default**. Once you turn one on, it needs no setup per service: no catalog files, no mapping and no list of names.

| | AWS | Azure |
|---|---|---|
| Needs | The `aws` CLI, signed in with read-only access | The `az` CLI, signed in with Reader access |
| Allowed traffic (candidate edges) | Security groups and their rules, attached to ECS services, Lambda functions, RDS/Aurora and ElastiCache | Network security group rules between application security groups (ASGs) |
| Real traffic (observed edges) | X-Ray service map, last 6 hours | Application Insights dependencies (application map), last 7 days |
| Becomes a database / queue / topic | RDS, Aurora, DynamoDB, ElastiCache / SQS / SNS | SQL, Cosmos DB, Redis, PostgreSQL, MySQL / Service Bus / Event Hubs |
| Calls made | `ec2 describe-security-groups`, `ecs list-*` / `describe-services`, `lambda list-functions`, `rds describe-db-*`, `elasticache describe-cache-clusters`, `xray get-service-graph` | `network asg list`, `network nsg list`, `monitor log-analytics query` |

depgraph never sees or stores a key. It runs the CLI you already use, with the login that CLI already has. An error names the command that failed, never its output.

## Try it on sample data first (2 minutes)

The repo has saved CLI output for a small AWS account and an Azure subscription:

```bash
depgraph collect --set full --fixture tests/fixtures/landscape \
  --aws-fixture tests/fixtures/cloud/aws \
  --azure-fixture tests/fixtures/cloud/azure --output data
depgraph build
depgraph deps orders-py --snapshot data/snapshot.json
```

`orders-py` now has a database `orders-aurora` that is allowed by a security group *and* seen by X-Ray, a queue `orders-events`, and an external API `api-mailgun-net`. All of these came from AWS and none were written down anywhere.

## Turn it on for your account

**1. Sign in with read-only access.**

- AWS: any role with the `ReadOnlyAccess` managed policy works. The minimum is `ec2:DescribeSecurityGroups`, `ecs:List*`, `ecs:DescribeServices`, `lambda:ListFunctions`, `rds:Describe*`, `elasticache:DescribeCacheClusters`, `xray:GetServiceGraph`. Check with `aws sts get-caller-identity`.
- Azure: `Reader` on the subscription, plus `Log Analytics Reader` on the workspace that holds Application Insights. Check with `az account show`.

**2. Try one run.** You don't have to edit any config:

```bash
depgraph collect --set full --aws      # or --azure, or both
depgraph build
```

**3. Keep it on.** In `config/config.yaml`:

```yaml
aws:
  enabled: true
  profile: readonly          # optional: an AWS CLI profile name
  regions: [eu-west-1]       # optional: empty uses the CLI's default region
  env: prod                  # optional: see "Environments" below
azure:
  enabled: true
  subscriptions: []          # optional: empty uses the az default
  log_analytics_workspaces:  # only for real traffic from Application Insights
    - 00000000-0000-0000-0000-000000000000
  env: prod
```

Only names and ids go here. Credentials stay with the CLI.

## How names are tied together

Cloud names rarely match repo names exactly. depgraph matches them **loosely** but only to services it already knows from your repos:

| Cloud name | Matches | Why |
|---|---|---|
| ECS service `orders-py-prod` | `orders-py` | `prod` is dropped |
| Security group `orders-py-prod-ecs-sg` on that service | `orders-py` | Attached to the service; `ecs`, `sg`, `prod` are dropped |
| ASG `asg-storefront` with tag `app: edge-proxy-rs` | `edge-proxy-rs` | An `app` / `service` / `component` tag wins |
| X-Ray node `orders-aurora.cluster-abc….rds.amazonaws.com` | `resource:database:orders-aurora` | The first part of the host name |
| `notifications-prod.azurewebsites.net` | `notifications` | A host inside Azure is a service |
| `api.stripe.com` | `resource:external-api:api-stripe-com` | Any other host is external |

Anything that matches nothing goes to `data/unresolved.json` with the reason. Nothing is drawn from a guess. To fix a name, add one line to `config/aliases.yaml` (`legacy-batch: batch-runner`) or tag the resource with `app: <service>`.

Two security groups or ASGs that point at each other become a **candidate** edge, which means *allowed*. An X-Ray or Application Insights call becomes an **observed** edge with source `cloud`, which means *seen*. `depgraph drift` shows where the two disagree, for example a rule that allows traffic nobody uses.

## Environments

An observed call needs an environment. depgraph uses, in order:

1. `env:` in the `aws` or `azure` section, when one account or subscription is one environment;
2. an environment word at the start or end of the service name (`orders-py-prod`, `staging-billing`);
3. otherwise the call goes to `unresolved.json` as `environment is not known; set env in the cloud config`.

### Guessing Kubernetes namespaces (also opt-in)

The same goes for Istio calls in namespaces you have not mapped. By default their calls are left out and `depgraph build` prints a block to paste. To use the guesses directly, put this at the top of `config/environments.yaml`:

```yaml
guess_namespaces: true
```

`depgraph build` then lists each guess (`shop-acc -> acc`). A namespace you map yourself always wins. A namespace with no environment word (`qa`) is still left out.

## What it does not do yet

- **VPC Flow Logs / NSG flow logs.** IP-level traffic would also catch services without X-Ray or Application Insights. Not read yet.
- **New services from the cloud.** Cloud resources tie to services that exist in your repos. An ECS service with no repo shows up in `unresolved.json` and doesn't become a node of its own.
- **Network Firewall, WAF, NACLs, Azure Firewall, CIDR-only rules.** These rules don't say which service they're for.
- **EKS / AKS workloads.** These come from the Kubernetes manifests in your repos (tutorial 8) and Istio, not from these readers.

## Optional: VPC flow logs between ECS services

To include network traffic, enable VPC Flow Logs separately in AWS and publish
**default-format** records to CloudWatch Logs. Then opt in under `aws:` in
`config/config.yaml`; depgraph discovers active flow logs and their log groups:

```yaml
aws:
  enabled: true
  env: prod
  flow_logs: true
  # flow_log_group: /aws/vpc/flow  # optional: limit collection to one group
```

On `depgraph collect --set full --aws`, depgraph reads up to 10,000 log events
from the last six hours, lists running ECS tasks and network interfaces, and
maps IP addresses only when both ends unambiguously match current service tasks
in the same VPC for the whole logged interval. It needs `logs:FilterLogEvents`,
`ec2:DescribeNetworkInterfaces`, `ec2:DescribeFlowLogs`, `ecs:ListTasks` and `ecs:DescribeTasks` in
addition to the permissions above. CloudWatch Logs reads can incur charges.
If a discovered group has a custom or unknown log format, or the result is
truncated or incomplete, depgraph warns and emits no new flow records;
previous raw data, if present, remains in place. Collector errors are also
reported as warnings. Check those warnings before relying on completeness.

The graph labels these edges `network_traffic`, not application calls. They
show packet direction, so a request and its response can appear in opposite
directions; they carry no request count and do not affect dependency drift or
impact calculations. Only accepted, complete default-format VPC records and
running ECS service tasks are included. This does not yet identify Lambda,
RDS, ECS on EC2 without task ENIs, stopped tasks, cross-account identities,
NAT-translated endpoints, custom flow-log formats, S3 logs, or Azure flow
logs. Flow logging itself can have gaps; an absent edge doesn't prove there was
no traffic. Model mappings still have to resolve to known repo components.

To try the packet mapping without an AWS account, use the saved example:

```bash
depgraph collect --set full --fixture tests/fixtures/landscape \
  --aws-fixture tests/fixtures/cloud/aws-flows --output data
depgraph build
```

Azure NSG flow logs are being retired; new Azure collection should use VNet
flow logs. See [Microsoft's migration notice](https://learn.microsoft.com/en-us/azure/network-watcher/nsg-flow-logs-overview).
