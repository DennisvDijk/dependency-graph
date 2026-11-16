# Fase 4

## F4-1 t/m F4-8

- **Taak:** historie, scores, retentie, doctor, Entra als optie, init en MCP-config, releases, quickstart
- **Status:** gereed op fixtures. De geschreven faselijst stond niet in de repo; dit is de invulling van de resterende productgaten plus Dennis' adoptie-eis. Afwijking staat in de open vragen.
- **Gedaan:**
  - F4-1: `build` archiveert elke snapshot onder `runs/<generated_at>/`. `depgraph runs` toont ze.
  - F4-2: `store::run_at` kiest de nieuwste run op of vóór een tijdstip. `diff` blijft twee snapshotbestanden vergelijken.
  - F4-3: `confidence` op een edge, 0 tot 1, uit status en of er calls zijn. `show --format json` neemt het veld mee.
  - F4-4: `depgraph retain --now <tijd>` verwijdert runs ouder dan `retention_days`. De klok komt van `--now` of, in de CLI, van nu.
  - F4-5: `depgraph doctor` controleert config, glab (alleen exitcode), of een token beschikbaar is zonder het te tonen, en de leeftijd van de snapshot. `--online` doet een read-only `GET /api/v4/version`.
  - F4-6: [ADR-010](../decisions/ADR-010-entra-optioneel.md). `auth.provider: entra` is optioneel. Generic JWKS blijft de default. `--insecure-local` verandert niet.
  - F4-7: `depgraph init`, `depgraph mcp-config`, shell completions, voorbeelden in `--help`.
  - F4-8: release-workflow voor macOS arm64/x86_64 en Linux, `packaging/install.sh`, een Homebrew-formula, de bestaande containerimage, en [quickstart.md](../quickstart.md).
- **Tests:** 94 geslaagd (`cargo test --all-targets --all-features`). init, doctor en mcp-config tegen vastgelegde output. Entra-tokens zijn lokaal getekend, zonder netwerk.
- **Open vragen:** klopt deze lezing van F4-1..F4-8 met Dennis' geschreven plan? De GitLab-versie via `GET /api/v4/version`. De echte Entra tenant- en client-id.
- **Volgende stap:** Dennis volgt de quickstart en vult de Entra-placeholders pas in als de app-registratie klaar is.

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
