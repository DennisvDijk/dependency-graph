# ADR-003: Runtimebron is Istio; Tempo is optioneel

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Beantwoordt vragen 5 en 6 uit [ADR-001](ADR-001-aannames.md).

## Context

De standaardaanname was: geen mesh, en OpenTelemetry onbekend. Dennis corrigeert dat. OpenTelemetry is aanwezig. Langfuse traceert LLM-aanroepen. Istio is aanwezig. De context die de graaf nodig heeft, staat in de GitLab-repo's. Gedeclareerde data is daarom de primaire bron. Wat Istio meet, vult die graaf aan.

## Besluit

1. `runtime_source` is `istio`.
2. De metriek is `istio_requests_total` met `reporter="destination"`.
3. Tempo is optioneel. In de voorbeeldconfig staat `tempo.enabled: false` en een `example.com`-URL. De namen `traces_service_graph_*` en de omgevingsdimensie blijven "verifieer vóór gebruik" tot Tempo aan gaat.
4. Primaire bron is gedeclareerde data in de repo's: `catalog-info.yaml`, manifests, Helm en kustomize, en GitLab CI-jobs met `environment:`.
5. Geobserveerde edges uit Istio zijn complementair. Ze vervangen de gedeclareerde graaf niet.
6. Langfuse is een mogelijke latere bron voor edges naar `resource:model-endpoint:<naam>`. Niet in fase 1-2. Geen collector en geen configblok in deze fase.
7. Voorstel, niet geïmplementeerd. De kind-lijst van resources is te applicatie-centrisch voor dit landschap. Firewallregels, network policies, ingresses en infra-componenten uit de repo's (manifests, Helm, Terraform en andere IaC, Istio-config) zijn belangrijke `Resource`-bronnen. Voorgestelde kinds, nog niet in de parser: `network-policy`, `firewall-rule`, `infra`.

## Gevolgen

- Hubble en Cilium zijn niet de mesh. Er komt geen Hubble-collector tenzij dit besluit wijzigt.
- Jaeger is niet gekozen.
- De runtime-collector was in deze ADR nog een stub. [ADR-008](ADR-008-runtime-istio.md) bouwt hem: read-only Prometheus, namespace naar env, en een `[1d]`-terugval als de retentie korter is dan 7 dagen.
- De Kubernetes-API is niet de manier waarop de pilot gedeclareerde workloads leest. Die staan in git. Zie [ADR-004](ADR-004-pilot-en-datadog.md).
