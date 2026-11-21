# Changelog

All notable changes to depgraph. Format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow the policy in [docs/releases.md](docs/releases.md#versioning). No release has been tagged yet, so everything below is unreleased and sits on `main`.

## [Unreleased]

### Added
- `depgraph impact` explains each result: the chain of ties and whether it is confirmed or inferred (#40).
- `depgraph diff` names what changed between runs, and can draw it as Mermaid (`--format mermaid`); the MCP server has a `diff` tool that reads archived runs by key (#42, #51).
- Source scanner reads Java host wrappers, Spring `convertAndSend`, environment-variable defaults in four languages, and environment URLs (#41, #47).
- Compose reading: services built from source become components, and hosts named in `environment` become candidate calls. `.env` files and secret-named keys are never read (#50).
- Benchmarks against four real systems with hand-written truth: Online Boutique, Sock Shop, Istio Bookinfo and the OpenTelemetry Demo (#47, #50). Ratchets must never be lowered.
- Release pipelines for GitHub and GitLab: cosign signatures, checksums, a dependency list and, on GitHub, build provenance (#43).
- Apache-2.0 licence (#48).

### Changed
- README rewritten around scanning, with real output; Docker usage, health check and CLI timeout documented (#44, #46, #52).

### Fixed
- `0.0.0.0` and other all-digit hosts were read as a service named `0`; they are now skipped (#50).
- `impact` on a cycle ends and lists each node once, without the start node (#49).
