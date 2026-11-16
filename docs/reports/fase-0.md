# Fase 0

## F0-1 — Repostructuur, CI en lege pipeline

- **Taak:** F0-1
- **Status:** gereed
- **Gedaan:** Cargo-crate met modellen, ids, config, validatie, een clap-CLI en een MCP-stub (`serve-mcp`, nog zonder OAuth). Collectors, merge, analyse en publishers zijn stubs die `not implemented in F0-1` teruggeven en geen bronsysteem benaderen. `runtime_source` is `istio`, libraries staan uit, Datadog is een optionele read-only stub. `graph.schema.json` komt uit de Rust-modellen. GitHub Actions draait fmt, clippy, test en cargo-deny. `.gitlab-ci.yml` blijft de pipeline voor wanneer de code op GitLab draait.
- **Tests:** 65 geslaagd (`cargo test --all-targets --all-features`). Fixtures only; geen netwerk.
- **Metrics:** geen. Er is niet verzameld, dus er is geen telling van projecten, edges of dekking.
- **Afwijkingen en ADR:** [ADR-001](../decisions/ADR-001-aannames.md) (antwoorden van Dennis plus resterende aannames), [ADR-002](../decisions/ADR-002-rust-en-github.md) (Rust; toolrepo nu op GitHub), [ADR-003](../decisions/ADR-003-runtime-bron.md) (Istio, Tempo optioneel), [ADR-004](../decisions/ADR-004-pilot-en-datadog.md) (alle teamapps, Datadog read-only), [ADR-005](../decisions/ADR-005-mcp-server.md) (OAuth 2.1 MCP-server).
- **Open vragen:** GitLab-versie via `GET /api/v4/version`, en de exacte groeps-paden van Dennis' team. Verifieer-vóór-gebruik staat in ADR-001.
- **Volgende stap:** F0-3 t/m F0-8 zijn met fixtures gebouwd. F0-2 (een echt token) blijft een menselijke stap. Dennis draait de eerste echte inventory lokaal via `glab`. De MCP-server hoort bij fase 3, of eerder als Dennis dat wil.

## F0-3 t/m F0-8

- **Taak:** inventory, namen/aliassen, catalogusschema, omgevingsmapping, pilot, propose-catalog
- **Status:** gereed op fixtures. Echte GitLab-cijfers meet Dennis lokaal.
- **Gedaan:** `depgraph inventory` pagineert `GET /groups/:id/projects` (`include_subgroups`, `archived=false`), respecteert `per_page`, `max_concurrency` en `Retry-After` op 429, en schrijft `inventory.csv`. Het token komt uit `GITLAB_TOKEN` of `glab config get token --host <host>` en wordt niet gelogd of opgeslagen. `depgraph validate-catalog` gebruikt het volledige schema en eist dat `dependsOn` binnen de meegegeven bestanden oplost. `depgraph resolve-env` gebruikt `config/environments.yaml`. De pilot is alle projecten in de geconfigureerde groepen. `depgraph propose-catalog` schrijft lokaal een draft; `--open-mr` opent een merge request en merged niet.
- **Tests:** contracttests tegen opgenomen GitLab-fixtures, inclusief paginatie, 429, en het glab-pad.
- **Afwijkingen en ADR:** geen nieuwe ADR. ADR-004 (pilot, geen cluster-aanroep) en ADR-001 (glab, geen libraries) zijn de basis.

## Lokaal draaien tegen self-managed GitLab

```sh
glab auth login --hostname gitlab.example.com
# of: export GITLAB_TOKEN=...   (depgraph slaat het token niet op)
# pas config/config.yaml aan: gitlab.base_url en gitlab.groups
depgraph inventory --output inventory.csv
depgraph resolve-env production
depgraph validate-catalog path/naar/catalog-info.yaml
depgraph propose-catalog --from ./checkout --output ./proposed-catalog
# optioneel, opent MR's en merged nooit:
depgraph propose-catalog --from ./checkout --open-mr --project group/project
```

Vervang `gitlab.example.com` en `group/project` door de echte host en het echte projectpad. De groeps-paden van het team zijn nog een open vraag.

## Nog niet gestart

- **F0-2:** geblokkeerd op tokens van een mens. Geen echt token in deze repo of in de bot.
