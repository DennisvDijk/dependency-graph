# ADR-010: Entra ID is een optionele preset

- Status: geaccepteerd
- Datum: 2026-11-16
- Besluitvormers: Dennis (eigenaar). Bouwt voort op [ADR-005](ADR-005-mcp-server.md).

## Context

De MCP-server is een OAuth 2.1 resource server voor elke issuer met een JWKS-document. Dennis' organisatie gebruikt Microsoft Entra ID. Dat mag geen verplichte afhankelijkheid worden, en depgraph mag geen token doorzetten naar GitLab of Prometheus.

## Besluit

1. `auth.provider` staat default op `generic`. Dan gelden `mcp.issuer`, `mcp.audience` en `mcp.jwks_url`, met scope `depgraph:read` in `scope` of `scp`.
2. `auth.provider: entra` leidt de issuer af als `https://login.microsoftonline.com/<tenant>/v2.0` en het JWKS-adres uit het OpenID-document op `/.well-known/openid-configuration`. Het gebruikelijke `jwks_uri` is `https://login.microsoftonline.com/<tenant>/discovery/v2.0/keys`.
3. Tenant-id en client-id komen uit de config of, als ze gezet zijn, uit `DEPGRAPH_ENTRA_TENANT_ID` en `DEPGRAPH_ENTRA_CLIENT_ID`. Alleen placeholders staan in de repo.
4. De audience is `auth.audience`, bijvoorbeeld `api://<client-id>`. De kale client-id is ook een geldige `aud`. `exp`, `nbf` en de handtekening worden gecontroleerd.
5. Leesrecht is het gedelegeerde scope `depgraph.read` (ook `depgraph:read`) in `scp`, of de app-role `Depgraph.Read` / `depgraph.read` in `roles`. `allowed_groups` en `allowed_roles` beperken optioneel verder.
6. `--insecure-local` slaat deze controle over en weigert een niet-loopback bind. Dat pad verandert niet.

## Gevolgen

- Geen Entra-SDK. Alleen JWT en JWKS.
- Een client met Entra doet de browserflow zelf via de Protected Resource Metadata. depgraph geeft geen token terug.
- Registratiestappen staan in [mcp.md](../mcp.md).
