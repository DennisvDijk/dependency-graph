# ADR-014: Broncode lezen en berichtenverkeer

- Status: besluit
- Vervangt: ADR-001 vraag 10 (geen async systemen in fase 1-2)

## Besluit

depgraph leest applicatiebroncode (Python, Elixir, Rust, JavaScript, TypeScript, Go, Java, Kotlin, C#) op letterlijke namen, en volgt queues en topics als aparte knopen.

- **Wat wordt gelezen:** URL's met een hostnaam, databaseverbindingen (alleen de hostnaam) en letterlijke queue- en topicnamen bij `queue_url`, `topic_arn`, `topic` en vergelijkbare parameters, plus Kafka-consumers en -producers. Een naam die pas bij runtime ontstaat, wordt niet geraden.
- **Richting:** een queue of topic krijgt `publishes_to` of `consumes_from` alleen als de omringende code aangeeft of er geschreven of gelezen wordt. Zonder duidelijk werkwoord gebeurt er niets.
- **Bewijs:** alles is een kandidaat (`static`), nooit een waarneming.
- **Impact:** een `publishes_to`-edge wijst bij analyse van uitgever naar topic naar lezer. Een wijziging bij de uitgever raakt dus het topic en zijn consumers; een wijziging bij een consument raakt de uitgever niet.
- **Geen geheimen:** uit een verbindingsstring blijft alleen de hostnaam over.
- **Beperking:** per repository worden maximaal 100 bestanden gelezen, bestanden met integratiecode eerst. Tests, vendor-mappen en gegenereerde bestanden worden overgeslagen.

## Gevolg

Geen configuratie per service. Een repository die elders een andere naam heeft, wordt via `aliases.yaml` of de naam van de repository gevonden, zoals bij cloudbronnen.
