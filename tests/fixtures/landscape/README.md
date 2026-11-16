# Landscape fixture

A small but realistic landscape for the graph-correctness test (`tests/landscape.rs`):

- `orders-py`: a Python service. Workload label `orders` (alias), a network policy, a firewall rule without a label, Terraform for its own bucket. Declares Keycloak, Redis and the network stack; names `notifications` only as a soft dependency, by project path.
- `notifications`: a plain service.
- `third-party`: a deploy repo for open-source apps. One catalog file with two entities (Keycloak, Redis) and a Helm chart whose templated labels must be ignored.
- `infra-network`: a Terraform-only repo described as a catalog `Resource`.
- `billing-ex`: an Elixir service run as a StatefulSet, with a firewall rule that names its endpoints (a host, a cluster service, a CIDR) and a docker-compose file.
- `edge-proxy-rs`: a Rust DaemonSet without a catalog file; its `target/` build output must not count.
- `prometheus/`: recorded Istio metrics. Includes an OpenAI egress call and a call from an unmapped `qa` namespace.
