# depgraph container image.
#
# - Base images are pinned by digest. To update them, change the digest on
#   purpose (and the tag in the comment) in a reviewed change.
# - The build is offline-reproducible from the repo: `--locked`, no network
#   after the dependency fetch.
# - The runtime image is distroless and non-root. It has no shell, so the health
#   check is a depgraph subcommand.
#
# Serve (authenticated; configure `auth` in config/config.yaml first):
#   docker run -p 8080:8080 -v "$PWD/config:/app/config:ro" -v "$PWD/data:/app/data:ro" depgraph
# Run any other command:
#   docker run --rm -v "$PWD:/app" depgraph report --snapshot /app/data/snapshot.json
# The container never needs a token to serve. Mount the snapshot read-only.

# rust:1.100-bookworm
FROM rust:1.100-bookworm@sha256:ade39a710ef85f3506cffcfec3e8a24ba090badef1dff2fb798f71d878833fea AS build
WORKDIR /src

# Dependencies first, so a source-only change reuses this layer.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src \
 && echo 'fn main() {}' > src/main.rs \
 && echo '' > src/lib.rs \
 && cargo build --release --locked \
 && rm -rf src target/release/depgraph target/release/deps/depgraph* target/release/.fingerprint/depgraph*

# Everything the code embeds at compile time (include_str!) must be copied.
COPY src ./src
COPY config ./config
COPY schemas ./schemas
COPY skills ./skills
RUN cargo build --release --locked

# gcr.io/distroless/cc-debian12:nonroot
FROM gcr.io/distroless/cc-debian12:nonroot@sha256:9dac0a79194e45a7da0158a9c6da57b217585af0786db3845d1f0ec1a0dd182f

ARG VERSION=dev
ARG REVISION=unknown
LABEL org.opencontainers.image.title="depgraph" \
      org.opencontainers.image.description="Self-hosted dependency graph: what depends on this, and what breaks if I change it" \
      org.opencontainers.image.source="https://github.com/DennisvDijk/dependency-graph" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.revision="${REVISION}"

COPY --from=build /src/target/release/depgraph /usr/local/bin/depgraph
WORKDIR /app
USER nonroot:nonroot
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["/usr/local/bin/depgraph", "healthcheck"]
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/depgraph"]
# Authenticated by default. `--insecure-local` only works on loopback, so it
# cannot be reached from outside a container; do not use it here.
CMD ["serve-mcp", "--bind", "0.0.0.0:8080", "--snapshot", "/app/data/snapshot.json"]
