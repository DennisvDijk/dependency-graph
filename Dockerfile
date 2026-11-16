# Local use and EKS Auto Mode. TLS terminates in front of the container.
# Mount config/ and a snapshot, then:
#   docker run --rm -p 8080:8080 -v "$PWD:/app" -w /app depgraph serve-mcp --insecure-local
FROM rust:1.100-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/target/release/depgraph /depgraph
WORKDIR /app
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/depgraph"]
CMD ["serve-mcp", "--insecure-local", "--snapshot", "/app/data/snapshot.json"]
