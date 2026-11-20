# ADR-016: API- en eventcontracten als eigen node

Status: geaccepteerd (slice 1 van 3)

## Besluit

Een OpenAPI-, AsyncAPI- of proto-bestand in een repo wordt een node `resource:contract:<naam>`.

- `provides_contract` loopt van de component naar het contract. De component is de enige component van de repo; zijn er meer, dan blijft de edge weg en komt het contract in `unresolved.json`. Status `declared_only`.
- `uses_contract` loopt van de gebruiker naar het contract. Een proto-import is `declared_only`. Een gebruik dat alleen volgt uit een bekende aanroep van de aanbieder is `candidate`. AsyncAPI krijgt geen afgeleid gebruik.
- Impact loopt van contract naar gebruiker; `provides_contract` wordt omgekeerd bij het lopen, zoals `publishes_to`.
- Namen komen uit de titel van de spec, of uit de repo-naam als de titel algemeen is, met het achtervoegsel `-openapi`, `-asyncapi` of `-proto`.
- Het vocabulaire blijft gesloten en in Rust gedefinieerd (ADR-015).

## Gevolgen

De parser is regel- en sleutelgebaseerd en leest per project maximaal 50 bestanden. Het is een heuristiek, geen volledige spec-validatie.

## Volgende slices

2. `depgraph contract-diff` en de MR-check: breaking changes tussen twee spec-versies.
3. Events: AsyncAPI send/receive gekoppeld aan queue- en topic-nodes, met de te informeren eigenaren.
