# Running depgraph in a container

The image is built from the `Dockerfile` in this repository. It is distroless, runs as a non-root user, and has no shell. Base images are pinned by digest.

```sh
docker build -t depgraph .
```

## Serve the graph to agents

```sh
docker run -d --name depgraph -p 8080:8080 \
  -v "$PWD/config:/app/config:ro" \
  -v "$PWD/data:/app/data:ro" \
  depgraph
```

- It runs `depgraph serve-mcp --bind 0.0.0.0:8080 --snapshot /app/data/snapshot.json`.
- It is authenticated: every `/mcp` request needs a valid token from the issuer in the `mcp:` section of `config/config.yaml` (`issuer`, `audience`, `jwks_url`; see [mcp.md](mcp.md)). The shipped file has placeholder values, so set your own before you expose the port. The server starts either way; a wrong issuer means every client is refused, not that it serves open. `--insecure-local` only works on loopback, so it cannot be reached from outside a container. Do not use it here.
- Mount the snapshot read-only. The server never needs a token to read it. Rebuild the snapshot outside the container and replace the file; restart the container to pick it up.

## Run any other command

The entrypoint is `depgraph`, so arguments are the subcommand:

```sh
docker run --rm -v "$PWD:/app" depgraph impact payments-api --snapshot /app/data/snapshot.json
docker run --rm -v "$PWD:/app" depgraph report --snapshot /app/data/snapshot.json
```

`collect` needs your tokens and network access; pass them as environment variables (`-e GITLAB_TOKEN`, without a value, to forward it from your shell) and keep them out of the image.

## Health check

The image declares a health check, so `docker ps` shows `healthy` or `unhealthy`:

```
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD depgraph healthcheck
```

There is no shell and no `curl` in the image, so the check is a depgraph subcommand. It requests `http://127.0.0.1:8080/healthz` and exits 0 on a 2xx answer, non-zero otherwise (3 second limit). `/healthz` answers `ok` without a token and reveals nothing about the graph, so a load balancer can use it too. To probe another address: `depgraph healthcheck --url http://host:8080/healthz`.

In Kubernetes, use an HTTP probe on `/healthz`, port 8080, instead of the exec check.

## Timeouts

depgraph runs `glab` (token lookup), `kubectl` and the cloud CLIs (`aws`, `az`) as child processes and stops each after 120 seconds. For a slow GitLab or a big account, raise it:

```sh
DEPGRAPH_CLI_TIMEOUT_SECS=600 depgraph collect --set full
```

The value is in seconds. Zero, a negative number or text is ignored and the default applies. In a pipeline, set it as a CI/CD variable.

## Image version labels

```sh
docker build --build-arg VERSION=0.2.0 --build-arg REVISION="$(git rev-parse HEAD)" -t depgraph:0.2.0 .
```

These become the `org.opencontainers.image.version` and `.revision` labels. They default to `dev` and `unknown`.
