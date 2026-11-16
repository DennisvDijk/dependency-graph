# ADR-008: Istio-queries, namespaces en retentie

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Werkt [ADR-003](ADR-003-runtime-bron.md) uit. De stub-zin in ADR-003 geldt niet meer voor de runtime-collector.

## Context

Fase 1 leest gedeclareerde data uit git. Fase 2 vult die graaf aan met wat Istio in Prometheus meet. Dennis wil die collector nu, ook al zijn de echte tellingen uit fase 1 nog niet binnen. Tempo blijft uit.

## Besluit

1. De runtimebron blijft Istio. De query is `istio_requests_total` met `reporter="destination"`, gegroepeerd op source- en destination-workload en hun namespace. `request_protocol` levert `protocol`.
2. Fouten komen uit dezelfde metriek met `response_code=~"5.."`. `error_rate_7d` is dat aantal gedeeld door het totaal. `p95_ms_7d` komt uit `histogram_quantile(0.95, … istio_request_duration_milliseconds_bucket)`.
3. Het venster is `[7d]`, behalve als `prometheus.retention_days` lager is dan `window_days`. Dan is de query `[1d]`. `last_seen` is de timestamp van het Prometheus-sample. Het dagtotaal staat op de edge als `calls_total`, niet als `calls_7d`. Deze fase telt de samples van deze run bij elkaar op. Een vorige snapshot wordt niet meegenomen: de build blijft een pure functie van de raw records en de meegegeven tijd.
4. Een namespace mapt naar `dev`, `acc` of `prod` via `k8s_namespaces` in `environments.yaml`. Een onbekende namespace blijft onopgelost. Depgraph raadt hem niet.
5. Een serie wordt geen edge als `request_path` of `path` in `prometheus.health_paths` staat, of als `source_workload` in `prometheus.test_clients` staat.
6. Prometheus wordt alleen gelezen, met `GET /api/v1/query`. Een token hoort in de env-var die `prometheus.token_env` noemt, nooit in de config of in een foutmelding.
7. Tempo blijft `enabled: false`. Er is geen Tempo-query in deze fase.

## Gevolgen

- De gedeclareerde graaf blijft de basis. Istio bevestigt of vult aan. Zie ADR-003, punt 4 en 5.
- `depgraph collect --prometheus` praat met de geconfigureerde URL. `--prometheus-fixture` leest opgenomen antwoorden en opent geen socket.
