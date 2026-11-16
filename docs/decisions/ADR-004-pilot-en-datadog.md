# ADR-004: De pilot is alle teamapps; Datadog is read-only

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Beantwoordt vraag 15 uit [ADR-001](ADR-001-aannames.md).

## Context

Het plan vroeg om 5 tot 10 pilotapplicaties. Dennis neemt het hele team: alle applicaties in de GitLab-groepen van zijn team ([ADR-001](ADR-001-aannames.md), vraag 2). De paden van die groepen zijn nog niet ingevuld. De verzameling moet via de code-workspaces lopen: de repo's op GitLab, niet een rechtstreekse aanroep van het cluster.

Datadog mag deployment- en health-status leveren, alleen lezen.

## Besluit

1. De pilot is alle applicaties van het team. `pilot.yaml` houdt `apps: []`. Een lege lijst is hier geen shortlist-die-nog-leeg-is. De scope is elk project dat het token mag zien, optioneel vernauwd met `gitlab.groups` of `gitlab.projects` ([ADR-006](ADR-006-volledige-scan.md)). Er is geen allowlist van 5 tot 10 namen.
2. Collectors voor deze pilot lezen GitLab-code: catalogus, manifests, Helm, kustomize, Terraform en andere IaC, Istio-config in de repo, en CI-jobs met `environment:`. Ze raken het cluster niet rechtstreeks aan. De Kubernetes-collector blijft in de boom als latere controle, en is niet het pilotpad.
3. Datadog is een optionele, read-only statusbron. `datadog.enabled` staat default op `false`. De config noemt alleen de env-var-namen `DEPGRAPH_DATADOG_API_KEY` en `DEPGRAPH_DATADOG_APP_KEY`, plus een placeholder-site. Geen API-sleutel en geen app-key in de repo of in de graaf. De collector `datadog_status` is een stub en schrijft nergens naar Datadog.
4. Zelfde voorstel als [ADR-003](ADR-003-runtime-bron.md): resource-kinds `network-policy`, `firewall-rule` en `infra` voor firewallregels, network policies, ingresses en infra-componenten uit de repo's. [ADR-007](ADR-007-resource-kinds.md) accepteert ze. De parser kent ze.

## Gevolgen

- Een PR die één applicatienaam aan `pilot.yaml` toevoegt, verkleint de pilot niet. De scan volgt ADR-006.
- `example-group/pilot` in de fixtures is geen verplichte config. Een lege `groups` en `projects` scant alles wat het token mag zien.
- Datadog blijft uit tot iemand `enabled` aanzet én de env-vars buiten de repo zet.
