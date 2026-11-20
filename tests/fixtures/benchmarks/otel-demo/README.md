# OpenTelemetry Demo benchmark

A trimmed copy of the OpenTelemetry Demo (Apache-2.0, see `LICENSE`), pinned at
open-telemetry/opentelemetry-demo commit `7a88536cf02f7a6ba5f4ee2b3a2b242a8c56c74c`.
17 services in Go, C#, Java, Kotlin, JavaScript, Python, Ruby, PHP, Rust and C++.

Kept: each service's `Dockerfile` and source files under 40 KB (the frontend only
its server-side gateways), plus `flagd`'s flag file. Dropped: tests, protobuf,
lock files, assets, observability back ends, and anything over the size limit.

## Where the wiring is

This system wires services through `compose.yaml`: a bare `- CART_ADDR` whose value
comes from the upstream `.env` (`CART_ADDR=cart:${CART_PORT}`). depgraph does not
read `.env` files, because they usually hold secrets. So `services/compose/compose.yaml`
is a **trimmed copy**: only the `*_ADDR` and `*_HOST` settings, with the values
`.env` gives them written in, and `build: .` on each service. `depends_on`,
ports, images and everything else are dropped.

## Truth

`truth.yaml` is every address setting that names another service in this fixture
(32 edges), resolved by hand from the pinned `.env`. Left out: the collector,
Grafana, Jaeger, Kafka, Valkey and PostgreSQL (not part of the fixture), and
`0.0.0.0` bind addresses.

## What this measures, and its limits

The truth comes from the same settings the tool reads, so this benchmark checks
that compose settings are read correctly and that addresses, ports and URL forms
are handled, not whether the code uses them. The first run found 0 of 32 edges
(no compose reader for environment values), then a bug where `0.0.0.0` was read
as a service named `0`. Both were fixed after seeing them, so the score is tuned.
The ratchet in `tests/benchmark.rs` is set just below the current score.
