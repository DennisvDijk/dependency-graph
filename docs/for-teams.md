# depgraph voor teams

depgraph tekent runtime-afhankelijkheden en deploys over GitLab-repo's. De toolrepo staat op GitHub. Jullie applicatierepo's blijven op GitLab. depgraph leest die repo's en Prometheus. Het schrijft er niet naar, behalve één MR-notitie en één drift-issue vanuit de bot-token.

Hoe je een agent aansluit staat in [mcp.md](mcp.md).

## catalog-info.yaml bijhouden

Een `catalog-info.yaml` in de applicatierepo, Backstage-vorm:

```yaml
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: payments-api
  annotations:
    gitlab.com/project-slug: example-group/payments-api
    depgraph/otel-service-name: payments-api
    depgraph/k8s-app-name: payments-api
    depgraph/image-repository: registry.example.com/payments-api
    depgraph/soft-dependencies: example-group/notifications
spec:
  type: service
  lifecycle: experimental
  owner: platform-team
  system: payments
  dependsOn:
    - component:default/billing-api
    - resource:default/payments-db
```

`metadata.name` is de canonieke naam: lowercase kebab-case, hetzelfde als in `component:<naam>`. `spec.type` is `service`, `job`, `library` of `website`. `spec.lifecycle` is `experimental`, `production` of `deprecated`. `spec.owner` en `spec.system` komen in de site-filters terecht.

Zachte afhankelijkheden horen in de annotatie `depgraph/soft-dependencies`, niet als harde `dependsOn`.

Houd de naam gelijk aan het OpenTelemetry `service.name` en aan het label `app.kubernetes.io/name`. Wijkt een workload-naam af, zet dan een alias in `config/aliases.yaml` van de toolrepo. Eén alias, één component.

## Labels

- Kubernetes: `app.kubernetes.io/name` is de componentnaam.
- De wekelijkse drift-issue draagt het label `depgraph-drift`. Er blijft één open issue; depgraph werkt die bij.
- De MR-check zet één notitie met het merkteken `<!-- depgraph-mr-check -->` en werkt diezelfde notitie bij.

## Omgevingen

We kennen drie namen: `dev`, `acc`, `prod`. GitLab-namen `development`, `acceptance`, `staging` en `production` worden daarop gemapt. Gebruik geen vierde omgevingsnaam in `environment:` zonder dat het AI Platform-team de mapping aanpast.

Workloads dragen het label `app.kubernetes.io/name` met dezelfde canonieke naam.

## Wat je niet commit

- tokens, `.netrc`, kubeconfig, wachtwoorden in URLs
- SBOM- of trace-exports uit productie
- een tweede naam voor hetzelfde component zonder dat de alias in `config/aliases.yaml` naar precies één component wijst

## Pilot

De pilot is alle applicaties van het team, gelezen uit de GitLab-repo's. Er is geen shortlist van 5 tot 10. Interne libraries zitten niet in de graaf. Vragen lopen via het AI Platform-team.

Ids en de hash van een edge staan in [naming.md](naming.md). Besluiten staan in [ADR-001](decisions/ADR-001-aannames.md).
