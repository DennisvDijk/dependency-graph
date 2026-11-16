# Fase 2

## F2-1 t/m F2-8

- **Taak:** runtimebron, Istio-collector, identiteit, SBOM, normalizer v2, drift, dekking, checklist
- **Status:** gereed op fixtures. Dennis gaf expliciet groen licht om door te bouwen. De echte fase-1-tellingen (projecten, catalogusdekking, mismatches, unresolved, Istio-bevestiging, live kubectl) staan nog open in [fase-1](fase-1.md). Die gate is daarmee opzij gezet, niet gemeten.
- **Gedaan:** [ADR-008](../decisions/ADR-008-runtime-istio.md) legt Istio vast: `istio_requests_total` met `reporter="destination"`, gegroepeerd op workload en namespace, namespace naar env via `environments.yaml`, en `[1d]` plus `calls_total` en `last_seen` als Prometheus minder dan 7 dagen bewaart. `runtime_edges` leest Prometheus read-only of een fixture. Health checks en testclients vallen weg via config. Aliassen lossen namen op; onbekende namen gaan naar `unresolved.csv` met volume; onbekende namespaces worden niet geraden. Bestemmingen buiten de mesh (`PassthroughCluster`, externe hosts) worden `Resource`s met `inferred: true`. [ADR-009](../decisions/ADR-009-sbom-overgeslagen.md) slaat digest/sha/tag-koppeling over: er zijn geen interne libraries. De normalizer zet `both`, `observed_only`, `declared_only` en `candidate`, en laat edges verlopen op `last_seen` tegen de meegegeven tijd (`edge_expiry_days` 30, `retention_days` 90). `drift` schrijft per env een markdown. `meta.json` heeft `coverage`. VirtualService, DestinationRule, AuthorizationPolicy en NetworkPolicy leveren `candidate_calls`, nooit observed.
- **Tests:** 80 geslaagd (`cargo test --all-targets --all-features`), inclusief golden `tests/fixtures/phase2/graph.json` en een tweede merge met dezelfde bytes. De fase-1-golden blijft staan.
- **Open vragen:** de GitLab-versie via `GET /api/v4/version`. De echte Prometheus-retentie en de namespace-lijst vult Dennis in.
- **Volgende stap:** Dennis draait de commando's hieronder tegen zijn Prometheus.

## Lokaal tegen Prometheus

```sh
# config/config.yaml
#   prometheus.url          read-only basis-URL, zonder userinfo
#   prometheus.retention_days   lager dan 7 → queries gebruiken [1d]
#   prometheus.health_paths en prometheus.test_clients
#   prometheus.token_env    alleen de naam van de env-var, nooit het token
# config/environments.yaml  k8s_namespaces per dev, acc, prod
depgraph collect --set full --prometheus --output data
depgraph build --raw data/raw --output data/snapshot.json
depgraph drift --snapshot data/snapshot.json --output drift_report
depgraph library-impact
```

`library-impact` antwoordt `libraries disabled`. `--prometheus-fixture` leest opgenomen query-antwoorden en praat niet met Prometheus. Zonder `--prometheus` slaat collect de runtime-collector over.

## Checklist voor Dennis

- [ ] `prometheus.url` wijst naar de read-only Prometheus. Het token staat alleen in de env-var van `token_env`.
- [ ] `retention_days` klopt met hoe lang Prometheus samples bewaart. Onder 7 dagen verwacht je `calls_total` in plaats van `calls_7d`.
- [ ] Elke mesh-namespace staat in precies één `k8s_namespaces`-lijst.
- [ ] `health_paths` en `test_clients` dekken de probes en loadtests die anders edges worden.
- [ ] `aliases.yaml` noemt de workload-namen die niet gelijk zijn aan de componentnaam. De rest hoort in `unresolved.csv`, met volume.
- [ ] `drift_report/prod.md` toont `both`, `observed_only`, `declared_only` en `candidate` naast wat je in de catalogus verwacht.
- [ ] `meta.json` `coverage` noemt het aantal components, observed edges en unresolved rijen.
- [ ] Een externe host (ServiceEntry, PassthroughCluster) staat als `resource:external-api:…` met `inferred: true`, niet als component.
- [ ] Tempo blijft uit tot de metric-namen geverifieerd zijn.
