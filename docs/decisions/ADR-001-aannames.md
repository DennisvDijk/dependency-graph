# ADR-001: Standaardaannames voor de open vragen

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Vastgelegd binnen de werkregels van sectie 14: eerst de open vragen, daarna pas fase 0.

## Context

Het plan laat zeventien vragen open die de vorm van collectors, config en de graaf bepalen. Deze run bouwt geen fase 1. Elke vraag krijgt hier de status `standaardaanname` met het afgesproken default, zodat F0-1 niet blokkeert. Een aanname is geen meting. Wat een mens nog moet invullen staat onderaan, samen met de punten die we verifiëren vóór we er logica op bouwen.

Afwijkingen van de taal en de repo-locatie staan in [ADR-002](ADR-002-rust-en-github.md).

## Besluiten

### 1. GitLab self-managed of gitlab.com, en welke versie

- Status: standaardaanname
- Besluit: self-managed, recente versie. De exacte versie wordt later gelezen met `GET /api/v4/version`.
- Gevolg: de client praat tegen de API v4 van een eigen instance. `config.yaml` gebruikt `https://gitlab.example.com` tot een mens de echte origin bevestigt. Geen aanname over gitlab.com-specifieke features.

### 2. Welke GitLab-groepen en subgroepen in scope

- Status: standaardaanname
- Besluit: alleen de groepen van de pilotteams.
- Gevolg: `gitlab.groups` blijft een placeholder (`example-group/pilot`) tot de concrete paden bekend zijn. De collector mag niet "alle groepen van de instance" pakken.

### 3. Kubernetes? Welke clusters en namespaces per omgeving

- Status: standaardaanname
- Besluit: ja. Eén cluster, één namespace per omgeving.
- Gevolg: `kubernetes.clusters` noemt precies het env-var-veld `kubeconfig_env`, nooit de kubeconfig zelf. `k8s_namespaces` is leeg tot de namen vaststaan. Een namespace mag niet aan twee canonieke omgevingen hangen.

### 4. GitOps (Argo CD/Flux) of deploys via GitLab CI

- Status: standaardaanname
- Besluit: GitLab CI met `environment:`. Kubernetes is de controle, niet de bron van de deploy-intentie.
- Gevolg: deploy-nodes komen uit GitLab environments. Workloads in het cluster bevestigen ze. Argo CD en Flux worden niet uitgelezen in fase 1-2.

### 5. OpenTelemetry-instrumentatie en tracedestinatie

- Status: standaardaanname
- Besluit: onbekend. Fase 1 start met meten van de dekking, niet met een aangenomen backend.
- Gevolg: `runtime_source` staat op `none`. Er worden geen runtime-edges verzonnen. De vraag blijft open tot een mens de instrumentatie en de bestemming aanwijst.

### 6. Service mesh (Istio) of Cilium/Hubble

- Status: standaardaanname
- Besluit: geen mesh.
- Gevolg: geen Istio- of Hubble-collector in fase 1-2. `runtime_source` heeft de waarden wel, zodat een correctie geen config-schema-breuk is. Hubble-flowcontext blijft ongelezen tot de Cilium-versie bekend is.

### 7. Waar SBOMs staan, welk formaat, gelabeld met digest, sha of tag

- Status: standaardaanname
- Besluit: CycloneDX JSON als artifact van een GitLab-job.
- Gevolg: `sbom.sources` is leeg in de voorbeeldconfig. De collector zoekt later job-artifacts, niet een aparte SBOM-server, tot een mens dat wijzigt. Library-koppeling wacht op vraag 8.

### 8. Interne package-prefixes

- Status: standaardaanname
- Besluit: leeg. Geen `Library`-nodes tot de prefixes bekend zijn.
- Gevolg: `internal_package_prefixes` en `sdk_hints.yaml` blijven leeg. Een SBOM mag nog geen library-edge promoveren alleen omdat een purl bestaat.

### 9. Canonieke omgevingsnamen en mapping

- Status: standaardaanname
- Besluit: `dev`, `acc`, `prod`.
- Gevolg: `environments.yaml` mapt GitLab `development` en OTel `dev`/`development` op `dev`; GitLab `acceptance`/`staging` en OTel `acc`/`staging` op `acc`; GitLab `production` en OTel `prod`/`production` op `prod`. Elke bronnaam hoort bij precies één canonieke omgeving. Kubernetes-namespaces zijn leeg.

### 10. Async systemen (Kafka, RabbitMQ, NATS)

- Status: standaardaanname
- Besluit: geen in fase 1-2.
- Gevolg: het model kent `publishes_to` en `consumes_from` al, maar er is geen collector voor een broker. Resource-kinds `topic` en `queue` blijven geldig voor handmatig vastgelegde resources.

### 11. Hoe database- en external-API-afhankelijkheden worden vastgelegd

- Status: standaardaanname
- Besluit: `kind: Resource` in een centrale repo.
- Gevolg: resource-ids zijn `resource:<kind>:<naam>` met kind `database`, `topic`, `queue`, `external-api`, `model-endpoint`, `identity-provider` of `other`. De centrale repo zelf is nog niet gekozen; F0-1 leest hem niet.

### 12. Paden van API-contracten

- Status: standaardaanname
- Besluit: `openapi/**`, `**/*.proto`, `schema/**`.
- Gevolg: `contract_paths` in `config.yaml` bevat exact die drie globs. Een statische collector mag later alleen onder deze paden zoeken.

### 13. Kunnen CI-runners Prometheus, Tempo, de Kubernetes-API en het SBOM-systeem bereiken

- Status: standaardaanname
- Besluit: ja. Zo niet, dan een dedicated runner.
- Gevolg: de GitLab-pipeline gaat ervan uit dat de scheduled runner de bronnen kan bereiken. F0-1 test die bereikbaarheid niet en gebruikt geen echte endpoints. Tokens blijven uit deze repo.

### 14. Eigenaarsteam en reviewers

- Status: standaardaanname
- Besluit: het platformteam.
- Gevolg: `CODEOWNERS` noemt `@platform-team` als placeholder. Een mens vervangt dat door de echte GitHub-teamslug. Tot die tijd is het platformteam de reviewende partij in de documentatie.

### 15. Welke 5-10 pilotapplicaties

- Status: standaardaanname
- Besluit: nog te kiezen met de teams (`TBD`).
- Gevolg: `pilot.yaml` heeft `apps: []`. Er wordt geen applicatie geïmporteerd "omdat die voor de hand ligt".

### 16. Goedgekeurd platform voor de uitvoerende bot, en welke data daarheen mag

- Status: standaardaanname
- Besluit: geen tokens en geen echte data naar de bot.
- Gevolg: fixtures only. Config bevat `example.com`. Collectors in F0-1 openen geen socket. Logs en `meta.json` redigeren credential-URLs en bekende tokenvormen. Het platform zelf (welke bot-omgeving) is hiermee niet gekozen; alleen de datagrens staat vast.

### 17. Bestaande labelconventie op workloads

- Status: standaardaanname
- Besluit: `app.kubernetes.io/name`.
- Gevolg: de Kubernetes-collector, zodra die bestaat, leest die label als applicatienaam. Afwijkende labels worden niet geraden.

## Vragen die een mens nog moet beantwoorden

De standaardaanname is genoeg om F0-1 te bouwen. Deze punten blijven een antwoord van een mens nodig hebben voordat een latere fase echte bronnen leest:

1. Bevestig self-managed versus gitlab.com, en noteer de versie uit `GET /api/v4/version`.
2. Welke concrete groepen en subgroepen van de pilotteams vallen in scope?
5. Is er OpenTelemetry-instrumentatie, en waar landen de traces?
6. Klopt het dat er geen Istio en geen Cilium/Hubble is?
8. Welke interne package-prefixes horen bij `Library`-nodes?
14. Bevestig het platformteam en de echte GitHub-teamslug in `CODEOWNERS`.
15. Welke 5 tot 10 pilotapplicaties doen mee?
16. Welk platform is goedgekeurd voor de uitvoerende bot? De datagrens staat al vast: geen tokens en geen echte data.

Vragen 3, 4, 7, 9, 10, 11, 12, 13 en 17 hebben een werkbare aanname. Een mens corrigeert ze als de werkelijkheid anders is; F0-1 codeert ze als config, niet als verborgen constanten in de collectors.

## Verifieer vóór gebruik

Niet gebruiken als feit tot iemand de bron heeft gelezen:

- Tempo-metriek- en labelnamen (`traces_service_graph_*`) en de omgevingsdimensie.
- Hubble-flowcontext per Cilium-versie.
- GitLab-API-parameters en de minimale rol per endpoint.
- Of er een Cartography-module voor GitLab bestaat.
- Dependency-Track-API-rechten en -endpoints.

F0-1 bevat geen client voor deze systemen. Een latere collector die een naam uit deze lijst hard codeert zonder verificatie is een afwijking van dit besluit.
