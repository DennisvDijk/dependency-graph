# depgraph tutorials

depgraph answers one question well: **what depends on what, at runtime, in dev, acc and prod?** It reads your GitLab repos, whether services in any language, Terraform stacks, or deploy repos for open-source apps (what you *declare*), and Istio metrics (what *actually calls what*), shows the gateways and load balancers in between, puts all of it in one graph, and lets you, your CI and your AI agent ask questions about it.

Pick the path that matches what you want to do:

| I want to… | Read | Time |
|---|---|---|
| See what it does, on sample data, before anything else | [1. Try it in five minutes](01-try-it.md) | 5 min |
| Get my service into the graph correctly | [2. Add your service](02-add-your-service.md) | 10 min |
| Cover Terraform, firewall rules, Docker, services in any language (Python, Elixir, Rust…) and open-source apps we run | [8. Terraform, firewalls, Docker, any language and open-source apps](08-terraform-python-open-source.md) | 15 min |
| Discover ties from our AWS or Azure account, with nothing configured per service | [9. Cloud discovery (AWS, Azure)](09-cloud-discovery.md) | 10 min |
| Answer "who breaks if I change this?" and similar questions | [3. Ask the graph questions](03-ask-questions.md) | 10 min |
| Let Cursor, Claude, VS Code or opencode use the graph | [4. Connect your AI agent](04-ai-agent.md) | 5 min |
| Get a comment on merge requests that touch an API with prod consumers | [5. Merge request check](05-mr-check.md) | 15 min |
| Run depgraph for my team on real GitLab and Prometheus data | [6. Run it on real data](06-real-data.md) | 1 hour |
| Share a summary of a run without exposing repository or host names | [6. Run it on real data](06-real-data.md#share-a-run-summary) | 2 min |
| Fix something that does not work | [7. Troubleshooting](07-troubleshooting.md) | — |

Most developers only need **1, 2 and 4**; add **8** if you own infrastructure or run open-source apps. Tutorial 6 is for the person who runs depgraph for a team.

## Three ideas to keep in mind

1. **Declared versus observed.** A `dependsOn` in `catalog-info.yaml` is a claim. An Istio call is evidence. depgraph keeps both and shows the difference as *drift*.
2. **Three environments.** Everything maps onto exactly `dev`, `acc` or `prod`. GitLab's `staging` becomes `acc`; Kubernetes namespaces are mapped in `config/environments.yaml`.
3. **Read-only.** depgraph never changes your repos, clusters or Prometheus. The only writes are one merge request note, one drift issue, and draft catalog merge requests you ask for explicitly. Tokens live in environment variables or `glab`, never in config files.

Reference material: [quickstart](../quickstart.md), [MCP server and auth](../mcp.md), [naming rules](../naming.md), [decisions (ADRs)](../decisions/).
