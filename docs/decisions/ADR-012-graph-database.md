# ADR-012: Graph database, nog niet

- Status: voorgesteld, wacht op metingen
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Nog geen keuze.

## Context

De graaf staat in JSON-snapshots. `GraphStore` is de enige deur. Een graph database is pas nodig als één van deze drie waar is:

1. Meer dan 50.000 actieve edges in één omgevingsview.
2. p95 van een query boven 2 seconden, na optimalisatie.
3. Meerdere teams willen ad-hoc queries (Cypher of vergelijkbaar).

Kandidaten, niet gekozen: Memgraph, Neo4j Community, Apache AGE op Postgres.

## Besluit

Nog geen database. `depgraph stats --snapshot <bestand>` print het aantal actieve edges en de p95 van een lokale impact-query in milliseconden. Die cijfers horen hier pas als Dennis ze op echte data invult.

| Meting | Drempel | Waarde |
| --- | --- | --- |
| Actieve edges in één omgeving | 50.000 | nog niet gemeten |
| p95 query | 2.000 ms | nog niet gemeten |
| Teams die ad-hoc queries willen | meer dan het pilotteam | nog niet gemeten |

## Gevolgen

- Migratie is een nieuwe `GraphStore`-adapter, geen herschrijving van de normalizer of de analyse.
- Tot een drempel gehaald is, blijft het bestand de bron.
