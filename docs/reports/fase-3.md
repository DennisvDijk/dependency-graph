# Fase 3

## F3-1 t/m F3-7

- **Taak:** analyse op petgraph, CLI, MR-check, Pages, drift-issue, MCP met OAuth 2.1, teamdocs
- **Status:** gereed op fixtures. Echte tellingen zijn voor Dennis.
- **Gedaan:** `dependencies`, `consumers`, `impact`, `deploy_order`, `cycles`, `critical_nodes`, `orphans` en `diff` lopen op petgraph. Alleen actieve edges tellen. `A -> B` betekent dat A van B afhangt; consumers zijn de ancestors. `deploy_order` sorteert `declared_depends_on` en `observed_calls` die niet `soft` zijn, en rapporteert cycles. `library_impact` antwoordt `libraries disabled`. De CLI-commando's `show`, `deps`, `consumers`, `impact`, `deploy-order`, `cycles`, `library-impact`, `drift`, `critical`, `orphans`, `diff` en `at` nemen `--env`, `--format table|json|mermaid` en `--snapshot`. `ci/depgraph-mr-check.yml` is een include met `allow_failure: true`. `depgraph mr-check` lost het project op via aliases, noemt prod-consumers, ziet contractwijzigingen via `contract_paths`, en werkt één MR-notitie bij. `--dry-run` print alleen. De site is één HTML-bestand met Cytoscape.js, filters voor env, status, owner en system, zoeken op naam of alias, en een detailpaneel. `publish drift-issue` maakt of werkt één open issue met label `depgraph-drift` bij. `serve-mcp` is de read-only MCP-server uit [ADR-005](../decisions/ADR-005-mcp-server.md), met Protected Resource Metadata, JWT-validatie tegen JWKS, audience-binding en scope `depgraph:read`. `component_context` geeft projectpad, URL, branch, catalogus, manifests, owner, deps en consumers. Het token gaat niet door naar een bronsysteem. `--insecure-local` mag alleen op loopback. Een distroless Dockerfile staat in de repo.
- **Tests:** 90 geslaagd (`cargo test --all-targets --all-features`), inclusief een cycle, een lege graaf, een onbekende node, de MR-notitie, de drift-issue en een lokaal JWKS.
- **Open vragen:** de GitLab-versie via `GET /api/v4/version`. De IdP-issuer en audience vult Dennis in.
- **Volgende stap:** Dennis meet de punten hieronder en draait `serve-mcp` lokaal.

## Lokaal MCP

```sh
depgraph serve-mcp --insecure-local --snapshot data/snapshot.json
```

Dat bindt aan `mcp.bind` (standaard `127.0.0.1:8080`) en print een waarschuwing. Auth staat uit. Een publiek adres wordt geweigerd.

Met een IdP: zet `mcp.issuer`, `mcp.audience` en `mcp.jwks_url`, en laat `--insecure-local` weg. Clients sturen `Authorization: Bearer` met scope `depgraph:read`. Zie [mcp.md](../mcp.md).

## Te meten door Dennis

- Of `deploy-order` op de echte prod-graaf een cycle rapporteert.
- Of de top 20 van `critical` de componenten zijn die het team als kritiek ziet.
- Of de MR-notitie op een echte merge request één keer blijft en wordt bijgewerkt.
- Of de wekelijkse drift-issue één open issue met label `depgraph-drift` blijft.
- Of een agent via `component_context` het juiste GitLab-project opent.
- Of tokens van de IdP door de JWKS-check komen en een token zonder `depgraph:read` wordt geweigerd.
