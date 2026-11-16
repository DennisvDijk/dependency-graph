# ADR-005: depgraph is een MCP-server met OAuth 2.1

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Beantwoordt vraag 16 uit [ADR-001](ADR-001-aannames.md) en tilt plantaak F3-6 naar een kernvereiste.

## Context

Het product moet lokaal draaien, of eventueel op EKS Auto Mode, en aanspreekbaar zijn als MCP-server. De bot die de code bouwt, krijgt nog steeds geen tokens en geen echte data. OAuth hoort bij het product, niet bij deze bouwfase.

F3-6 beschreef een read-only MCP-server `depgraph-mcp` boven de analysebibliotheek. Dat is geen latere extra meer. Het is een eis aan het gereedschap.

## Besluit

`depgraph serve-mcp` start de server. In deze fase is dat een stub: het commando eindigt met `not implemented in F0-1` en doet geen OAuth.

Wanneer de server echt wordt gebouwd, gelden deze eisen:

- Transport: Streamable HTTP, volgens de huidige MCP-specificatie.
- depgraph is een OAuth 2.1 resource server.
- Protected Resource Metadata volgens RFC 9728.
- Tokens zijn audience-bound. Resource indicators volgens RFC 8707.
- PKCE blijft bij de authorization server. depgraph implementeert geen PKCE.
- JWT's worden gevalideerd tegen de JWKS van een externe identity provider.
- Alleen read-only tools. Geen tool schrijft naar GitLab, Kubernetes, Prometheus, Datadog of een ander bronsysteem.

De toollijst, al vastgelegd in `src/mcp` en nog zonder handler:

- `show`
- `deps`
- `consumers`
- `impact`
- `deploy_order`
- `cycles`
- `library_impact`
- `drift`
- `critical_nodes`
- `orphans`
- `diff`

Deploymentdoelen:

- Lokaal: één binary of een container.
- EKS Auto Mode: een containerimage. Helm of platte manifests komen later, niet in deze fase.

OAuth en de HTTP-transportlaag zijn gepland bij fase 3, of eerder als Dennis dat wil. F0-1 levert alleen het commando, de module en deze lijst.

## Gevolgen

- Analysefuncties blijven de plek waar een tool zijn antwoord vandaan haalt. De MCP-laag voegt geen tweede graaflogica toe.
- `library_impact` blijft in de lijst staan ook al staan `Library`-nodes uit ([ADR-001](ADR-001-aannames.md), vraag 8). De tool kan later "niets in scope" antwoorden zonder een tweede servercontract.
- Er staat geen client-secret, geen IdP-config en geen JWT in deze repo.
