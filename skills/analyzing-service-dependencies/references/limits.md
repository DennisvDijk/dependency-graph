# What the graph cannot see

## Contents
- Check coverage first
- Known blind spots
- Measured accuracy

## Check coverage first

Before telling the user "nothing depends on X" or "X has no dependencies":

1. `depgraph report` and read the counts and unresolved names.
2. `depgraph show X`: does the node exist, and from which sources?
3. Ask which sources were enabled. A repositories-only run has no traffic evidence, so every tie is declared or candidate.

## Known blind spots

- Source reading finds literal names only. Hosts built at run time (`prefix + name`, a computed queue name) are not read.
- Hosts built with a helper such as a Java `Hostname("payment")` call are not read.
- Service meshes other than Istio telemetry, and load balancers defined in Terraform or CloudFormation, are not read.
- API contracts (OpenAPI, AsyncAPI, protobuf) are not modelled.
- Ownership is an attribute from catalog files; a repository without a catalog file has no owner.
- Live AWS and Azure reading and the GitLab pipelines have been tested on saved output, not on every real account.

## Measured accuracy

On two public systems with hand-written truth lists, precision is 1.00 and recall 1.00 (Online Boutique) and 1.00 / 0.79 (Sock Shop). These are not numbers for the user's estate. Only a hand check of a sample of edges on the user's own repositories gives that; offer to help pick about 50 edges to verify (see `docs/benchmarks.md`).
