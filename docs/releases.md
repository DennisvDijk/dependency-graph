# Releases and how to verify them

Releases are built and signed by CI, not by hand: the GitLab pipeline (`.gitlab/release.yml`) and the GitHub workflow (`.github/workflows/release.yml`). Both run when a tag such as `v0.2.0` is pushed:

```bash
git tag -s v0.2.0 -m "v0.2.0"     # -s signs the tag with your own key
git push origin v0.2.0
```

**GitLab** builds the Linux binary, writes `SHA256SUMS` and a CycloneDX dependency list (`sbom.cdx.json`), signs each file and the container image with cosign, and creates the release with `glab release create`.

**GitHub** builds the macOS (both architectures) and Linux binaries, then in one job writes `SHA256SUMS` and `sbom.cdx.json`, signs every file with keyless cosign, checks its own signatures, adds a build provenance attestation for each archive, and attaches everything to the release. It does not publish a container image.

## Versioning

depgraph follows [Semantic Versioning](https://semver.org/). While the version is `0.x`, a minor bump (`0.2.0`) may change the CLI, the MCP tools or the snapshot format; a patch bump (`0.1.1`) only fixes bugs. From `1.0.0` on:

- **Major:** a removed or renamed command, flag or MCP tool, or a change to `schemas/graph.schema.json` that breaks an existing reader.
- **Minor:** a new command, collector, output or optional field.
- **Patch:** a fix that does not change the interface.

The snapshot schema is versioned inside the file, so an older snapshot stays readable. Every user-visible change gets a line in [CHANGELOG.md](../CHANGELOG.md) under `Unreleased`; cutting a release means renaming that heading to the version and date, bumping `version` in `Cargo.toml`, and tagging as above.

## What you get

| File | Purpose |
|---|---|
| `depgraph-x86_64-unknown-linux-gnu.tar.gz` | The binary |
| `SHA256SUMS` | Checksums of the files |
| `sbom.cdx.json` | Every crate that went into the binary |
| `<file>.bundle` | Signature and certificate for each file |
| `<registry>/<project>:<tag>` | Container image, signed by digest |

The GitLab pipeline has no macOS binaries: they need a macOS runner. Add a job like `build-linux` with that runner's tag when you have one. The GitHub workflow builds them on GitHub's macOS runners.

## Verify before you run

### A release from GitHub

Replace `OWNER/REPO` with the repository. Use any release file in place of the archive.

```bash
cosign verify-blob \
  --bundle depgraph-aarch64-apple-darwin.tar.gz.bundle \
  --certificate-identity-regexp '^https://github.com/OWNER/REPO/\.github/workflows/release\.yml@refs/tags/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  depgraph-aarch64-apple-darwin.tar.gz

gh attestation verify depgraph-aarch64-apple-darwin.tar.gz --repo OWNER/REPO   # build provenance

sha256sum -c SHA256SUMS      # on macOS: shasum -a 256 -c SHA256SUMS
```

### A release from GitLab

Replace `GITLAB` and `GROUP/PROJECT` with your GitLab address and project path.

```bash
cosign verify-blob \
  --bundle depgraph-x86_64-unknown-linux-gnu.tar.gz.bundle \
  --certificate-identity-regexp '^https://GITLAB/GROUP/PROJECT//' \
  --certificate-oidc-issuer https://GITLAB \
  depgraph-x86_64-unknown-linux-gnu.tar.gz

cosign verify \
  --certificate-identity-regexp '^https://GITLAB/GROUP/PROJECT//' \
  --certificate-oidc-issuer https://GITLAB \
  REGISTRY/GROUP/PROJECT:v0.2.0

sha256sum -c SHA256SUMS
```

A pass means the file was produced by this project's pipeline for that tag and has not changed since. It does not say the code is free of bugs.

## Keyless or with a key

By default signing is keyless: GitLab hands the job a short-lived identity token and cosign exchanges it for a certificate through Sigstore's public services. That needs the runner to reach them.

A GitLab behind a firewall usually cannot. Two ways out:

1. Run your own Sigstore (Fulcio and Rekor) and point cosign at it.
2. Create a cosign key pair, store the private key and password as masked, protected CI/CD variables `COSIGN_PRIVATE_KEY` and `COSIGN_PASSWORD`, and publish the public key. The pipeline then signs with the key. Users verify with `--key cosign.pub` instead of the identity flags.

## Pinned tools

In the GitHub workflow every action that touches the release is pinned to a commit (the tag is in a comment). The GitLab cosign image is the CI/CD variable `COSIGN_IMAGE`. Its default is a moving tag; for production, mirror the image and set the variable to `registry/path@sha256:<digest>`.

## Not covered yet

- Neither pipeline has run on a real tag yet. Expect a runner or network detail on the first run; try a `v0.0.1-test` tag in a scratch project first (it creates a real release, which you can delete).
- The GitLab cosign image default is a moving tag, and the CI workflow (not the release one) still uses version tags for its actions.
