# Fase 4

## F4-1 t/m F4-8

Het plan staat letterlijk in [plan-fase-4.md](../plan-fase-4.md).

- **Taak:** classificatie, padscore, tijdreizen, tijdlijn, MR-check, twee beslissingen, scorecard. Adoptie-extra's horen erbij maar zijn geen F4-nummer.
- **Status:** gereed op fixtures. Uitrol voorbij de pilot wacht op Dennis.
- **Gedaan:**
  - F4-1: hard/soft uit `depgraph/soft-dependencies`, daarna een ingelezen chaos-fixture, daarna een heuristiek. `criticality` en `criticality_source`. Geen chaos-run.
  - F4-2: padscore = product van call-aandeel maal hardheid (`scoring` in config). Zichtbaar in impact, critical en MCP. `confidence` blijft ernaast ([ADR-011](../decisions/ADR-011-confidence-en-padscore.md)).
  - F4-3: `depgraph diff --from <run> --to <run>` en `depgraph at --date <date>` op `runs/`.
  - F4-4: `depgraph timeline --env prod --since <time>` uit deployment- en incident-fixtures. Datadog wordt niet aangeroepen.
  - F4-5: blokkerende MR-check bestaat en staat uit (`mr_check.blocking: false`). Aanzetten is een goedkeuringspunt. Override-label `depgraph-override`.
  - F4-6: [ADR-012](../decisions/ADR-012-graph-database.md) voorgesteld, wacht op metingen. `depgraph stats` print edges en query-p95.
  - F4-7: [ADR-013](../decisions/ADR-013-portaal.md) voorgesteld. Geen portaal. `catalog-info.yaml` blijft de route.
  - F4-8: `depgraph scorecard`, `scorecard.html` en MCP-tool `scorecard`. Verbeterhulp, geen blaam.
  - Adoptie: init, doctor, mcp-config, completions, releases, quickstart, historie en retentie, optionele Entra ([ADR-010](../decisions/ADR-010-entra-optioneel.md)).
- **Tests:** 100 geslaagd (`cargo test --all-targets --all-features`). Classificatie, padscore, tijdlijn, scorecard, init, doctor, mcp-config, Entra-tokens zonder netwerk.
- **Open vragen:** de GitLab-versie via `GET /api/v4/version`. Of een team de blokkerende MR-check aanzet. De metingen voor ADR-012. Of een team een portaal wil dragen. De echte Entra-ids.
- **Volgende stap:** Dennis meet stats op een echte snapshot en beslist over de MR-check en het portaal.

## Eerste run voor een ontwikkelaar

```sh
git clone https://github.com/DennisvDijk/dependency-graph.git
cd dependency-graph
cargo install --path . --locked
depgraph init
depgraph doctor
depgraph show payments-api --snapshot tests/fixtures/phase1/graph.json
depgraph serve-mcp --insecure-local --snapshot tests/fixtures/phase1/graph.json
depgraph mcp-config cursor
```
