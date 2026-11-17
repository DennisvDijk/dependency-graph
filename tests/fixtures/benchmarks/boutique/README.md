# Online Boutique benchmark

A trimmed copy of Google's Online Boutique microservices demo
(Apache-2.0, see `LICENSE`), pinned to upstream commit
`c7cb6d5bfd6ea3aed768634ebbcb98cb7a523712`.

Kept: source files, Dockerfiles and each service's `deploy/` manifests.
Dropped: generated protobuf code, static assets, tests, binaries.

`truth.yaml` is the list of real service-to-service calls, read from the
manifests' env vars. `tests/benchmark.rs` runs the real pipeline on this copy
and fails when precision or recall falls below the ratchet in that file.
Raise the thresholds when the tool improves; never lower them to pass.
