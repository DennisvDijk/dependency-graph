# ADR-016: API- en eventcontracten als eigen node

Status: geaccepteerd (slice 1, 2 en 3)

## Besluit

Een OpenAPI-, AsyncAPI- of proto-bestand in een repo wordt een node `resource:contract:<naam>`.

- `provides_contract` loopt van de component naar het contract. De component is de enige component van de repo; zijn er meer, dan blijft de edge weg en komt het contract in `unresolved.json`. Status `declared_only`.
- `uses_contract` loopt van de gebruiker naar het contract. Een proto-import is `declared_only`. Een gebruik dat alleen volgt uit een bekende aanroep van de aanbieder is `candidate`. AsyncAPI krijgt geen afgeleid gebruik.
- Impact loopt van contract naar gebruiker; `provides_contract` wordt omgekeerd bij het lopen, zoals `publishes_to`.
- Namen komen uit de titel van de spec, of uit de repo-naam als de titel algemeen is, met het achtervoegsel `-openapi`, `-asyncapi` of `-proto`.
- Het vocabulaire blijft gesloten en in Rust gedefinieerd (ADR-015).

## Gevolgen

De parser is regel- en sleutelgebaseerd en leest per project maximaal 50 bestanden. Het is een heuristiek, geen volledige spec-validatie.

## Slice 2: diff

`depgraph contract-diff` vergelijkt alleen operaties: een verwijderde operatie (of verwijderd bestand) is breaking, een toegevoegde niet. Velden, types en verplichte parameters worden niet gezien. De MR-check gebruikt dit als de basisversie beschikbaar is; dan vraagt alleen een breaking wijziging met geobserveerde prod-consumers om goedkeuring. Zonder basisversie geldt het oude gedrag (elk contractbestand). Bij een afgekapte operatielijst (500) wordt niets beweerd.

## Slice 3: events

Een AsyncAPI-bestand zegt wat zijn eigen service met elk kanaal doet. Versie 2: `publish` betekent dat anderen naar de service publiceren, dus de service consumeert; `subscribe` betekent dat zij verstuurt. Versie 3: `action: send` of `receive`. Elk kanaal wordt een `topic`-node met `publishes_to` of `consumes_from` (status `candidate`: een spec kan verouderd zijn), dezelfde edges als de broncode-lezer. Zo bereikt een wijziging in een event de consumers, en `impact` toont de eigenaren om te informeren. AsyncAPI krijgt geen afgeleide `uses_contract`. Een kanaal dat in de infra als queue staat krijgt een topic-node naast de queue-node; de naam wordt niet geraden.
