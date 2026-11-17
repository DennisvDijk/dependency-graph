# How the graph is built: abstraction layers and tools

This page covers only the construction of the graph: which abstractions it passes
through, which rules turn facts into nodes and edges, and which libraries do the work.
Integrations (what is read from where) are in the [README](../README.md) and
[`architecture.md`](architecture.md); naming rules are in [`naming.md`](naming.md).

## The abstraction ladder

Each rung is a distinct type or file. Data only moves up. A rung knows nothing about the
rungs above it, so each can be tested alone.

| # | Abstraction | Type / file | Produced by | Contains | Knows about |
|---|---|---|---|---|---|
| 0 | **Source material** | files, CLI output, metrics | the world | whatever exists | nothing of ours |
| 1 | **Raw record** | `RawRecord { collector, kind, payload }`, one JSON object per JSONL line in `data/raw/` | collectors (`src/collectors/`) | a small typed fact, secrets already removed | its own source only |
| 2 | **Intermediate candidates** | private structs in `src/normalize/merge.rs` (`CandidateCall`, `ResourceLink`, `Message`, `ObservedAgg`, `DeploymentCand`) | the first pass of `merge_records` | facts grouped by meaning: "caller says it calls X", "resource linked to component Y" | names, but not yet resolved |
| 3 | **Resolved names** | kebab-case strings after aliases, loose matching and gateway-host redirects | `normalize` | one canonical name per thing, or an `unresolved` row | the alias table and environment map |
| 4 | **Nodes and edges** | `Node`, `Edge`, `EdgeSource`, `EdgeAttrs` in `src/models.rs` | the second pass of `merge_records` | typed ids, evidence, status, criticality | the model, not any source |
| 5 | **Snapshot** | `GraphSnapshot` → `data/snapshot.json` | `to_canonical_json` | all nodes and edges, sorted | the schema |
| 6 | **Side files** | `data/unresolved.json`, `meta.json`, `Coverage` | the same build | what was seen but not placed, and counts | |
| 7 | **Indexed view** | petgraph `Graph` built per query in `src/analysis/algo.rs` | analysis | adjacency for one environment and edge filter | a snapshot |
| 8 | **Answer** | impact list, cycles, deploy order, scores | analysis | a result for a person or an assistant | |

*Why a ladder and not one big pass:* a wrong guess at rung 3 would otherwise be baked into
the graph. Keeping raw records on disk means any build can be replayed, diffed or debugged
without touching the sources again.

## Rung 1: raw records

```json
{"collector":"landscape","kind":"candidate_call","payload":{"source":"billing-ex","destination":"shop-acme-store-io","loose":true}}
```

Rules for a record:

- It states one fact in the collector's own words. It never says what the fact means for the graph.
- It carries no secrets: connection strings keep only the host; token shapes are replaced with `***` (`src/redact.rs`).
- `kind` is a short verb or noun (`project`, `workload`, `resource_link`, `candidate_call`, `gateway_host`, `message`). The merge pass dispatches on it.

## Rungs 2 and 3: normalization

`merge_records(records, options)` is a **pure function**: records and options in, graph and
side output out. It reads no clock (`generated_at` is passed in), makes no network call and
touches no disk. This is what makes builds reproducible and testable.

### Name resolution, in order

1. **Kebab-case** every name (`.`, `_`, case differences normalise to `-`).
2. **Aliases** (`config/aliases.yaml`): one alias, one component. An alias pointing at two components is a config error.
3. **Image aliases:** the container image a service runs links it to the component built from that image.
4. **Loose matching**, for candidates only: `payments.shop.svc` matches component `payments` by first label. A loose name that matches nothing becomes an `unresolved` row. It never creates a node.
5. **Gateway hosts:** an external call whose host equals a known gateway host is redirected to that gateway node.
6. **Environment:** namespace or source env name maps to exactly one of `dev`, `acc`, `prod` through `config/environments.yaml`. An unmapped name is unresolved, not defaulted.

*Why guess so little:* a missing edge is a visible gap; a wrong edge looks like knowledge.

### What becomes a node

| Record says | Node |
|---|---|
| a project, catalog entry or workload with a name | `component:<name>` |
| a component deployed to an environment | `deployment:<name>@<env>` |
| a queue, topic, database, gateway, firewall rule, … | `resource:<kind>:<name>` |
| a cluster or workload object | `cluster:k8s-<fingerprint>`, `workload:<fingerprint>` |
| a package (only if `libraries.enabled`) | `library:<purl without version>` |

Ids are built only by functions in `src/ids.rs` (`component_id`, `deployment_id`,
`resource_id`, `library_id`) and parsed back by `parse_node_id`. The id prefix must agree
with the node type or validation fails.

### What becomes an edge

| Fact | Edge | Direction |
|---|---|---|
| catalog or manifest `dependsOn` | `declared_depends_on` | caller → callee |
| telemetry saw a call | `observed_calls` (needs an env) | client → server |
| source code names a host or resource | `candidate_calls` | caller → callee |
| flow log packets | `network_traffic` | packet direction, **ignored by impact** |
| component runs as a deployment | `deployed_as` | component → deployment |
| deployment in cluster / workload of component | `runs_in_cluster`, `has_workload` | |
| code publishes or reads a queue/topic | `publishes_to`, `consumes_from` | per the surrounding verb |
| gateway forwards to a service | `routes_to` | gateway → service, **not reversed** in impact |

**Edge id** = `edge:` + SHA-256 of `type \n from \n to \n env` (UTF-8, no trailing newline).
The same tie always gets the same id, so runs can be compared. Several records that give the
same id are folded into one edge with several `EdgeSource`s (`fold_same_id`).

### Evidence, status and criticality

After folding, three passes enrich each edge:

- **`assign_status`:** `both` (declared and observed), `observed_only`, `declared_only`, `candidate`, or `n/a`.
- **Criticality:** `hard`, `soft` or `unknown`, classified from the evidence (`src/classify.rs`) and then inherited by edges that have none of their own (`inherit_criticality`). Where the value came from is kept in `criticality_source`.
- **`expire_edges`:** an edge not seen for `edge_expiry_days` becomes `active: false`; after `retention_days` it is dropped. Time comes from `generated_at`.

### Confidence

`confidence(edge)` (in `src/analysis/algo.rs`) is a single number for ranking how firm one
edge is. It is not a probability.

| Status | Base |
|---|---|
| both | 0.9 |
| observed_only | 0.6 |
| n/a | 0.5 |
| declared_only | 0.4 |
| candidate | 0.2 |

+0.1 (capped at 1.0) if the edge has measured calls; 0.0 if inactive. The *path score* used
for impact ranking multiplies call share by a hardness factor (hard 1.0, unknown 0.5,
soft 0.2, configurable). See [ADR-011](decisions/ADR-011-confidence-en-padscore.md).

## Rung 5: canonical snapshot

`to_canonical_json` sorts nodes and edges by id, sources by `(kind, first_seen, last_seen,
evidence_ref)`, and uses `BTreeMap` and struct field order for keys. Same input gives
byte-identical output, so a diff between runs shows only real change.

## Validation (a gate, not a step)

`validate()` in `src/normalize/validate.rs` checks the finished graph before anything is
written: header and schema version, node id prefix versus type, edge endpoints exist,
`observed_calls` has an env, every edge has at least one source with
`first_seen ≤ last_seen`, alias integrity, and a secret scan over every string. A
violation fails the build; it never repairs silently.

## Rung 7: querying

Analysis builds a petgraph `Graph` from the snapshot for one environment and one edge
filter (`index`), then walks it:

- **dependencies / consumers / impact:** a breadth-first walk. Impact flows from callee to caller; `network_traffic` is excluded, `routes_to` is not reversed.
- **deploy order:** a topological sort over deploy edges; callees come first.
- **cycles:** strongly connected groups (Tarjan) on the induced subgraph.
- **critical nodes:** betweenness centrality on the undirected graph.
- **diff / edges_at:** set difference of two snapshots; evidence windows for a point in time.

## Tools and why each one

| Tool (crate) | Used for | Why this and not something else |
|---|---|---|
| `serde`, `serde_json` | records, snapshot, side files | the on-disk contract is JSON; derive keeps types and files in step |
| `yaml_serde` | YAML manifests, catalog files, config | maintained fork of the YAML serde backend |
| `hcl-edit` | Terraform | a real HCL parser, so Terraform is read, not pattern-matched |
| `regex` | source-code scanning, ids, module expansion | only for unstructured text, where no parser exists; findings stay candidates |
| `schemars` | generates `schemas/graph.schema.json` from the Rust types | one source of truth; a test fails on drift |
| `jsonschema` | validates files against a schema | catches malformed external input |
| `petgraph` | graph structure and traversal | well-tested; avoids hand-written graph code |
| `sha2`, `hex` | edge ids | stable, content-derived identity |
| `jiff` | timestamps, expiry, freshness | correct time arithmetic without a clock inside pure code |
| `semver` | version precedence between deployments | compares real version strings |
| `clap` | the CLI | derive-based commands and completions |
| `thiserror` | typed errors | errors with a next step, not stack traces |
| `insta`, `assert_cmd`, `tempfile` (dev) | snapshot, CLI and fixture tests | readable diffs on whole-graph output |

Not used on purpose: an RDF or OWL stack, a graph database (until measurements require one,
[ADR-012](decisions/ADR-012-graph-database.md)), and a plugin system for user-defined
entity types ([ADR-015](decisions/ADR-015-ontologie-en-blueprints.md)).

## Design rules that shape every rung

1. **Pure build.** Same records and options give the same graph.
2. **Evidence travels with the edge.** Declared, observed and guessed are never merged into one claim.
3. **Unknown stays unknown.** Unmatched names go to `unresolved.json`.
4. **Closed vocabulary.** New node or edge types need a code change, a schema update and a line in `naming.md`.
5. **Secrets stop at rung 1.** Nothing above it can contain one, and rung 5 validation scans for them anyway.
6. **One door.** `GraphStore` is the only way to read or write snapshots, so changing storage later touches one adapter.

## Extending the graph

| You want | Touch |
|---|---|
| a new thing read from a new place | a collector that emits `RawRecord`s; a branch in `merge_records` only if it needs a new `kind` |
| a new kind of resource | `ResourceKind` in `models.rs`, the schema (`UPDATE_SCHEMA=1 cargo test --test schema`), `naming.md` |
| a new relation | `EdgeType`, its direction rule in analysis, `naming.md`, a fixture and a test |
| a new gateway type in a cloud | one entry in the gateway table of that cloud's collector |
| proof that it helps | a benchmark system and truth file ([`benchmarks.md`](benchmarks.md)) |
