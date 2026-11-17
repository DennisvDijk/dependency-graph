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

`--insecure-local` refuses a bind address that is not loopback. It also answers only requests whose `Host` and `Origin` headers name loopback, so a web page cannot reach it through DNS rebinding.

Tokens must be signed with an asymmetric algorithm (RS*, PS*, ES* or EdDSA). Tokens that use a shared secret (HS*) are rejected. The server asks the issuer for its key set at most once every 30 seconds, so a key rotation can take that long to be picked up.

## Entra ID (optional)

Leave `auth.provider: generic` unless the organization uses Entra. With Entra, the MCP client performs the browser flow. depgraph never asks you to paste a token. `--insecure-local` ignores the provider and still only binds loopback.

```yaml
auth:
  provider: entra
  tenant_id: 00000000-0000-0000-0000-000000000000
  client_id: 11111111-1111-1111-1111-111111111111
  audience: api://11111111-1111-1111-1111-111111111111
  allowed_roles: []
  allowed_groups: []
```

Real ids may be supplied as `DEPGRAPH_ENTRA_TENANT_ID` and `DEPGRAPH_ENTRA_CLIENT_ID` instead of committing them. The issuer becomes `https://login.microsoftonline.com/<tenant>/v2.0`. The JWKS URL is the `jwks_uri` from `https://login.microsoftonline.com/<tenant>/v2.0/.well-known/openid-configuration` (the v2 document publishes `https://login.microsoftonline.com/<tenant>/discovery/v2.0/keys`).

App registration:

1. Entra admin center → App registrations → New registration. Note the application (client) id and the directory (tenant) id.
2. Expose an API. Set the Application ID URI to `api://<client-id>`.
3. Add a delegated scope `depgraph.read` (admins and users can consent).
4. App roles → create an application role `Depgraph.Read` (allowed member types: applications and users) for service clients.
5. API permissions → add the delegated scope, then Grant admin consent.
6. Optional: put group object ids in `allowed_groups` or extra role values in `allowed_roles`.

Clients: `depgraph mcp-config cursor|claude|vscode|opencode`. The JSON has the server URL only. The client discovers `/.well-known/oauth-protected-resource` and opens the browser.

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
