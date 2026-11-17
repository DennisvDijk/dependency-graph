# ADR-015: Geen formele ontologie en geen blueprints

- Status: besluit
- Datum: 2026-11-17
- Besluitvormers: Dennis (eigenaar).

## Context

De vraag kwam op of depgraph een formele ontologie (RDF, OWL, SHACL) of "blueprints"
(door teams zelf gedefinieerde entiteittypes, zoals in ontwikkelaarsportalen) moet
gebruiken. Tot nu toe was dit niet vastgelegd. Het model is wel een ontologie in
gewone zin: een vaste lijst van dingen (nodes) en relaties (edges), beschreven in
`docs/naming.md` en `src/models.rs`.

## Besluit

1. Het model blijft een klein, gesloten vocabulaire in Rust. `schemas/graph.schema.json` wordt eruit gegenereerd.
2. Geen RDF, OWL of SHACL. Impactanalyse is graaf-traversal, geen inferentie, en er is geen afnemer die RDF vraagt.
3. Geen blueprints. De belofte van depgraph is geen configuratie per service. Een gesloten vocabulaire houdt edges vergelijkbaar tussen teams.
4. De Backstage-entiteiten uit `catalog-info.yaml` zijn invoer, geen eigen schema.

## Wanneer opnieuw bekijken

- Een echte behoefte die de gesloten lijst niet dekt, bijvoorbeeld eigenaren of API-contracten als eigen nodes.
- Een andere tool die de graaf in een standaardformaat wil lezen. De eerste stap is dan een export, geen nieuw intern model.

## Gevolgen

- Nieuwe node- of edgetypes vragen een codewijziging, een schema-update en een regel in `docs/naming.md`.
- Zie [architecture.md](../architecture.md) voor de lagen en methodes.
