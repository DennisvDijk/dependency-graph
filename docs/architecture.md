# Architecture: layers, methods and why

depgraph answers one question: *what depends on this, and what breaks if I change it?*
It does this without per-service configuration, by reading what a company already
has: repositories, manifests, infrastructure code, cloud accounts and telemetry.

This page explains the layers, the methods used in each, and the reasoning. Individual
decisions live in [`docs/decisions/`](decisions/); naming rules in [`naming.md`](naming.md).

For the construction details (types, rules, tools) see [`graph-construction.md`](graph-construction.md).

## The shape in one picture

```text
 sources            collectors          normalize              model & store         analysis          interfaces
 ───────            ──────────          ─────────              ─────────────         ────────          ──────────
 catalog files ┐                                                                      impact / deps     CLI
 manifests     │                       resolve names                                  cycles            generated site
 IaC (HCL, CFN)├─▶ RawRecord ─────────▶ merge into nodes ──▶ GraphSnapshot ─────────▶ deploy order  ──▶ MR comment
 source code   │   {collector,         and edges               (snapshot.json)        drift             MCP server (read-only)
 cloud CLIs    │    kind, payload}     validate                unresolved.json        scores            reports
 telemetry     ┘                       (unknown ⇒ unresolved)  GraphStore = one door
```

Data flows left to right and never back. Each arrow is a plain file on disk
(`data/raw/…`, `data/snapshot.json`, `data/unresolved.json`), so any stage can be
inspected, replayed or tested on its own.

## Layer 1: sources

Everything is read-only. Nothing is written to a repository, cluster or account.

| Source | Examples |
|---|---|
| Declared | `catalog-info.yaml`, Kubernetes manifests, Helm/kustomize output, `docker-compose`, GitLab CI `environment:` |
| Infrastructure code | Terraform (real HCL parser), CloudFormation: queues, topics, subscriptions, security groups, Lambda triggers |
| Source code | Python, Elixir, Rust, JS/TS, Go, Java, Kotlin, C#: URLs, database hosts, literal queue and topic names |
| Gateways | Ingress, HTTPRoute; AWS ALB/NLB and API Gateway; Azure Application Gateway and API Management |
| Live cloud (opt-in) | AWS and Azure list/describe calls, X-Ray, Application Insights, flow logs |
| Live cluster (opt-in) | Kubernetes workloads (never secrets) |
| Telemetry | Istio request metrics from Prometheus |

*Why many sources:* no single source is complete or fully trustworthy. Declarations go stale,
telemetry only sees traffic that happened, code only shows literal names. The graph is the
combination, and each tie remembers which sources support it.

## Layer 2: collectors

A collector turns one kind of source into `RawRecord { collector, kind, payload }`:
small, typed facts such as "this workload exists", "this file calls host X",
"this load balancer routes to this service". Collectors do not decide what a name
*means*; they report what they saw. Code lives in `src/collectors/`.

Methods:

- **Real parsers where the format is structured.** Terraform uses an HCL parser, YAML uses a YAML parser. A file that does not parse is skipped and listed, never guessed at.
- **Literal-name heuristics for source code.** Regular expressions find URLs, connection strings and queue/topic parameters. Computed names (`${env}-orders`) are not guessed.
- **Table-driven rules for cloud resources.** A short list says "this resource type is a gateway, read these fields". Adding a type is a few lines, not a new parser.
- **Redaction at the edge.** Connection strings keep only the host. Token shapes are scrubbed. A test checks that planted secrets never reach output.

*Why this split:* parsers are precise but only exist for structured formats; heuristics reach
everything else but can be wrong. So heuristic output is always labelled as a candidate (below).

## Layer 3: normalize

`src/normalize/` merges raw records into one graph.

1. **Resolve names.** Everything becomes lowercase kebab-case. Hosts like `orders.shop.svc` map to a component by their first label. Explicit aliases (`config/aliases.yaml`) win over inference.
2. **Match loosely, create nothing.** A candidate call to a name that is not a known component does not create an orphan node. It becomes a row in `unresolved.json`.
3. **Merge.** Several records about the same tie become one edge with several sources.
4. **Redirect to intermediaries.** A call to a gateway's hostname points at the gateway node, so a backend change shows the gateway and its callers as affected.
5. **Validate** against the schema and naming rules.

*Why `unresolved.json`:* a wrong edge looks like knowledge. An honest "I saw this but could not place it" lets people fix a name once and keeps the graph trustworthy.

## Layer 4: the model (our "ontology")

The model is deliberately small and closed. It is defined in Rust (`src/models.rs`), and
`schemas/graph.schema.json` is generated from it; a test fails if they drift.

**Nodes**

| Type | Id | Meaning |
|---|---|---|
| Component | `component:<name>` | something that is built and deployed |
| Deployment | `deployment:<name>@<env>` | a component in `dev`, `acc` or `prod` |
| Resource | `resource:<kind>:<name>` | database, topic, queue, external API, gateway, network policy, firewall rule, infra, model endpoint, identity provider, other |
| Library | `library:<purl>` | a versionless package (off by default) |
| Cluster, Workload | | discovered Kubernetes objects |

**Edges**

| Type | Evidence strength |
|---|---|
| `declared_depends_on` | a repo or catalog says so |
| `observed_calls` | telemetry saw the call, per environment |
| `network_traffic` | packets seen; direction is not call direction, so it never drives impact |
| `candidate_calls` | static evidence suggests a tie; not proof |
| `deployed_as`, `runs_in_cluster`, `has_workload` | structure |
| `publishes_to`, `consumes_from` | messaging, with direction taken from the surrounding code |
| `provides_contract`, `uses_contract` | an OpenAPI, AsyncAPI or proto file: the repo's component provides it; a proto import is a declared use, a call to the provider a candidate use |
| `routes_to` | gateway to backend |
| `uses_library` | library use |

Each edge carries `sources` (who said so, when), a `status` (`both`, `observed_only`,
`declared_only`, `candidate`), a `criticality` (`hard`, `soft`, `unknown`) and an
`active` flag. Ids are a hash of `(type, from, to, env)`, so the same tie always has the
same id and diffs between runs are meaningful.

Types are grouped into families in one place (`EdgeType::class()`, `ResourceKind::family()`), so adding a type forces a decision about how it behaves. The families are listed in [`naming.md`](naming.md).

**Ownership.** Each team keeps its own list, so depgraph does not try to unify the lists; it reads the owner each one names. From `catalog-info.yaml` a component gets `owner` (a reference such as `group:default/Team-Payments` is reduced to `team-payments`), `system` and `lifecycle`. A `Group` entity with a `parent` gives the owner's `department`. These are attributes, not nodes, so the closed vocabulary stays closed. `depgraph impact` lists the owner of each affected component and ends with the distinct owners to tell. A component whose owner is missing shows an empty owner, and the scorecard counts it under `(none)`.

*Why keep evidence on the edge:* "declared", "observed" and "guessed" answer different
questions. Drift is the gap between declared and observed. Impact should say how sure it is.

### Why not a formal ontology or blueprints?

Short version: the schema above is an ontology in the plain sense (a fixed vocabulary of
things and relations), but we do not use a formal ontology language or user-defined
blueprints. See [ADR-015](decisions/ADR-015-ontologie-en-blueprints.md).

- **RDF/OWL/SHACL** give inference and open-world modelling. We have no reasoning requirement, no consumer that speaks RDF, and the extra toolchain would cost more than it returns. Impact analysis is graph traversal, not inference.
- **Blueprints** (as in developer portals: each team defines its own entity types and fields) trade zero configuration for flexibility. depgraph's promise is the opposite: it works with nothing configured per service. A closed vocabulary also keeps edges comparable across teams.
- **Backstage's entity model** is an input, not our schema: we read `catalog-info.yaml` (components, owners, `dependsOn`) and map it onto our nodes.
- **Revisit when:** a real need appears that the closed set cannot express (for example ownership or API contracts as first-class nodes), or another tool needs to consume the graph in a standard format. Export (not a new internal model) would be the first step.

## Layer 5: store

`GraphStore` is the only door to the data. Today it is a JSON snapshot on disk,
canonically sorted, so two runs on the same input are byte-identical and diffable.
Snapshots are archived per run for timelines and diffs. A graph database is deferred
until measurements demand it ([ADR-012](decisions/ADR-012-graph-database.md)); moving to one
is a new adapter, not a rewrite.

## Layer 6: analysis

`src/analysis/` and `src/score.rs`.

- **Impact:** follows edges from the changed thing to its dependents. Network traffic does not count. A route to a gateway is not reversed, so a backend change reaches the gateway in front of it.
- **Dependencies, cycles, deploy order, hard edges, orphans, drift, library impact, diffs, point-in-time views.**
- **Scores:** `confidence` says how firm one edge is; the path score is call share times a hardness factor along a path, for ranking, never as a probability ([ADR-011](decisions/ADR-011-confidence-en-padscore.md)). Factors are configuration, not code.
- **Freshness:** a scheduled check fails when the snapshot is older than a limit.

## Layer 7: interfaces

CLI (`depgraph show`, `impact`, `deps`, …), a generated static site, merge-request comments
(`mr-check`), reports safe to share, and a read-only MCP server so AI assistants can ask
the graph instead of guessing. All read the same snapshot.

## Cross-cutting methods

| Method | What it means | Why |
|---|---|---|
| Zero per-service config | discovery by convention; aliases only for exceptions | adoption cost kills dependency tools |
| Candidate, not fact | static findings are labelled and never shown as observed | a confident wrong edge is worse than a missing one |
| Unresolved, not guessed | unknown names go to a list | keeps precision honest and fixable |
| Observed first, short tables second | prefer telemetry; keep a small table of resource types | avoids one parser per cloud resource and the upkeep that brings |
| Deterministic output | sorted, hashed ids, no timestamps in ids | reviewable diffs, reproducible tests |
| Read-only and redacted | no writes, secrets dropped at the collector | safe to run inside a firewall |
| Schema from code | JSON schema generated from Rust types | one source of truth |

## How we know it works

| Layer of testing | What it proves | What it cannot prove |
|---|---|---|
| Unit tests per collector and rule | each rule does what we wrote | that the rule matches real files |
| Landscape and cloud fixtures | whole pipeline on a made-up estate, including live-cloud output saved as files | behaviour on a live account |
| Precision traps (`trap_*` tests in `src/collectors/source.rs`) | code that mentions a service without calling it (comments, docstrings, placeholders, generated and mock files, dead-letter queues) produces nothing, and real calls next to them are still found | traps nobody has thought of yet |
| Secret-leak test | planted secrets never appear in output | file types we did not plant |
| Schema and naming tests | output matches the contract | |
| **Benchmarks** (`tests/benchmark.rs`, `depgraph eval`) | precision and recall on a pinned real open-source system against a hand-written truth list, with a ratchet in CI | accuracy on your estate: tuned on public demos |
| Freshness job | the scheduled pipeline is still producing | |

See [`benchmarks.md`](benchmarks.md). The honest gap: nothing replaces running
`depgraph discover` on real repositories and hand-checking a sample of edges
([ADR-012](decisions/ADR-012-graph-database.md) records where those measurements go).

## Known limits

- Source reading uses literal-name heuristics; findings are candidates.
- GitLab release and CI pipelines, and the live AWS/Azure calls, are tested on fixtures and saved CLI output, not on a real runner or account.
- Service meshes other than Istio telemetry, and Terraform/CloudFormation load balancers, are not read.
- Computed queue names are not read. Application code that names a dead-letter queue is not treated as publishing to it.
- Ownership is an attribute of a component (`owner`, `department`, `lifecycle`), not a node. API contracts (OpenAPI, AsyncAPI, protobuf) and environments as nodes are not modelled yet.
