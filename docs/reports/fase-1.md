# Fase 1

## F1-1 t/m F1-7

- **Taak:** collectors, normalizer v1, golden graph, show/deps/consumers, lokale publisher, schedules
- **Status:** gereed op fixtures. Cijfers op echte data zijn voor Dennis.
- **Gedaan:** `gitlab_inventory` en `gitlab_deployments` lezen een fixture of de GitLab-API. Live `collect` gebruikt dezelfde selectie als inventory: volledige scan als `gitlab.groups` en `gitlab.projects` leeg zijn, anders de vereniging, met filters en een warning voor een gearchiveerd expliciet project (ADR-006). `k8s_workloads` leest manifests uit de repo. `kubernetes.live_read` staat uit; alleen dan mag kubectl read-only meedoen, zonder de kubeconfig te loggen. De normalizer lost aliassen op, houdt één deployment per component per omgeving, kiest de hoogste versie (digest, semver, git-sha, overig), schrijft `deployment_mismatches` in `meta.json` en `unresolved.csv`. ADR-007 accepteert `network-policy`, `firewall-rule` en `infra`. `depgraph show`, `deps` en `consumers` lezen een snapshot. `publish snapshot` en `site` schrijven lokale bestanden. `.gitlab-ci.yml` roept die commando's aan op de drie schedules.
- **Tests:** 74 geslaagd (`cargo test --all-targets --all-features`), inclusief golden file `tests/fixtures/phase1/graph.json`, idempotentie, en CLI-tests op die snapshot.
- **Metrics:** geen echte telling. Zie hieronder.
- **Afwijkingen en ADR:** [ADR-006](../decisions/ADR-006-volledige-scan.md) (scan en projecten) en [ADR-007](../decisions/ADR-007-resource-kinds.md) (resource-kinds).
- **Open vragen:** de GitLab-versie via `GET /api/v4/version`. Groepspaden zijn optioneel.
- **Volgende stap:** Dennis draait de commando's hieronder tegen zijn GitLab.

## Lokaal de eerste echte graaf

```sh
glab auth login --hostname <gitlab-host>
# of: export GITLAB_TOKEN=...
# config/config.yaml: gitlab.base_url. groups en projects mogen leeg blijven.
depgraph inventory --output inventory.csv
depgraph collect --set full --output data
# smaller: depgraph collect --set full --group org/infra/platform --project org/other-team/ingress-gateway --output data
depgraph build --raw data/raw --output data/snapshot.json
depgraph site --snapshot data/snapshot.json --output public
depgraph show <component> --snapshot data/snapshot.json
depgraph deps <component> --snapshot data/snapshot.json --env prod
depgraph consumers <component> --snapshot data/snapshot.json
```

`collect` zonder `--fixture` gebruikt GitLab. `--fixture` is alleen voor de tests.

## Te meten door Dennis

- Aantal projecten in de inventory tegen de echte groepen.
- Dekking van `catalog-info.yaml` versus wat `propose-catalog` nog moet voorstellen.
- Aantal `deployment_mismatches` op productie.
- Aantal regels in `unresolved.csv`.
- Of `istio_requests_total` met `reporter="destination"` de gedeclareerde edges bevestigt. Die collector is nog geen fase-1-levering; de runtimebron staat in ADR-003.
- Of live kubectl (`kubernetes.live_read: true`) iets toevoegt boven de manifests in git. Default blijft uit.
