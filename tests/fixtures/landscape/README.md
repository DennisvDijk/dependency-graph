# Landscape fixture

A small but realistic landscape for the graph-correctness test (`tests/landscape.rs`):

- `orders-py`: a Python service. Workload label `orders` (alias), a network policy, a firewall rule without a label, Terraform for its own bucket. Declares Keycloak, Redis and the network stack; names `notifications` only as a soft dependency, by project path.
- `notifications`: a plain service, with a CloudFormation template (short-form `!Ref`/`!GetAtt`) whose security group admits an `orders-py` Lambda.
- `third-party`: a deploy repo for open-source apps. One catalog file with two entities (Keycloak, Redis) and a Helm chart whose templated labels must be ignored.
- `infra-network`: a Terraform-only repo described as a catalog `Resource`.
- `billing-ex`: an Elixir service run as a StatefulSet, with a firewall rule that names its endpoints (a host, a cluster service, a CIDR) and a docker-compose file.
- `edge-proxy-rs`: a Rust DaemonSet without a catalog file; its `target/` build output must not count.
- `aws-platform`: AWS security groups in Terraform across two files. Groups tied by ECS attachment (`payments-sg` -> billing-ex), by tag, and by name (`orders-py-prod-sg`, `prod-keycloak`); an RDS group (-> `resource:database:orders-db`); a CIDR-only rule; and `legacy-batch-sg`, which matches no service and must stay out of the graph.
- `orders-py/deploy/ingress.yaml`: an Ingress `public-edge` for host `shop.acme-store.io`, routing to `orders-py` and to `no-such-service` (reported, not invented). `billing-ex` calls that host in code, so it gets a candidate edge to the gateway.
- `prometheus/`: recorded Istio metrics. Includes an OpenAI egress call and a call from an unmapped `qa` namespace.
