# Fase 0

## F0-1 — Repostructuur, CI en lege pipeline

- **Taak:** F0-1
- **Status:** gereed
- **Gedaan:** Cargo-crate met modellen, ids, config, validatie, een clap-CLI en een MCP-stub (`serve-mcp`, nog zonder OAuth). Collectors, merge, analyse en publishers zijn stubs die `not implemented in F0-1` teruggeven en geen bronsysteem benaderen. `runtime_source` is `istio`, libraries staan uit, Datadog is een optionele read-only stub. `graph.schema.json` komt uit de Rust-modellen. GitHub Actions draait fmt, clippy, test en cargo-deny. `.gitlab-ci.yml` blijft de pipeline voor wanneer de code op GitLab draait.
- **Tests:** 50 geslaagd (`cargo test --all-targets --all-features`). Fixtures only; geen netwerk.
- **Metrics:** geen. Er is niet verzameld, dus er is geen telling van projecten, edges of dekking.
- **Afwijkingen en ADR:** [ADR-001](../decisions/ADR-001-aannames.md) (antwoorden van Dennis plus resterende aannames), [ADR-002](../decisions/ADR-002-rust-en-github.md) (Rust; toolrepo nu op GitHub), [ADR-003](../decisions/ADR-003-runtime-bron.md) (Istio, Tempo optioneel), [ADR-004](../decisions/ADR-004-pilot-en-datadog.md) (alle teamapps, Datadog read-only), [ADR-005](../decisions/ADR-005-mcp-server.md) (OAuth 2.1 MCP-server).
- **Open vragen:** GitLab-versie via `GET /api/v4/version`, en de exacte groeps-paden van Dennis' team. Verifieer-vóór-gebruik staat in ADR-001.
- **Volgende stap:** F0-2 heeft een mens nodig (tokens). F0-3 en F0-5 kunnen met fixtures verder. F0-2 t/m F0-8 zijn niet gestart. De MCP-server hoort bij fase 3, of eerder als Dennis dat wil.

## Nog niet gestart

- **F0-2:** geblokkeerd op tokens van een mens. Geen echt token in deze repo of in de bot.
- **F0-3:** niet gestart. Kan met fixtures.
- **F0-4:** niet gestart.
- **F0-5:** niet gestart. Het catalog-schema is een stub; de afronding kan met fixtures.
- **F0-6, F0-7, F0-8:** niet gestart.
