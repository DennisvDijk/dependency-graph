# depgraph voor teams

depgraph tekent runtime-afhankelijkheden en deploys over GitLab-repo's. De toolrepo staat op GitHub. Jullie applicatierepo's blijven op GitLab. Er gaat niets vanuit depgraph terug naar die repo's, het cluster of Prometheus.

Fase 0 legt alleen de repo, de config en de validatie neer. Er wordt nog niets verzameld. Echte tokens en echte data horen niet in deze repo en niet bij de bot die de tool bouwt.

## Wat je later aanlevert

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

`metadata.name` is de canonieke naam: lowercase kebab-case, hetzelfde als in `component:<naam>`. `spec.type` is `service`, `job`, `library` of `website`. `spec.lifecycle` is `experimental`, `production` of `deprecated`. Het schema in deze repo is nog een stub; F0-5 maakt het af.

Zachte afhankelijkheden horen in de annotatie `depgraph/soft-dependencies`, niet als harde `dependsOn`.

## Omgevingen

We kennen drie namen: `dev`, `acc`, `prod`. GitLab-namen `development`, `acceptance`, `staging` en `production` worden daarop gemapt. Gebruik geen vierde omgevingsnaam in `environment:` zonder dat het platformteam de mapping aanpast.

Workloads dragen het label `app.kubernetes.io/name` met dezelfde canonieke naam.

## Wat je niet commit

- tokens, `.netrc`, kubeconfig, wachtwoorden in URLs
- SBOM- of trace-exports uit productie
- een tweede naam voor hetzelfde component zonder dat de alias in `config/aliases.yaml` naar precies één component wijst

## Pilot

De lijst van 5 tot 10 pilotapplicaties is leeg tot de teams die kiezen. Aanmelden loopt via het platformteam, niet via een PR die alleen een naam toevoegt.

Ids en de hash van een edge staan in [naming.md](naming.md). Open vragen staan in [ADR-001](decisions/ADR-001-aannames.md).
