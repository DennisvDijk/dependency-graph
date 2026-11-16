# depgraph MCP

`depgraph serve-mcp` is a read-only MCP server over Streamable HTTP. It is an OAuth 2.1 resource server. PKCE stays at the identity provider. The access token is checked here and is not sent to GitLab, Kubernetes, Prometheus, or Datadog.

## Config

```yaml
mcp:
  issuer: https://idp.example.com
  audience: https://depgraph.example.com
  jwks_url: https://idp.example.com/.well-known/jwks.json
  bind: 0.0.0.0:8080
```

`audience` is the resource indicator (RFC 8707). Tokens must carry that audience, and the scope `depgraph:read`.

Protected Resource Metadata is served at `/.well-known/oauth-protected-resource` (RFC 9728). A request without a valid token gets `401` and a `WWW-Authenticate` header that points at that metadata.

## Local, without an IdP

This binds loopback only and prints a warning. Do not publish the port.

```sh
depgraph serve-mcp --insecure-local --snapshot data/snapshot.json
```

The container image is the same binary:

```sh
docker build -t depgraph .
docker run --rm -p 8080:8080 -v "$PWD:/app" -w /app depgraph serve-mcp --insecure-local --snapshot /app/data/snapshot.json
```

`--insecure-local` refuses a bind address that is not loopback.

## Identity provider

1. Register a client for the agent (opencode, pi, or another MCP client) at the IdP. The IdP runs PKCE. depgraph does not.
2. Set the resource / audience to the URL the client uses to reach depgraph.
3. Issue access tokens as JWTs signed by a key in `mcp.jwks_url`.
4. Put `depgraph:read` in the `scope` claim (space separated) or the `scp` array.
5. Point the client at `POST /mcp` with `Authorization: Bearer <token>`.

depgraph caches the JWKS and fetches it again when it sees an unknown `kid`, so key rotation does not need a restart.

## Tools

`show`, `deps`, `consumers`, `impact`, `deploy_order`, `cycles`, `library_impact`, `drift`, `critical_nodes`, `orphans`, `diff`, and `component_context`.

`component_context` takes a component name or alias and returns the GitLab project path, web URL, default branch, catalog path, manifest paths, owner, and the direct dependencies and consumers. Agents should call it before opening the repository.

`library_impact` returns `libraries disabled`.
