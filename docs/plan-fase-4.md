# Plan: fase 4

Dennis' faselijst, vertaald. De adoptie-extra's (init, doctor, mcp-config, Entra, releases) staan hier niet; die horen bij ADR-010 en `docs/reports/fase-4.md`.

- **F4-1 Hard/soft-classificatie.** Bronnen op volgorde van betrouwbaarheid: (1) expliciete verklaring in `depgraph/soft-dependencies` in `catalog-info.yaml`, (2) resultaten van fault-injection of chaos in dev of acc (afhankelijkheid uit, client blijft werken = soft), (3) heuristieken (fallback, circuit breaker, async). Leg `criticality` vast als `hard`, `soft` of `unknown` (default `unknown`) plus `criticality_source` (`declared`, `experiment`, `heuristic`). Een heuristiek is bewijs, geen oorzaak. Alleen lezen: het gereedschap neemt experimentresultaten uit een bestand of fixture op en draait zelf geen chaos.
- **F4-2 Gewogen edges.** `w(e) = calls(e) / som van alle uitgaande calls van de client`. Padscore = product van `w` langs het pad, maal een hardheidsfactor (hard 1.0, unknown 0.5, soft 0.2). De factoren zijn aannames in `config.yaml`. De score is voor rangschikking, niet als kans. Zichtbaar in impact, critical en MCP.
- **F4-3 Tijdreizen.** `depgraph diff --from <run> --to <run>` en `depgraph at --date <date>` op opgeslagen snapshots.
- **F4-4 Deployment-correlatie.** `depgraph timeline --env prod --since <time>`: deployments uit GitLab als tijdlijn naast fouten of incidenten. Datadog blijft optioneel en read-only (ADR-004). Tests gebruiken fixtures.
- **F4-5 Blokkerende MR-check.** Opt-in per repo. Een `contract_change` met geobserveerde prod-consumers vraagt extra goedkeuring of een override-label. Staat uit. Aanzetten vraagt expliciete teamgoedkeuring.
- **F4-6 Beslispunt graph database.** Een ADR met gemeten cijfers. Criteria (één van drie): meer dan 50.000 actieve edges in één omgevingsview; p95-querytijd boven 2 s na optimalisatie; meerdere teams willen ad-hoc queries (Cypher of vergelijkbaar). Kandidaten: Memgraph, Neo4j Community, Apache AGE op Postgres. `GraphStore` maakt migratie een adapter, geen herbouw. `depgraph stats` meet edge-aantal en query-p95. Status: voorgesteld, wacht op metingen.
- **F4-7 Beslispunt portaal (Backstage of vergelijkbaar).** Criteria: meerdere teams willen ownership, docs en scorecards in één UI naast de graaf; er is een team dat het portaal kan onderhouden. `catalog-info.yaml` houdt die route open. Resultaat is een ADR (voorgesteld).
- **F4-8 Datakwaliteit-scorecard per team.** Volledigheid van `catalog-info.yaml`, telemetrie aanwezig, geen onopgeloste namen. Als verbeterhulp, niet om te veroordelen. CLI, sitepagina en MCP-tool.

Uitrol voorbij de pilot alleen na expliciete goedkeuring van Dennis.
