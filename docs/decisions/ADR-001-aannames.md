# ADR-001: Standaardaannames voor de open vragen

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Vastgelegd binnen de werkregels van sectie 14: eerst de open vragen, daarna pas fase 0.

## Context

Het plan laat zeventien vragen open die de vorm van collectors, config en de graaf bepalen. Elke vraag begon als `standaardaanname`. Dennis heeft op 2026-11-16 een deel beantwoord; die punten staan op `beantwoord`. Wat hij niet invulde, blijft `standaardaanname`. Wat nog een mens nodig heeft, staat onderaan.

Afwijkingen van de taal en de repo-locatie staan in [ADR-002](ADR-002-rust-en-github.md). De runtimebron staat in [ADR-003](ADR-003-runtime-bron.md), de pilot en Datadog in [ADR-004](ADR-004-pilot-en-datadog.md), de MCP-server in [ADR-005](ADR-005-mcp-server.md).

## Besluiten

### 1. GitLab self-managed of gitlab.com, en welke versie

- Status: beantwoord
- Besluit: self-managed. Lokaal is `glab` het toegangspad: de host- en auth-configuratie van `glab`, niet een token in deze repo. Collectors mogen de REST API rechtstreeks aanroepen; voor een lokale run is de `glab`-config het verwachte pad.
- Gevolg: `config.yaml` houdt `https://gitlab.example.com` tot de echte origin bekend is. De versie is nog niet genoteerd. Een mens leest die nog via `GET /api/v4/version`. Geen gitlab.com-specifieke features.

### 2. Welke GitLab-groepen en subgroepen in scope

- Status: beantwoord
- Besluit: vervangen door [ADR-006](ADR-006-volledige-scan.md). Standaard ziet depgraph elk project dat het token mag zien. `gitlab.groups` vernauwt optioneel, recursief, op een groep of subgroep (`platform`, `org/infra/platform`). `gitlab.projects` voegt losse projectpaden toe (`org/other-team/ingress-gateway`).
- Gevolg: de voorbeeldconfig laat beide lijsten leeg. Filters (`exclude_forks`, `exclude_archived`, paden, topics, activiteit, visibility) gelden na de lijst. Expliciete projecten slaan die filters over, behalve archief.

### 3. Kubernetes? Welke clusters en namespaces per omgeving

- Status: standaardaanname
- Besluit: ja. Eén cluster, één namespace per omgeving.
- Gevolg: `kubernetes.clusters` noemt precies het env-var-veld `kubeconfig_env`, nooit de kubeconfig zelf. `k8s_namespaces` is leeg tot de namen vaststaan. Een namespace mag niet aan twee canonieke omgevingen hangen.

### 4. GitOps (Argo CD/Flux) of deploys via GitLab CI

- Status: standaardaanname
- Besluit: GitLab CI met `environment:`. Kubernetes is de controle, niet de bron van de deploy-intentie.
- Gevolg: deploy-nodes komen uit GitLab environments. Workloads in het cluster bevestigen ze. Argo CD en Flux worden niet uitgelezen in fase 1-2.

### 5. OpenTelemetry-instrumentatie en tracedestinatie

- Status: beantwoord
- Besluit: OpenTelemetry is aanwezig. Langfuse doet daarnaast LLM-tracing. Welke runtimebron de graaf voedt, staat in vraag 6 en in [ADR-003](ADR-003-runtime-bron.md).
- Gevolg: Langfuse is geen fase-1-bron. Het kan later edges naar `resource:model-endpoint:…` voeden. Er is nog geen Langfuse-collector en geen Langfuse-config.

### 6. Service mesh (Istio) of Cilium/Hubble

- Status: beantwoord
- Besluit: Istio is aanwezig. De aanname "geen mesh" vervalt. `runtime_source` is `istio`. Tempo is optioneel. Uitwerking: [ADR-003](ADR-003-runtime-bron.md).
- Gevolg: Hubble is niet de gekozen mesh. De voorbeeldconfig zet `runtime_source: istio` en `tempo.enabled: false`. De runtime-collector blijft een stub en opent geen socket.

### 7. Waar SBOMs staan, welk formaat, gelabeld met digest, sha of tag

- Status: standaardaanname
- Besluit: CycloneDX JSON als artifact van een GitLab-job.
- Gevolg: `sbom.sources` is leeg in de voorbeeldconfig. De collector zoekt later job-artifacts, niet een aparte SBOM-server, tot een mens dat wijzigt. Library-koppeling doet deze SBOM niet: vraag 8 zet `Library`-nodes uit.

### 8. Interne package-prefixes

- Status: beantwoord
- Besluit: er zijn geen interne libraries. Het landschap is Kubernetes, infrastructuur, firewalling en aanverwante platformonderdelen. Geen `Library`-nodes en geen `uses_library`-edges in scope.
- Gevolg: de types blijven in het model staan voor later gebruik. `libraries.enabled` staat default op `false`, `internal_package_prefixes` blijft leeg, en `sdk_hints.yaml` blijft leeg. Zolang `libraries.enabled` uit staat, mag een collector die nodes en edges niet aanmaken. Een niet-lege prefixlijst bij `enabled: false` is een configfout.

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

- Status: beantwoord
- Besluit: het AI Platform-team. De code leeft uiteindelijk op GitLab. Deze GitHub-repo is voorlopig de bouwplaats; `.gitlab-ci.yml` blijft de pipeline voor de uitvoering daar.
- Gevolg: `CODEOWNERS` noemt `@ai-platform` als placeholder-slug, met die toelichting. De teamnaam staat vast; de slug is alleen de GitHub-plaatsvervanger.

### 15. Welke 5-10 pilotapplicaties

- Status: beantwoord
- Besluit: niet een shortlist van 5 tot 10. De pilot is alle applicaties van Dennis' team. Uitwerking, inclusief Datadog als optionele read-only statusbron: [ADR-004](ADR-004-pilot-en-datadog.md).
- Gevolg: `pilot.yaml` houdt `apps: []`. Dat is geen lege selectie. De scope is elke app in de groepen uit vraag 2. Verzamelen leest de repo's, niet het cluster.

### 16. Goedgekeurd platform voor de uitvoerende bot, en welke data daarheen mag

- Status: beantwoord
- Besluit: het product draait lokaal of eventueel op EKS Auto Mode, en moet aanspreekbaar zijn als MCP-server met OAuth 2.1. Dat tilt plantaak F3-6 naar een kernvereiste. Uitwerking: [ADR-005](ADR-005-mcp-server.md). De datagrens voor de bot die de tool bouwt blijft: geen tokens en geen echte data.
- Gevolg: `depgraph serve-mcp` en de module `mcp` zijn stubs zonder OAuth. Config blijft `example.com`. Collectors in F0-1 openen geen socket. Logs en `meta.json` redigeren credential-URLs en bekende tokenvormen.

### 17. Bestaande labelconventie op workloads

- Status: standaardaanname
- Besluit: `app.kubernetes.io/name`.
- Gevolg: de Kubernetes-collector, zodra die bestaat, leest die label als applicatienaam. Afwijkende labels worden niet geraden.

## Vragen die een mens nog moet beantwoorden

1. De exacte GitLab-versie, via `GET /api/v4/version`. Self-managed en het `glab`-pad staan vast.

Groepspaden zijn geen blokkade meer. Zie ADR-006.

Vragen 3, 4, 7, 9, 10, 11, 12, 13 en 17 blijven `standaardaanname`. Een mens corrigeert ze als de werkelijkheid anders is.

## Verifieer vóór gebruik

Niet gebruiken als feit tot iemand de bron heeft gelezen:

- Tempo-metriek- en labelnamen (`traces_service_graph_*`) en de omgevingsdimensie, pas als `tempo.enabled` aan gaat.
- GitLab-API-parameters en de minimale rol per endpoint.
- Of er een Cartography-module voor GitLab bestaat.
- Dependency-Track-API-rechten en -endpoints, alleen als er toch een SBOM-server bij komt. De default is een CycloneDX-artifact van een GitLab-job, en `Library`-nodes staan uit.

Hubble-flowcontext is niet meer van toepassing: Istio is de mesh ([ADR-003](ADR-003-runtime-bron.md)). `istio_requests_total` met `reporter="destination"` is het besluit van Dennis, geen open meetvraag.

F0-1 bevat geen client voor deze systemen. Een latere collector die een naam uit deze lijst hard codeert zonder verificatie is een afwijking van dit besluit.
