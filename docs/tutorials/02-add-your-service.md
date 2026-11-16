# 2. Add your service

depgraph finds every project your GitLab token can see. A service shows up better, with an owner, the right name, and declared dependencies to compare against runtime, when its repository has a `catalog-info.yaml` at the root. The format is Backstage's, so the file stays useful if you ever adopt a portal.

## Let depgraph draft it

From a checkout of your repository:

```sh
depgraph propose-catalog --from . --output proposed-catalog
cat proposed-catalog/*/catalog-info.yaml
```

It reads your Kubernetes manifests, Terraform and network policies and writes a draft. Nothing in your repo changes. Review it, copy it to the repo root, fill in what it could not guess (`owner`, `system`, `dependsOn`).

For many repos at once, point `--from` at a folder of checkouts. A platform team can open the drafts as GitLab merge requests with `--open-mr --project <path>` (one `--project` per repo, in folder order). depgraph never merges them.

## The file

```yaml
apiVersion: backstage.io/v1alpha1
kind: Component
metadata:
  name: payments-api                       # canonical name, lowercase-kebab
  annotations:
    gitlab.com/project-slug: example-group/payments-api
    depgraph/otel-service-name: payments-api
    depgraph/k8s-app-name: payments-api
    depgraph/image-repository: registry.example.com/payments-api
    depgraph/soft-dependencies: example-group/notifications
spec:
  type: service                            # service | job | library | website
  lifecycle: production                    # experimental | production | deprecated
  owner: payments-team
  system: payments
  dependsOn:
    - component:default/billing-api        # another service
    - resource:default/payments-db         # a database, queue, bucket…
```

The rules that matter:

- **One name everywhere.** `metadata.name`, the OpenTelemetry `service.name` and the Kubernetes label `app.kubernetes.io/name` should be the same. That is how a runtime call from a pod gets matched to your catalog entry. If a workload name cannot change, ask whoever runs depgraph to add an alias (see [tutorial 6](06-real-data.md#aliases)).
- **`dependsOn` is for hard dependencies**: if it is down, you are down. Anything you degrade gracefully without goes in `depgraph/soft-dependencies` (comma-separated project paths). This sets the hardness used in impact scores: hard 1.0, unknown 0.5, soft 0.2.
- **Environments** in your `.gitlab-ci.yml` `environment:` should be one of `development`, `acceptance`, `staging`, `production` (or `dev`, `acc`, `prod`). Another name will not map to an environment until the mapping is extended.
- **Do not commit** tokens, kubeconfigs or internal URLs with credentials in them. depgraph rejects credentialed URLs and known token shapes in the graph anyway.

- **Several entities in one file** are fine: separate them with a `---` line, as in Backstage. This is handy for a deploy repo that runs several open-source apps ([tutorial 8](08-terraform-python-open-source.md)). Every entity needs `spec.lifecycle`, `spec.type` and `spec.owner`.

## Check it before you push

```sh
depgraph validate-catalog catalog-info.yaml path/to/billing-api/catalog-info.yaml
```

Every `dependsOn` must resolve to an entity in the files you pass, so pass the files of the things you depend on too. `valid (N entities)` means you are done; otherwise the error names the field.

## How you know it worked

After the next collection run (see [tutorial 6](06-real-data.md)):

```sh
depgraph show payments-api
depgraph drift --env prod     # are your declared deps the ones Istio sees?
depgraph scorecard            # does your team have owner, system and telemetry filled in?
```

An `observed_only` edge to a service you own is usually a missing `dependsOn`. A `declared_only` edge to a service that has been quiet for weeks may be a dependency you no longer have.
