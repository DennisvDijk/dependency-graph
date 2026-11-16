# Fase 0

## F0-1 — Repostructuur, CI en lege pipeline

- **Taak:** F0-1
- **Status:** gereed
- **Gedaan:** Cargo-crate met modellen, ids, config, validatie en een clap-CLI. Collectors, merge, analyse en publishers zijn stubs die `not implemented in F0-1` teruggeven en geen bronsysteem benaderen. `graph.schema.json` komt uit de Rust-modellen. GitHub Actions draait fmt, clippy, test en cargo-deny. `.gitlab-ci.yml` heeft de stages en jobs uit het plan, plus de drie schedules als commentaar voor de GitLab-UI.
- **Tests:** 46 geslaagd (`cargo test --all-targets --all-features`). Fixtures only; geen netwerk.
- **Metrics:** geen. Er is niet verzameld, dus er is geen telling van projecten, edges of dekking.
- **Afwijkingen en ADR:** [ADR-001](../decisions/ADR-001-aannames.md) (standaardaannames), [ADR-002](../decisions/ADR-002-rust-en-github.md) (Rust in plaats van Python; toolrepo op GitHub, bronnen blijven GitLab, Kubernetes, Prometheus/Tempo en CycloneDX).
- **Open vragen:** zie ADR-001, "Vragen die een mens nog moet beantwoorden" en "Verifieer vóór gebruik".
- **Volgende stap:** F0-2 heeft een mens nodig (tokens). F0-3 en F0-5 kunnen met fixtures verder. F0-2 t/m F0-8 zijn niet gestart.

## Nog niet gestart

- **F0-2:** geblokkeerd op tokens van een mens. Geen echt token in deze repo of in de bot.
- **F0-3:** niet gestart. Kan met fixtures.
- **F0-4:** niet gestart.
- **F0-5:** niet gestart. Het catalog-schema is een stub; de afronding kan met fixtures.
- **F0-6, F0-7, F0-8:** niet gestart.
