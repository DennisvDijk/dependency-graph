# ADR-006: Resource-kinds voor netwerk en infra

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar), vastgelegd als acceptatie van het voorstel uit [ADR-003](ADR-003-runtime-bron.md) en [ADR-004](ADR-004-pilot-en-datadog.md).

## Context

Het landschap is Kubernetes, infrastructuur en firewalling, niet een verzameling interne libraries. De oorspronkelijke kind-lijst (`database`, `topic`, `queue`, `external-api`, `model-endpoint`, `identity-provider`, `other`) dekt de platformobjecten die in de repo's staan niet.

## Besluit

De parser kent drie extra kinds. Meer niet.

- `network-policy` voor Kubernetes `NetworkPolicy`
- `firewall-rule` voor firewallregels uit de repo
- `infra` voor Terraform en andere IaC-componenten

Ids blijven `resource:<kind>:<naam>` met een kebab-case naam. Er is geen apart model voor regels, poorten of providers. Een bestand levert één node.

## Gevolgen

- `Library`-nodes blijven uit (ADR-001, vraag 8).
- Bestaande kinds veranderen niet.
- Een latere fase kan de nodes verrijken. Deze fase stopt bij het id.
