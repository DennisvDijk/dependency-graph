# Namen en ids

Canonieke omgevingen zijn `dev`, `acc` en `prod`. Een bronnaam (`staging`, `production`, `development`) mapt via `config/environments.yaml` op precies één daarvan. Een edge zonder omgeving gebruikt `null`, niet een vierde naam.

## Nodes

| Soort | Id | Naam |
| --- | --- | --- |
| component | `component:<naam>` | lowercase kebab-case |
| deployment | `deployment:<naam>@<env>` | dezelfde naam als het component; `env` is `dev`, `acc` of `prod` |
| resource | `resource:<kind>:<naam>` | kind is `database`, `topic`, `queue`, `gateway`, `external-api`, `model-endpoint`, `identity-provider`, `network-policy`, `firewall-rule`, `infra`, `contract` of `other` |
| library | `library:<purl-zonder-versie>` | `pkg:<type>/<naam>`, zonder `@versie`, `?qualifiers` of `#subpath` |

Een deployment `deployment:payments-api@prod` verwijst naar het bestaande node `component:payments-api`. Het type van het node moet bij het id-voorvoegsel horen.

## Edges

Het edge-id is `edge:` plus de lowercase hex SHA-256 van deze UTF-8-tekst, zonder extra newline aan het eind:

```text
<type>
<from_id>
<to_id>
<env>
```

`<env>` is leeg als de edge geen omgeving heeft. `<type>` is de wire-waarde: `declared_depends_on`, `observed_calls`, `network_traffic`, `candidate_calls`, `deployed_as`, `runs_in_cluster`, `has_workload`, `uses_library`, `publishes_to`, `consumes_from`, `routes_to`, `provides_contract`, `uses_contract`.

Bekend voorbeeld, omgeving leeg:

```text
declared_depends_on
component:payments-api
component:billing-api

```

Sha-256: `6aaec4d166acdcc592284a22feecf0cab4312bafc2f78939ce9f6d6a323479b1`.

Id: `edge:6aaec4d166acdcc592284a22feecf0cab4312bafc2f78939ce9f6d6a323479b1`.

`observed_calls` zonder omgeving is ongeldig. Elke edge heeft minstens één source, met `first_seen` niet later dan `last_seen`. `status` is `both`, `observed_only`, `declared_only`, `candidate` of `n/a` (default `n/a`). `criticality` is `hard`, `soft` of `unknown` (default `unknown`). `active` default `true`.

Source-kind: `catalog`, `otel`, `istio`, `hubble`, `sbom_hint`, `static`, `cloud`, `kubernetes`.

Zie [architecture.md](architecture.md) voor de lagen en [ADR-015](decisions/ADR-015-ontologie-en-blueprints.md) voor waarom het model gesloten blijft.

## Families (taxonomie)

De code groepeert types op één plek (`EdgeType::class()` in `src/models.rs` en `ResourceKind::family()` in `src/ids.rs`). Een nieuw type moet daar een groep krijgen, anders compileert het niet. Een test controleert dat dit bestand elk edge-type en elke resource-kind noemt.

| Edge-klasse | Types | Impact | Opmerking |
| --- | --- | --- | --- |
| dependency | `declared_depends_on`, `observed_calls`, `candidate_calls`, `routes_to` | ja | `routes_to` wordt niet omgekeerd: een wijziging bij de backend bereikt de gateway ervoor |
| messaging | `publishes_to`, `consumes_from` | ja | `publishes_to` wordt bij de analyse omgedraaid: een uitgever merkt niets van zijn topic |
| contract | `provides_contract`, `uses_contract` | ja | `provides_contract` wordt omgekeerd: een wijziging van het contract raakt de aanbieder niet, maar wel wie het gebruikt. Een gebruik dat alleen uit een bekende aanroep is afgeleid is een `candidate` |
| deployment | `deployed_as` | ja | |
| placement | `runs_in_cluster`, `has_workload` | nee | zegt waar iets draait, niet wat waarvan afhangt |
| library | `uses_library` | ja | |
| traffic | `network_traffic` | nee | pakketrichting is geen aanroeprichting |

| Resource-familie | Kinds |
| --- | --- |
| data store | `database` |
| messaging | `topic`, `queue` |
| network | `gateway`, `network-policy`, `firewall-rule` |
| external | `external-api`, `model-endpoint`, `identity-provider` |
| contract | `contract` |
| platform | `infra`, `other` |

## Canonical output

Voor het schrijven worden nodes en edges op id gesorteerd, en sources op kind, `first_seen`, `last_seen`, `evidence_ref`. Object-sleutels komen uit structs en `BTreeMap`, dus de sleutelvolgorde ligt vast. Het snapshot-schema is `schemas/graph.schema.json` en wordt uit de Rust-modellen gegenereerd; de test faalt als het bestand achterloopt.

Een alias wijst altijd naar precies één componentnaam, zonder het `component:`-voorvoegsel. Zie `config/aliases.example.yaml`. `config/aliases.yaml` blijft leeg tot er echte aliassen zijn. Een alias die twee keer voorkomt, of een lijst van componenten, is een configfout.

## Buiten scope, types blijven

`Library`-nodes en `uses_library`-edges blijven geldige ids. `libraries.enabled` staat uit: er zijn geen interne libraries, en collectors mogen ze niet schrijven. Resource-kinds `network-policy`, `firewall-rule` en `infra` zijn geaccepteerd in ADR-007. De parser kent ze. Er is geen apart model voor regels of providers.

## Geheimen

Geen veld mag een URL met userinfo bevatten, of een bekend tokenpatroon (`glpat-`, `ghp_`, `AKIA…`, private-key header, `client-key-data`, wachtwoord-toekenningen). Foutteksten en `meta.json` redigeren die vormen naar `***`. Kubeconfig-inhoud hoort nergens in config of graaf; alleen de naam van de env-var.
