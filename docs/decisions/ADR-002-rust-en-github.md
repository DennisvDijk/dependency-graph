# ADR-002: Rust in plaats van Python, en de toolrepo op GitHub

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Twee bewuste afwijkingen van het plan.

## Context

Het plan beschrijft depgraph als een Python 3.12-tool naast de data op GitLab. De eigenaar wijzigt twee dingen: de implementatie is Rust, en de repo van de tool staat op GitHub. De bronnen die de tool léést blijven GitLab, Kubernetes, Prometheus/Tempo (of later Istio/Hubble) en CycloneDX-SBOMs. De tool schrijft niet terug naar die bronsystemen.

[ADR-001](ADR-001-aannames.md) legt de inhoudelijke aannames vast. Dit besluit legt alleen de taal, de dependency-mapping en de plek van de repo vast.

## Besluit

1. depgraph is een Rust-crate (edition 2024, toolchain vastgezet in `rust-toolchain.toml`).
2. Deze repo is de toolrepo en leeft op GitHub. Review en CI voor de tool lopen via GitHub Actions.
3. `.gitlab-ci.yml` blijft bestaan voor het moment dat de collector ín GitLab draait, met de stages en jobs uit het plan.
4. F0-1 neemt de zware clients nog niet op in de build. Collectors en analyse zijn stubs die `not implemented in F0-1` teruggeven en geen socket openen. De crates hieronder zijn wél de gekozen stack zodra die code echt I/O of grafalgoritmen doet.

### Mapping van de Python-stack

| Plan (Python) | Rust | Wanneer |
| --- | --- | --- |
| pydantic | serde, plus schemars om `schemas/graph.schema.json` te genereren en in de test te vergelijken | F0-1, in gebruik |
| networkx | petgraph | bij de eerste analyse, niet in F0-1 |
| httpx | reqwest met rustls, timeouts, retry/backoff, en `Retry-After` op HTTP 429, op tokio | bij de eerste collector |
| pyyaml | yaml_serde, de onderhouden opvolger; `serde_yaml` en `serde_yml` zijn deprecated | F0-1, in gebruik |
| jsonschema | de jsonschema-crate, om fixtures tegen het gegenereerde schema te leggen | F0-1, in de tests |
| kubernetes-client | kube + k8s-openapi | bij de workload-collector |
| typer | clap | F0-1, in gebruik |
| pytest | cargo test, plus insta voor golden files | F0-1, in gebruik |
| ruff / mypy | `clippy -D warnings` en rustfmt | F0-1, in CI |

Een statische binary (en later een container) is het voordeel voor GitLab-runners: geen virtualenv en geen interpreter op de runner. F0-1 compileert in het officiële Rust-image (`rust:1.100.0-bookworm`) een gewone release-binary. Een musl-build kan daar later overheen zonder de pipeline-vorm te wijzigen.

## CI

GitHub Actions (`.github/workflows/ci.yml`) is de poort voor deze repo:

- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features -- -D warnings`
- `cargo test --all-targets --all-features`
- `cargo deny check` (cargo-deny 0.20.2)

`.gitlab-ci.yml` volgt het plan voor de uitvoering in GitLab:

- stages: `test`, `collect`, `build`, `publish`
- jobs: `test`, `collect`, `build`, `publish-snapshot`, `pages`, `drift-issue`
- `build` is `depgraph build`, niet `cargo build`; het binary wordt in `before_script` gebouwd
- schedules, aan te maken in de GitLab-UI omdat YAML geen cron registreert:
  - fast: `0 */6 * * *`, `COLLECT_SET=fast`
  - full: `30 3 * * *`, `COLLECT_SET=full`
  - weekly: `0 7 * * 1`, `COLLECT_SET=full` en `SCHEDULE_KIND=weekly`
- `drift-issue` draait alleen als `SCHEDULE_KIND=weekly`

In F0-1 eindigen `collect`, `build`, `publish snapshot`, `site` en `publish drift-issue` met exitcode 2. De scheduled jobs controleren die code, zodat de lege pipeline groen blijft en toch het binary aanroept. Die assert vervalt zodra het commando echt werk doet.

`ci/depgraph-mr-check.yml` is het include-bestand voor applicatierepo's. In F0-1 neemt het geen contact op met een bronsysteem en faalt het een merge request niet.

## Gevolgen

- Bijdragers hebben de vastgezette Rust-toolchain nodig (`just ci` dekt fmt, clippy en test).
- De tool wordt gereviewd op GitHub. GitLab blijft het bronsysteem voor applicatierepo's, schedules en — zodra de collector daar draait — de data.
- Er is geen Python-packaging, geen `requirements.txt` en geen venv in deze repo.
- Secrets, kubeconfigs en echte grafen horen hier niet in. Dat is ADR-001 vraag 16, en het geldt ook voor deze GitHub-repo.
