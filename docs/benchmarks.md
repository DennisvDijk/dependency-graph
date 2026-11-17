# Benchmarks: measuring precision and recall

Fixtures prove the code does what we wrote. They cannot show that it finds
what real systems contain. Benchmarks do that part: a real open-source system,
pinned to a commit, plus a list of its true dependencies read from its own
manifests (never from depgraph's output).

```sh
depgraph eval --snapshot data/snapshot.json --truth truth.yaml \
  --min-precision 0.95 --min-recall 0.95
```

It prints precision and recall overall, per edge type and per evidence source,
and lists every missed and every extra edge. `truth.yaml` is
`edges: ["caller -> callee", ...]`; components use their short name, other nodes
their full id.

## In CI

`tests/benchmark.rs` runs the whole pipeline (`collect`, `build`, `eval`) on each
system under `tests/fixtures/benchmarks/` and fails when a score drops below the
ratchet in that file. It is part of `cargo test`, so GitHub and GitLab both run it.
Raise the thresholds when the tool improves; do not lower them to pass.

| System | Pinned to | Truth | Scores |
|---|---|---|---|
| Online Boutique (11 services, Go/Java/C#/Python/Node) | `c7cb6d5` | 16 edges | precision 1.00, recall 1.00 |

## Reading the numbers

A perfect score on one system proves little: the fixes that got there were
tuned on it. The value is the next system. The first Boutique run found 0 nodes
(workloads without an `app.kubernetes.io/name` label were ignored) and one false
call from a documentation link in a message string; both are fixed and now guarded.

## Precision traps

Benchmarks mostly measure recall. Precision is guarded by small `trap_*` tests in
`src/collectors/source.rs`: code that *mentions* a service without calling it must
produce nothing. They cover trailing and block comments, docstrings, placeholder and
local hosts, generated and mock files, dead-letter queues and templated names, plus a
guard that real calls beside those constructs are still found. When a real run shows a
false edge, add the smallest trap that reproduces it, then fix the scanner.

## Adding a system

1. Copy the services' source and manifests into `tests/fixtures/benchmarks/<name>/services/<service>/`
   (drop generated code, assets, tests). Keep the upstream `LICENSE`.
2. Add `fixture.yaml`, `deployments.yaml` and `config/` as in `boutique/`.
3. Write `truth.yaml` from the manifests' env vars and service addresses.
4. Add `#[test] fn <name>() { benchmark("<name>", "0.90", "0.90"); }` and ratchet up.

Next candidates: OpenTelemetry Demo, Istio Bookinfo.

## Systems today

| System | Where the calls are written | Truth | Precision | Recall |
|---|---|---|---|---|
| Online Boutique (Go, C#, Java, Node, Python) | manifest env vars | 16 edges | 1.00 | 1.00 |
| Sock Shop (Java, Go, Node) | source and `application.properties` | 14 edges | 1.00 | 0.79 |

Sock Shop is the harder one: the hosts are literals in code, not env vars. It was
added after Boutique was tuned, so it is the first score not fitted to its own
test. Fixing it exposed four real gaps (a host with a `%s` domain suffix,
`${var:default}` hosts, SQL `tcp(host:port)` DSNs, `*.host=` settings in
properties files) and one resolution fix (a database that runs as a workload is
a component, not a second node). The three edges it still misses are listed in
its README, so the ratchet shows real headroom.
